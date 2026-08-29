//! Morning interview + daily plan commands (spec §5–6).

use serde::Serialize;
use tauri::State;

use aos_core::events::AppEvent;

use crate::db::models::{Commitment, DailyPlan};
use crate::db::plans::{self, LockDayInput, ReviseDayInput};
use crate::db::{now, today_local};
use crate::engine::emit_event;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Serialize)]
pub struct TodayPlan {
    pub plan: Option<DailyPlan>,
    pub commitments: Vec<Commitment>,
}

#[tauri::command]
pub fn get_today_plan(state: State<'_, AppState>) -> AppResult<TodayPlan> {
    get_plan_for_date(state, today_local())
}

#[tauri::command]
pub fn get_plan_for_date(state: State<'_, AppState>, date: String) -> AppResult<TodayPlan> {
    crate::db::local_day_bounds(&date)
        .ok_or_else(|| AppError::invalid("Plan date must use YYYY-MM-DD."))?;
    state.db.with(|conn| {
        let plan = plans::get_plan_by_date(conn, &date)?;
        let commitments = match &plan {
            Some(p) => plans::list_commitments(conn, p.id)?,
            None => vec![],
        };
        Ok(TodayPlan { plan, commitments })
    })
}

/// LOCK MY DAY (spec §6). Anchors the check-in cadence at lock time.
#[tauri::command]
pub fn lock_day(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: LockDayInput,
) -> AppResult<TodayPlan> {
    if input.date != today_local() {
        return Err(AppError::invalid("The morning interview can lock only today's plan."));
    }
    let (plan, commitments) = state.db.with_tx(|tx| plans::lock_day(tx, &input))?;
    {
        let mut engine = state.engine.lock();
        engine.checkin.last_at = now();
        engine.interview_snoozes = 0;
        engine.interview_snoozed_until = None;
    }
    emit_event(&app, &AppEvent::DayLocked { plan_id: plan.id });
    Ok(TodayPlan {
        plan: Some(plan),
        commitments,
    })
}

/// Edit today's locked contract without discarding focus history or progress.
#[tauri::command]
pub fn revise_day(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: ReviseDayInput,
) -> AppResult<TodayPlan> {
    if input.date != today_local() {
        return Err(AppError::invalid("Only today's plan can be edited."));
    }
    // Keep the final old-context activity and the plan mutation on one side
    // of the same monitoring boundary used by focus and break transitions.
    let _history_guard = state.activity_history_boundary.lock();
    let expected_impact = state.db.with(|conn| plans::validate_revision(conn, &input))?;
    if expected_impact.semantics_changed() {
        crate::engine::flush_open_session(&app);
        let prefixes = expected_impact
            .semantic_commitment_ids
            .iter()
            .map(|id| format!("c{id}|"))
            .collect::<Vec<_>>();
        let mut engine = state.engine.lock();
        // An in-flight answer was prompted with the old title/definition.
        // Cancel it before the durable cache is cleared so it cannot restore
        // a stale answer after this edit.
        state.invalidate_activity_tasks_with_engine(&mut engine);
        engine
            .classification_cache
            .retain(|key, _| !prefixes.iter().any(|prefix| key.starts_with(prefix)));
    }
    let (plan, commitments, applied_impact) = state.db.with_tx(|tx| {
        let result = plans::revise_day(tx, &input)?;
        if result.2.semantics_changed() {
            for id in &result.2.semantic_commitment_ids {
                tx.execute(
                    "DELETE FROM classification_cache WHERE cache_key LIKE ?1",
                    [format!("c{id}|%")],
                )?;
            }
            // The generation boundary above cancels every in-flight task.
            // Do not leave its placeholder sessions stuck as pending.
            tx.execute("UPDATE activity_sessions SET pending_ai=0 WHERE pending_ai=1", [])?;
        }
        Ok(result)
    })?;
    debug_assert_eq!(expected_impact, applied_impact);
    {
        let mut engine = state.engine.lock();
        if let Some(active) = engine.active_commitment.as_mut() {
            if let Some(updated) = commitments.iter().find(|item| item.id == active.id) {
                active.title = updated.title.clone();
                active.done_definition = updated.done_definition.clone();
            }
        }
    }
    emit_event(
        &app,
        &AppEvent::CommitmentChanged {
            commitment_id: None,
        },
    );
    Ok(TodayPlan {
        plan: Some(plan),
        commitments,
    })
}

#[tauri::command]
pub fn mark_day_off(state: State<'_, AppState>, date: Option<String>) -> AppResult<DailyPlan> {
    let date = date.unwrap_or_else(today_local);
    crate::db::local_day_bounds(&date)
        .ok_or_else(|| AppError::invalid("Day-off date must use YYYY-MM-DD."))?;
    state.db.with(|conn| plans::mark_day_off(conn, &date))
}

#[derive(Serialize)]
pub struct SnoozeResult {
    pub allowed: bool,
    pub message: Option<String>,
    pub snoozed_until: Option<i64>,
}

/// Snooze the morning interview 15 minutes (spec §5). Strict Mode caps it.
#[tauri::command]
pub fn snooze_interview(state: State<'_, AppState>, minutes: Option<u32>) -> AppResult<SnoozeResult> {
    let mut engine = state.engine.lock();
    let policy = aos_core::accountability::StrictPolicy {
        enabled: engine.settings.strict_mode,
        max_interview_snoozes: 2,
    };
    if !policy.can_snooze_interview(engine.interview_snoozes) {
        return Ok(SnoozeResult {
            allowed: false,
            message: Some("Strict Mode: no more snoozes today. Plan the day or mark it off.".into()),
            snoozed_until: None,
        });
    }
    let until = now() + minutes.unwrap_or(15).clamp(1, 120) as i64 * 60;
    engine.interview_snoozes += 1;
    engine.interview_snoozed_until = Some(until);
    engine.interview_prompted_date = None; // allow re-prompt after the snooze
    Ok(SnoozeResult {
        allowed: true,
        message: None,
        snoozed_until: Some(until),
    })
}

/// The over-commitment pushback line, exposed so the UI never duplicates it.
#[tauri::command]
pub fn commitment_limit_check(selected: usize) -> Option<String> {
    aos_core::accountability::too_many_commitments_message(selected)
}

#[tauri::command]
pub fn set_commitment_step_completed(
    state: State<'_, AppState>,
    commitment_id: i64,
    step_index: usize,
    completed: bool,
) -> AppResult<Commitment> {
    state.db.with(|conn| {
        plans::set_commitment_step_completed(conn, commitment_id, step_index, completed)
    })
}
