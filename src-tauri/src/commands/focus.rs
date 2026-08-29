//! Active commitment + focus session commands (spec §7, §15, §16).

use serde::Deserialize;
use rusqlite::OptionalExtension;
use tauri::State;

use aos_core::accountability::validate_switch_reason;
use aos_core::events::AppEvent;

use crate::db::models::Commitment;
use crate::db::{engine_data, plans, tasks};
use crate::engine::emit_event;
use crate::error::{AppError, AppResult};
use crate::state::{ActiveCommitment, AppState};

fn load_active(conn: &rusqlite::Connection, commitment: &Commitment) -> AppResult<ActiveCommitment> {
    let project_id = match commitment.task_id {
        Some(tid) => tasks::get(conn, tid)?.project_id,
        None => None,
    };
    Ok(ActiveCommitment {
        id: commitment.id,
        title: commitment.title.clone(),
        done_definition: commitment.done_definition.clone(),
        project_id,
    })
}

fn clear_runtime_focus_if_matches(
    engine: &mut crate::state::EngineState,
    commitment_id: i64,
) -> bool {
    let matches_runtime =
        engine.active_commitment.as_ref().map(|item| item.id) == Some(commitment_id);
    if matches_runtime {
        engine.active_commitment = None;
        engine.focus_session_id = None;
        engine.tracker.resolve();
    }
    matches_runtime
}

fn ensure_direct_start_allowed(conn: &rusqlite::Connection, commitment_id: i64) -> AppResult<()> {
    plans::actionable_commitment(conn, commitment_id)?;
    let today = crate::db::today_local();
    let other_active_today: Option<i64> = conn
        .query_row(
            "SELECT c.id FROM daily_commitments c
             JOIN daily_plans p ON p.id=c.plan_id
             WHERE c.status='active' AND c.id<>?1 AND p.date=?2 LIMIT 1",
            rusqlite::params![commitment_id, today],
            |row| row.get(0),
        )
        .optional()?;
    if other_active_today.is_some()
        || engine_data::active_focus(conn)?
            .is_some_and(|focus| focus.commitment_id != commitment_id)
    {
        return Err(AppError::invalid(
            "Use Switch priority and explain what changed before starting another commitment.",
        ));
    }
    Ok(())
}

fn current_contract(
    conn: &rusqlite::Connection,
) -> AppResult<(Option<crate::db::models::FocusSessionRow>, Option<i64>)> {
    let focus = engine_data::active_focus(conn)?;
    let active_row: Option<i64> = conn
        .query_row(
            "SELECT id FROM daily_commitments WHERE status='active'
             ORDER BY started_at DESC, id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let current_id = focus.as_ref().map(|row| row.commitment_id).or(active_row);
    Ok((focus, current_id))
}

/// Start (or resume) working on a commitment: opens a focus session and
/// makes it the active commitment.
#[tauri::command]
pub fn start_commitment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    commitment_id: i64,
) -> AppResult<Commitment> {
    let history_guard = state.activity_history_boundary.lock();
    state
        .db
        .with(|conn| ensure_direct_start_allowed(conn, commitment_id))?;
    // A commitment change is a session boundary: close and classify the open
    // activity under the OLD context before switching.
    crate::engine::flush_open_session(&app);
    let (commitment, focus, active) = state.db.with_tx(|tx| {
        ensure_direct_start_allowed(tx, commitment_id)?;
        let commitment = plans::activate_commitment(tx, commitment_id)?;
        let focus = engine_data::start_focus(tx, commitment_id)?;
        let active = load_active(tx, &commitment)?;
        Ok((commitment, focus, active))
    })?;
    {
        let mut engine = state.engine.lock();
        engine.active_commitment = Some(active);
        engine.focus_session_id = Some(focus.id);
        engine.tracker.resolve();
    }
    drop(history_guard);
    emit_event(&app, &AppEvent::FocusStarted { commitment_id });
    emit_event(&app, &AppEvent::CommitmentChanged { commitment_id: Some(commitment_id) });
    crate::tray::refresh(&app);
    Ok(commitment)
}

/// Pause the focus session; the commitment stays today's but nothing is
/// being tracked against it as "active work".
#[tauri::command]
pub fn pause_focus(app: tauri::AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let history_guard = state.activity_history_boundary.lock();
    crate::engine::flush_open_session(&app);
    let ended = state.db.with(|conn| engine_data::end_focus(conn, "paused"))?;
    let mut engine = state.engine.lock();
    engine.focus_session_id = None;
    let commitment_id = engine.active_commitment.as_ref().map(|c| c.id);
    engine.active_commitment = None;
    engine.tracker.resolve();
    drop(engine);
    drop(history_guard);
    if let (Some(f), Some(cid)) = (ended, commitment_id) {
        let _ = f;
        emit_event(&app, &AppEvent::FocusEnded { commitment_id: cid });
        emit_event(&app, &AppEvent::CommitmentChanged { commitment_id: None });
    }
    crate::tray::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn complete_commitment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    commitment_id: i64,
) -> AppResult<Commitment> {
    let history_guard = state.activity_history_boundary.lock();
    state
        .db
        .with(|conn| plans::actionable_commitment(conn, commitment_id).map(|_| ()))?;
    let had_focus = state
        .db
        .with(engine_data::active_focus)?
        .is_some_and(|focus| focus.commitment_id == commitment_id);
    if had_focus {
        // Attribute the tail of the work to this commitment before it closes.
        crate::engine::flush_open_session(&app);
    }
    let (commitment, ended_focus) = state.db.with_tx(|tx| {
        plans::actionable_commitment(tx, commitment_id)?;
        let ended = engine_data::end_focus_for_commitment(tx, commitment_id, "completed")?;
        let commitment = plans::set_commitment_status(tx, commitment_id, "completed", None, None)?;
        Ok((commitment, ended.is_some()))
    })?;
    let cleared_runtime = {
        let mut engine = state.engine.lock();
        clear_runtime_focus_if_matches(&mut engine, commitment_id)
    };
    drop(history_guard);
    if ended_focus || cleared_runtime {
        emit_event(&app, &AppEvent::FocusEnded { commitment_id });
        emit_event(&app, &AppEvent::CommitmentChanged { commitment_id: None });
    }
    if let Some(task_id) = commitment.task_id {
        emit_event(&app, &AppEvent::TaskCompleted { task_id });
    }
    emit_event(&app, &AppEvent::ScoresUpdated);
    crate::tray::refresh(&app);
    Ok(commitment)
}

#[derive(Deserialize)]
pub struct BlockedInput {
    pub commitment_id: i64,
    /// waiting_for_someone | need_information | technical_issue |
    /// need_decision | dont_know_next | other
    pub blocker_kind: String,
    pub note: Option<String>,
    /// Smallest next action that would unblock (spec §16) — becomes a task.
    pub next_action: Option<String>,
}

#[tauri::command]
pub fn block_commitment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: BlockedInput,
) -> AppResult<Option<i64>> {
    const BLOCKER_KINDS: &[&str] = &[
        "waiting_for_someone",
        "need_information",
        "technical_issue",
        "need_decision",
        "dont_know_next",
        "other",
    ];
    if !BLOCKER_KINDS.contains(&input.blocker_kind.as_str()) {
        return Err(AppError::invalid("Choose a valid blocker type."));
    }
    if input.note.as_deref().is_some_and(|note| note.chars().count() > 2_000) {
        return Err(AppError::invalid("Blocker notes must be 2,000 characters or fewer."));
    }
    if input
        .next_action
        .as_deref()
        .is_some_and(|action| action.trim().chars().count() > 300)
    {
        return Err(AppError::invalid("The next action must be 300 characters or fewer."));
    }
    let history_guard = state.activity_history_boundary.lock();
    state.db.with(|conn| {
        plans::actionable_commitment(conn, input.commitment_id).map(|_| ())
    })?;
    let had_focus = state
        .db
        .with(engine_data::active_focus)?
        .is_some_and(|focus| focus.commitment_id == input.commitment_id);
    if had_focus {
        crate::engine::flush_open_session(&app);
    }
    let note = format!(
        "[blocked:{}] {}",
        input.blocker_kind,
        input.note.as_deref().unwrap_or("")
    );
    let (ended_focus, created_task_id) = state.db.with_tx(|tx| {
        plans::actionable_commitment(tx, input.commitment_id)?;
        let commitment = plans::set_commitment_status(
            tx,
            input.commitment_id,
            "pending",
            None,
            Some(&note),
        )?;
        let ended_focus =
            engine_data::end_focus_for_commitment(tx, input.commitment_id, "blocked")?.is_some();
        let created_task_id = if let Some(action) = input.next_action.as_deref().filter(|a| !a.trim().is_empty()) {
            Some(
                tasks::create(
                tx,
                &tasks::TaskInput {
                    title: action.trim().to_string(),
                    description: format!("Unblocks: {}", commitment.title),
                    project_id: None,
                    parent_task_id: commitment.task_id,
                    status: "planned".into(),
                    priority: "must".into(),
                    estimated_minutes: Some(15),
                    due_date: None,
                    tags: vec!["unblock".into()],
                },
            )?
            .id,
            )
        } else {
            None
        };
        Ok((ended_focus, created_task_id))
    })?;
    let cleared_runtime = {
        let mut engine = state.engine.lock();
        clear_runtime_focus_if_matches(&mut engine, input.commitment_id)
    };
    drop(history_guard);
    if ended_focus || cleared_runtime {
        emit_event(&app, &AppEvent::FocusEnded { commitment_id: input.commitment_id });
        emit_event(&app, &AppEvent::CommitmentChanged { commitment_id: None });
    }
    crate::tray::refresh(&app);
    Ok(created_task_id)
}

#[derive(Deserialize)]
pub struct SwitchInput {
    /// Commitment being switched TO. None = ad-hoc pause of the plan.
    pub to_commitment_id: Option<i64>,
    pub from_commitment_id: Option<i64>,
    /// "What changed?" — required (spec §7, §15).
    pub reason: String,
    /// What happens to the original: still_today | later | defer | cancel
    #[serde(default = "default_disposition")]
    pub original_disposition: String,
}

fn default_disposition() -> String {
    "still_today".into()
}

/// Intentional reprioritization (spec §15): logged as a priority change,
/// not a distraction.
#[tauri::command]
pub fn switch_commitment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: SwitchInput,
) -> AppResult<Option<Commitment>> {
    validate_switch_reason(&input.reason).map_err(AppError::invalid)?;
    let from_status = match input.original_disposition.as_str() {
        "later" | "still_today" => "pending",
        "defer" => "deferred",
        "cancel" => "cancelled",
        other => return Err(AppError::invalid(format!("Invalid disposition: {other}"))),
    };
    if input.to_commitment_id.is_some() && input.to_commitment_id == input.from_commitment_id {
        return Err(AppError::invalid("Choose a different commitment to switch to."));
    }
    let history_guard = state.activity_history_boundary.lock();

    // Validate every referenced row before the session boundary, so malformed
    // input cannot create a spurious split in the timeline.
    state.db.with(|conn| {
        if let Some(to_id) = input.to_commitment_id {
            plans::actionable_commitment(conn, to_id)?;
        }
        let (_, current_id) = current_contract(conn)?;
        if current_id != input.from_commitment_id {
            return Err(AppError::invalid(
                "The commitment being switched is no longer the active focus.",
            ));
        }
        Ok(())
    })?;

    crate::engine::flush_open_session(&app);
    let (new_commitment, new_focus_id, new_active) = state.db.with_tx(|tx| {
        if let Some(to_id) = input.to_commitment_id {
            plans::actionable_commitment(tx, to_id)?;
        }
        let (current_focus, current_id) = current_contract(tx)?;
        if current_id != input.from_commitment_id {
            return Err(AppError::invalid(
                "The commitment being switched is no longer the active focus.",
            ));
        }
        let ts = crate::db::now();
        tx.execute(
            "INSERT INTO interruptions(kind, commitment_id, response, response_note, started_at, acknowledged_at, created_at)
             VALUES('priority_switch', ?1, 'priority_changed', ?2, ?3, ?3, ?3)",
            rusqlite::params![input.from_commitment_id, input.reason.trim(), ts],
        )?;
        if let Some(from_id) = input.from_commitment_id {
            plans::set_commitment_status(
                tx,
                from_id,
                from_status,
                Some("priorities_changed"),
                Some(input.reason.trim()),
            )?;
        }
        match input.to_commitment_id {
            Some(to_id) => {
                let commitment = plans::activate_commitment(tx, to_id)?;
                let focus = engine_data::start_focus(tx, to_id)?;
                let active = load_active(tx, &commitment)?;
                Ok((Some(commitment), Some(focus.id), Some(active)))
            }
            None => {
                if let Some(focus) = current_focus {
                    engine_data::end_focus_for_commitment(tx, focus.commitment_id, "switched")?;
                }
                Ok((None, None, None))
            }
        }
    })?;

    {
        let mut engine = state.engine.lock();
        engine.active_commitment = new_active;
        engine.focus_session_id = new_focus_id;
        engine.tracker.resolve();
    }
    drop(history_guard);
    if let Some(old_id) = input.from_commitment_id {
        emit_event(&app, &AppEvent::FocusEnded { commitment_id: old_id });
    }
    if let Some(new_id) = input.to_commitment_id {
        emit_event(&app, &AppEvent::FocusStarted { commitment_id: new_id });
    }
    emit_event(
        &app,
        &AppEvent::CommitmentChanged {
            commitment_id: input.to_commitment_id,
        },
    );
    crate::tray::refresh(&app);
    Ok(new_commitment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aos_core::types::Classification;

    #[test]
    fn clearing_another_commitment_preserves_the_live_distraction_episode() {
        let mut engine = crate::state::EngineState::new(crate::db::settings::Settings::default());
        engine.active_commitment = Some(ActiveCommitment {
            id: 7,
            title: "Live focus".into(),
            done_definition: "Keep working on the real active commitment.".into(),
            project_id: None,
        });
        engine.focus_session_id = Some(11);
        engine.tracker.tick(0, Classification::Focused, false);
        engine.tracker.tick(10, Classification::Distracted, false);
        let episode_started_at = engine.tracker.episode_started_at();
        assert!(episode_started_at.is_some());

        assert!(!clear_runtime_focus_if_matches(&mut engine, 99));
        assert_eq!(engine.active_commitment.as_ref().map(|item| item.id), Some(7));
        assert_eq!(engine.focus_session_id, Some(11));
        assert_eq!(engine.tracker.episode_started_at(), episode_started_at);

        assert!(clear_runtime_focus_if_matches(&mut engine, 7));
        assert!(engine.active_commitment.is_none());
        assert!(engine.focus_session_id.is_none());
        assert!(engine.tracker.episode_started_at().is_none());
    }
}
