//! The prompt window's API: interventions (spec §13), check-ins (spec §18),
//! break management (spec §17).

use serde::Serialize;
use tauri::{Manager, State};

use aos_core::accountability::BreakState;
use aos_core::events::AppEvent;

use crate::db::{engine_data, now, plans};
use crate::engine::emit_event;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingPrompt {
    Intervention {
        interruption: crate::db::models::InterruptionRow,
        commitment_title: Option<String>,
    },
    Checkin {
        checkin: crate::db::models::CheckinRow,
        commitment_title: Option<String>,
        cadence_min: u32,
    },
    BreakOver {
        commitment_title: Option<String>,
    },
    None,
}

#[tauri::command]
pub fn get_pending_prompt(state: State<'_, AppState>) -> AppResult<PendingPrompt> {
    let (open_interruption, break_over, active_title, cadence) = {
        let engine = state.engine.lock();
        (
            engine.open_interruption,
            engine.break_over_pending,
            engine.active_commitment.as_ref().map(|c| c.title.clone()),
            engine.settings.checkin_cadence_min,
        )
    };
    // A prompt names the commitment it was RAISED for, from its stored
    // row — the active commitment may have switched, completed, or
    // stopped since (the popup refreshes on events and a timer).
    let title_for = |cid: Option<i64>| -> Option<String> {
        cid.and_then(|c| {
            state
                .db
                .with(|conn| plans::get_commitment(conn, c))
                .ok()
                .map(|c| c.title)
        })
    };
    if let Some(id) = open_interruption {
        let interruption = state.db.with(|conn| engine_data::get_interruption(conn, id))?;
        let commitment_title = title_for(interruption.commitment_id);
        return Ok(PendingPrompt::Intervention {
            interruption,
            commitment_title,
        });
    }
    if break_over {
        // Back-to-work context IS the currently active commitment.
        return Ok(PendingPrompt::BreakOver {
            commitment_title: active_title,
        });
    }
    if let Some(checkin) = state.db.with(engine_data::unanswered_checkin)? {
        let commitment_title = title_for(checkin.commitment_id).or(active_title);
        return Ok(PendingPrompt::Checkin {
            checkin,
            commitment_title,
            cadence_min: cadence,
        });
    }
    Ok(PendingPrompt::None)
}

/// Answer an intervention (spec §13 options).
#[tauri::command]
pub fn respond_intervention(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
    response: String,
    note: Option<String>,
) -> AppResult<()> {
    if note
        .as_deref()
        .is_some_and(|value| value.chars().count() > 2_000)
    {
        return Err(AppError::invalid("Prompt notes must be 2,000 characters or fewer."));
    }
    // Serialize the row claim, any activity correction, and runtime recovery
    // publication with activity deletion. The response therefore either
    // fails against an already-deleted prompt or completes before deletion
    // resets/removes its history; it cannot resurrect recovery afterward.
    let _history_guard = state.activity_history_boundary.lock();
    // Atomically claim and validate this exact unanswered row before an
    // "actually work" response creates an activity boundary. A stale popup
    // or retry must fail without flushing the live aggregator.
    let interruption = state.db.with_tx(|tx| {
        let interruption = engine_data::get_interruption(tx, id)?;
        engine_data::respond_interruption(tx, id, &response, note.as_deref())?;
        Ok(interruption)
    })?;
    // Persist the still-open flagged draft before looking up the episode's
    // sessions, so every segment that can be corrected exists in SQLite.
    if response == "actually_work" {
        crate::engine::flush_open_session(&app);
    }
    let handled_runtime_prompt = {
        let mut engine = state.engine.lock();
        if engine.open_interruption == Some(id) {
            engine.open_interruption = None;
            match response.as_str() {
                // Start the recovery timer (spec §14).
                "return" => {
                    engine.recovering_interruption_id = Some(id);
                    engine.tracker.begin_recovery(now());
                }
                _ => {
                    engine.recovering_interruption_id = None;
                    engine.tracker.resolve();
                }
            }
            true
        } else {
            false
        }
    };
    // "This is actually work": teach the classifier about the EXACT activity
    // the intervention flagged (stored on the interruption row) — the live
    // foreground may already be this popup or another window by now.
    match response.as_str() {
        "actually_work"
            if !interruption.process_name.is_empty() || interruption.browser_domain.is_some() =>
        {
            // The correction's project is the INTERRUPTED commitment's,
            // resolved from the database — the engine's active commitment
            // may have switched, completed, or stopped before the user
            // answered this popup.
            let project_id = state.db.with(|conn| {
                Ok(interruption
                    .commitment_id
                    .and_then(|cid| plans::get_commitment(conn, cid).ok())
                    .and_then(|c| c.task_id)
                    .and_then(|tid| crate::db::tasks::get(conn, tid).ok())
                    .and_then(|t| t.project_id))
            })?;
            // Bounded to the flagged episode: from its true start (persisted
            // on the interruption; idle gaps make it underivable from the
            // intervention time) to shortly after the intervention fired —
            // never later history, never manually-classified rows.
            let window_start = interruption
                .episode_started_at
                .unwrap_or(interruption.started_at - interruption.distracted_secs.max(0))
                - 60;
            let window_end = interruption.started_at + 120;
            state.db.with(|conn| {
                let rows: Vec<(i64, String)> = {
                    let mut stmt = conn.prepare(
                        "SELECT id, local_date FROM activity_sessions
                         WHERE classification='distracted'
                           AND classification_source != 'manual'
                           AND process_name = ?1
                           AND COALESCE(browser_domain,'') = COALESCE(?2,'')
                           AND COALESCE(related_commitment_id,-1) = COALESCE(?3,-1)
                           AND ended_at >= ?4
                           AND started_at <= ?5
                         ORDER BY started_at",
                    )?;
                    let rows = stmt.query_map(
                        rusqlite::params![
                            interruption.process_name,
                            interruption.browser_domain,
                            interruption.commitment_id,
                            window_start,
                            window_end
                        ],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;
                    rows.collect::<Result<Vec<_>, _>>()?
                };
                let ids: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
                // Anchor the correction to the episode's sessions so deleting
                // that activity history deletes this memory with it. An
                // episode can span local midnight (sessions::insert splits
                // drafts there), putting its rows on multiple dates — store
                // one row per affected date, each anchored to that date's
                // first session, all sharing the interruption id as group_id:
                // ONE logical memory, and delete_range removes the whole
                // group when ANY covered date is deleted. No matched session
                // leaves one unanchored row, which delete_range covers by
                // creation time instead.
                let anchors: Vec<Option<i64>> = if rows.is_empty() {
                    vec![None]
                } else {
                    let mut per_date: Vec<Option<i64>> = vec![];
                    let mut last_date: Option<&str> = None;
                    for (id, date) in &rows {
                        if last_date != Some(date.as_str()) {
                            per_date.push(Some(*id));
                            last_date = Some(date.as_str());
                        }
                    }
                    per_date
                };
                for anchor in anchors {
                    conn.execute(
                        "INSERT INTO activity_corrections(session_id, group_id, process_name,
                            browser_domain, normalized_title, commitment_id, project_id,
                            old_classification, new_classification, reason, created_at)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,'distracted','supporting','Confirmed as work during intervention',?8)",
                        rusqlite::params![
                            anchor,
                            id,
                            interruption.process_name,
                            interruption.browser_domain,
                            aos_core::aggregator::normalize_title(&interruption.window_title),
                            interruption.commitment_id,
                            project_id,
                            now()
                        ],
                    )?;
                }
                // The flagged sessions themselves stop being penalized.
                for sid in ids {
                    conn.execute(
                        "UPDATE activity_sessions SET classification='supporting',
                            classification_confidence=1.0, classification_source='manual',
                            classification_reason='Confirmed as work during intervention',
                            pending_ai=0
                         WHERE id=?1",
                        [sid],
                    )?;
                    crate::db::scores::refresh_stored_score_for_session(conn, sid)?;
                }
                Ok(())
            })?;
            state.engine.lock().pipeline_dirty = true;
            emit_event(&app, &AppEvent::SessionsUpdated);
            emit_event(&app, &AppEvent::ScoresUpdated);
        }
        _ => {}
    }
    let flow_commitment = interruption.commitment_id.or_else(|| {
        state
            .engine
            .lock()
            .active_commitment
            .as_ref()
            .map(|active| active.id)
    });
    match response.as_str() {
        "priority_changed" => emit_event(
            &app,
            &AppEvent::PriorityChangeRequested {
                commitment_id: flow_commitment,
            },
        ),
        "blocked" => emit_event(
            &app,
            &AppEvent::BlockedFlowRequested {
                commitment_id: flow_commitment,
            },
        ),
        _ => {}
    }
    if handled_runtime_prompt {
        emit_event(&app, &AppEvent::DistractionResolved { recovery_secs: None });
    }
    close_popup_if_idle(&app, &state);
    Ok(())
}

/// Answer a periodic check-in (spec §18).
#[tauri::command]
pub fn respond_checkin(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    checkin_id: i64,
    response: String,
    note: Option<String>,
) -> AppResult<()> {
    if note
        .as_deref()
        .is_some_and(|value| value.chars().count() > 2_000)
    {
        return Err(AppError::invalid("Check-in notes must be 2,000 characters or fewer."));
    }
    let (checkin, newly_answered) = state.db.with_tx(|tx| {
        let checkin = engine_data::get_checkin(tx, checkin_id)?;
        if checkin.response.is_some() {
            return Ok((checkin, false));
        }
        engine_data::answer_checkin(tx, checkin_id, &response, note.as_deref())?;
        Ok((checkin, true))
    })?;
    if !newly_answered {
        close_popup_if_idle(&app, &state);
        return Ok(());
    }
    emit_event(&app, &AppEvent::CheckinAnswered { checkin_id });
    let flow_commitment = checkin.commitment_id.or_else(|| {
        state
            .engine
            .lock()
            .active_commitment
            .as_ref()
            .map(|active| active.id)
    });
    match response.as_str() {
        "priority_changed" => emit_event(
            &app,
            &AppEvent::PriorityChangeRequested {
                commitment_id: flow_commitment,
            },
        ),
        "blocked" => emit_event(
            &app,
            &AppEvent::BlockedFlowRequested {
                commitment_id: flow_commitment,
            },
        ),
        _ => {}
    }
    close_popup_if_idle(&app, &state);
    Ok(())
}

/// Start a planned break (spec §17). Not a distraction — and a session
/// boundary: pre-break work keeps its real classification, break time is
/// recorded separately.
#[tauri::command]
pub fn start_break(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    minutes: u32,
) -> AppResult<crate::db::models::BreakRow> {
    let history_guard = state.activity_history_boundary.lock();
    crate::engine::flush_open_session(&app);
    let minutes = minutes.clamp(1, 240);
    let row = state
        .db
        .with_tx(|tx| engine_data::start_break(tx, minutes as i64 * 60))?;
    {
        let mut engine = state.engine.lock();
        engine.current_break = Some((row.id, BreakState::start(row.started_at, minutes as i64 * 60)));
        engine.break_over_pending = false;
        engine.tracker.resolve();
    }
    drop(history_guard);
    emit_event(&app, &AppEvent::BreakStarted { ends_at: row.planned_end_at });
    close_popup_if_idle(&app, &state);
    Ok(row)
}

#[tauri::command]
pub fn end_break_now(app: tauri::AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let _history_guard = state.activity_history_boundary.lock();
    // Flush while the break context still applies: the elapsed break time is
    // stored as "Planned break", then normal tracking resumes.
    crate::engine::flush_open_session(&app);
    state.db.with(engine_data::end_break)?;
    {
        let mut engine = state.engine.lock();
        engine.current_break = None;
        engine.break_over_pending = false;
    }
    emit_event(&app, &AppEvent::BreakEnded);
    Ok(())
}

#[tauri::command]
pub fn acknowledge_break_over(app: tauri::AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.engine.lock().break_over_pending = false;
    close_popup_if_idle(&app, &state);
    Ok(())
}

/// Strict Mode keeps the prompt window open until everything is answered
/// (spec §28); otherwise it closes once nothing is pending.
fn close_popup_if_idle(app: &tauri::AppHandle, state: &State<'_, AppState>) {
    let anything_pending = {
        let engine = state.engine.lock();
        engine.open_interruption.is_some() || engine.break_over_pending
    } || matches!(
        state.db.with(engine_data::unanswered_checkin),
        Ok(Some(_))
    );
    if !anything_pending {
        if let Some(w) = app.get_webview_window("intervention") {
            let _ = w.close();
        }
    }
}

/// Blocked flow helper (spec §16): record blocker + optional next action.
#[tauri::command]
pub fn get_commitment_title(state: State<'_, AppState>, id: i64) -> AppResult<String> {
    if id <= 0 {
        return Err(AppError::invalid("bad id"));
    }
    state.db.with(|conn| plans::get_commitment(conn, id).map(|c| c.title))
}
