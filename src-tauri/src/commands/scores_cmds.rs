//! Today snapshot, scorecards, daily review, analytics (spec §19–23, §44).

use serde::{Deserialize, Serialize};
use tauri::State;

use aos_core::events::{AppEvent, MonitoringState};
use aos_core::patterns::{self, SessionFact};
use aos_core::scoring::DayTotals;

use crate::db::models::{Commitment, DailyPlan, DailyScoreRow, FocusSessionRow, InsightRow};
use crate::db::{engine_data, local_day_bounds, now, plans, scores, today_local};
use crate::engine::emit_event;
use crate::error::{AppError, AppResult};
use crate::state::{ActiveCommitment, AppState, CurrentActivity};

#[derive(Serialize)]
pub struct BreakInfo {
    pub started_at: i64,
    pub ends_at: i64,
}

#[derive(Serialize)]
pub struct CommitmentProgress {
    pub commitment_id: i64,
    pub focused_secs: i64,
}

#[derive(Serialize)]
pub struct TodaySnapshot {
    pub date: String,
    pub plan: Option<DailyPlan>,
    pub commitments: Vec<Commitment>,
    pub active_commitment: Option<ActiveCommitment>,
    pub focus_session: Option<FocusSessionRow>,
    pub totals: DayTotals,
    pub score: DailyScoreRow,
    pub current: Option<CurrentActivity>,
    pub monitoring_state: MonitoringState,
    pub monitoring_message: Option<String>,
    pub next_checkin_at: i64,
    pub current_break: Option<BreakInfo>,
    pub distracted_secs: i64,
    pub warned: bool,
    pub commitment_progress: Vec<CommitmentProgress>,
    pub extension_connected: bool,
}

/// One call that powers the Today view and the mini widget.
#[tauri::command]
pub fn get_today_snapshot(state: State<'_, AppState>) -> AppResult<TodaySnapshot> {
    let date = today_local();
    let (plan, commitments, focus_session, score) = state.db.with(|conn| {
        let plan = plans::get_plan_by_date(conn, &date)?;
        let commitments = match &plan {
            Some(p) => plans::list_commitments(conn, p.id)?,
            None => vec![],
        };
        let focus = engine_data::active_focus(conn)?;
        let score = scores::compute_day_score(conn, &date)?;
        Ok((plan, commitments, focus, score))
    })?;
    let commitment_progress = state.db.with(|conn| {
        commitments
            .iter()
            .map(|c| {
                Ok(CommitmentProgress {
                    commitment_id: c.id,
                    focused_secs: scores::commitment_focused_secs(conn, c.id)?,
                })
            })
            .collect::<AppResult<Vec<_>>>()
    })?;

    let engine = state.engine.lock();
    let mut totals = DayTotals {
        focused_secs: score.focused_secs,
        supporting_secs: score.supporting_secs,
        neutral_secs: score.neutral_secs,
        distracted_secs: score.distracted_secs,
        idle_secs: score.idle_secs,
        unknown_secs: score.unknown_secs,
    };
    let mut score = score;
    let mut commitment_progress = commitment_progress;

    // The open in-memory draft is not persisted yet — without it, hours of
    // continuous work in one window would never move today's numbers. Add
    // its live contribution to the displayed totals and derived scores —
    // but only while monitoring is actually producing samples.
    let monitoring_live = matches!(
        engine.monitoring_state,
        MonitoringState::Active | MonitoringState::Demo
    );
    if let Some(current) = engine.current_activity.as_ref().filter(|_| monitoring_live) {
        let now_ts = now();
        let (day_start, _) = crate::db::local_day_bounds(&date).unwrap_or((0, now_ts));
        let live_secs = (now_ts - current.since.max(day_start)).max(0);
        if live_secs > 0 {
            totals.add(current.outcome.classification, live_secs);
            match current.outcome.classification {
                aos_core::types::Classification::Focused => score.focused_secs += live_secs,
                aos_core::types::Classification::Supporting => score.supporting_secs += live_secs,
                aos_core::types::Classification::Neutral => score.neutral_secs += live_secs,
                aos_core::types::Classification::Distracted => score.distracted_secs += live_secs,
                aos_core::types::Classification::Idle => score.idle_secs += live_secs,
                aos_core::types::Classification::Unknown => score.unknown_secs += live_secs,
            }
            score.alignment = aos_core::scoring::commitment_alignment(&totals);
            score.focus_quality =
                aos_core::scoring::focus_score(&totals, score.context_switches as u32);
            score.total = aos_core::scoring::daily_score(
                score.completion,
                score.alignment,
                score.focus_quality,
                score.planning_accuracy,
            )
            .map(|d| d.total);
            if matches!(
                current.outcome.classification,
                aos_core::types::Classification::Focused | aos_core::types::Classification::Supporting
            ) {
                if let Some(active) = &engine.active_commitment {
                    if let Some(p) = commitment_progress
                        .iter_mut()
                        .find(|p| p.commitment_id == active.id)
                    {
                        p.focused_secs += live_secs;
                    }
                }
            }
        }
    }

    Ok(TodaySnapshot {
        date,
        plan,
        commitments,
        active_commitment: engine.active_commitment.clone(),
        focus_session,
        totals,
        score,
        current: engine.current_activity.clone(),
        monitoring_state: engine.monitoring_state,
        monitoring_message: engine.monitoring_message.clone(),
        next_checkin_at: engine.checkin.next_at(),
        current_break: engine
            .current_break
            .as_ref()
            .map(|(_, b)| BreakInfo {
                started_at: b.started_at,
                ends_at: b.ends_at,
            }),
        distracted_secs: engine.tracker.current_distracted_secs(),
        warned: engine.tracker.is_warned(),
        commitment_progress,
        extension_connected: engine
            .last_extension_report
            .as_ref()
            .is_some_and(|r| now() - r.at <= 60),
    })
}

#[tauri::command]
pub fn get_day_score(state: State<'_, AppState>, date: Option<String>) -> AppResult<DailyScoreRow> {
    let date = date.unwrap_or_else(today_local);
    local_day_bounds(&date)
        .ok_or_else(|| AppError::invalid("Score date must use YYYY-MM-DD."))?;
    state.db.with(|conn| scores::compute_day_score(conn, &date))
}

/// Daily rows for a date range: stored where finalized, computed for today.
#[tauri::command]
pub fn get_scorecard(
    state: State<'_, AppState>,
    from_date: String,
    to_date: String,
) -> AppResult<Vec<DailyScoreRow>> {
    local_day_bounds(&from_date)
        .ok_or_else(|| AppError::invalid("Scorecard dates must use YYYY-MM-DD."))?;
    local_day_bounds(&to_date)
        .ok_or_else(|| AppError::invalid("Scorecard dates must use YYYY-MM-DD."))?;
    if from_date > to_date {
        return Err(AppError::invalid("Scorecard start date must not be after its end date."));
    }
    let today = today_local();
    state.db.with(|conn| {
        let mut rows = scores::list_scores_range(conn, &from_date, &to_date)?;
        if today >= from_date && today <= to_date && !rows.iter().any(|r| r.date == today) {
            let live = scores::compute_day_score(conn, &today)?;
            if live.focused_secs + live.supporting_secs + live.neutral_secs + live.distracted_secs > 0
                || live.completion.is_some()
            {
                rows.push(live);
            }
        }
        rows.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(rows)
    })
}

#[derive(Serialize)]
pub struct ReviewData {
    pub plan: DailyPlan,
    pub commitments: Vec<Commitment>,
    pub score: DailyScoreRow,
    pub commitment_progress: Vec<CommitmentProgress>,
    pub ai_summary: Option<String>,
    pub already_reviewed: bool,
}

#[tauri::command]
pub fn get_review_data(state: State<'_, AppState>, date: Option<String>) -> AppResult<ReviewData> {
    let date = date.unwrap_or_else(today_local);
    local_day_bounds(&date)
        .ok_or_else(|| AppError::invalid("Review date must use YYYY-MM-DD."))?;
    state.db.with(|conn| {
        let plan = plans::get_plan_by_date(conn, &date)?
            .ok_or_else(|| AppError::NotFound(format!("no plan for {date}")))?;
        let commitments = plans::list_commitments(conn, plan.id)?;
        let score = scores::compute_day_score(conn, &date)?;
        let commitment_progress = commitments
            .iter()
            .map(|c| {
                Ok(CommitmentProgress {
                    commitment_id: c.id,
                    focused_secs: scores::commitment_focused_secs(conn, c.id)?,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        let ai_summary = engine_data::review_summary(conn, plan.id)?;
        Ok(ReviewData {
            already_reviewed: plan.ended_at.is_some(),
            plan,
            commitments,
            score,
            commitment_progress,
            ai_summary,
        })
    })
}

#[derive(Deserialize)]
pub struct ReviewItem {
    pub commitment_id: i64,
    pub completed: bool,
    /// Why it wasn't completed (spec §21 choices).
    pub reason: Option<String>,
    pub note: Option<String>,
}

#[derive(Deserialize)]
pub struct ReviewSubmission {
    pub date: Option<String>,
    pub items: Vec<ReviewItem>,
}

/// End-of-day review submission (spec §21): finalizes commitments, stores
/// the day's score, closes the plan.
#[tauri::command]
pub fn submit_review(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: ReviewSubmission,
) -> AppResult<DailyScoreRow> {
    let date = input.date.unwrap_or_else(today_local);
    local_day_bounds(&date).ok_or_else(|| AppError::invalid("Review date must use YYYY-MM-DD."))?;
    const REASONS: &[&str] = &[
        "underestimated_time", "priorities_changed", "distraction", "blocked",
        "too_many_commitments", "task_unclear", "unexpected_work", "other",
    ];
    let plan = state
        .db
        .with(|conn| plans::get_plan_by_date(conn, &date))?
        .ok_or_else(|| AppError::NotFound(format!("no plan for {date}")))?;
    if plan.locked_at.is_none() {
        return Err(AppError::invalid("The day must be locked before it can be reviewed."));
    }
    if plan.ended_at.is_some() {
        return Err(AppError::invalid("That day has already been reviewed."));
    }
    let commitments = state.db.with(|conn| plans::list_commitments(conn, plan.id))?;
    let expected: std::collections::HashSet<i64> = commitments.iter().map(|c| c.id).collect();
    let mut seen = std::collections::HashSet::new();
    for item in &input.items {
        if item
            .note
            .as_deref()
            .is_some_and(|note| note.chars().count() > 2_000)
        {
            return Err(AppError::invalid(
                "Review explanations must be 2,000 characters or fewer.",
            ));
        }
        if !expected.contains(&item.commitment_id) || !seen.insert(item.commitment_id) {
            return Err(AppError::invalid(
                "Review items must contain each commitment from this plan exactly once.",
            ));
        }
        let commitment = commitments
            .iter()
            .find(|c| c.id == item.commitment_id)
            .expect("membership checked");
        if !item.completed {
            if let Some(reason) = &item.reason {
                if !REASONS.contains(&reason.as_str()) {
                    return Err(AppError::invalid(format!("Invalid reason: {reason}")));
                }
            } else if !matches!(commitment.status.as_str(), "deferred" | "cancelled" | "dropped") {
                return Err(AppError::invalid(format!(
                    "Choose why \"{}\" was not completed.",
                    commitment.title
                )));
            }
        }
    }
    if seen != expected {
        return Err(AppError::invalid(
            "Review items must contain each commitment from this plan exactly once.",
        ));
    }

    // Close and classify the open activity under the still-active commitment
    // BEFORE scoring — otherwise the tail of the day's work is missing from
    // the finalized score and later gets attributed to no commitment.
    let _history_guard = state.activity_history_boundary.lock();
    crate::engine::flush_open_session(&app);
    let score = state.db.with_tx(|tx| {
        let current_plan = plans::get_plan_by_date(tx, &date)?
            .ok_or_else(|| AppError::NotFound(format!("no plan for {date}")))?;
        if current_plan.id != plan.id || current_plan.ended_at.is_some() {
            return Err(AppError::invalid("That day is no longer open for review."));
        }
        let current_ids: std::collections::HashSet<i64> = plans::list_commitments(tx, plan.id)?
            .into_iter()
            .map(|commitment| commitment.id)
            .collect();
        if current_ids != expected {
            return Err(AppError::invalid("The plan changed while the review was open. Reload it."));
        }
        for item in &input.items {
            if item.completed {
                plans::set_commitment_status(tx, item.commitment_id, "completed", None, None)?;
            } else {
                let current = plans::get_commitment(tx, item.commitment_id)?;
                let status = if current.status == "active" { "pending" } else { current.status.as_str() };
                plans::set_commitment_status(
                    tx,
                    item.commitment_id,
                    status,
                    item.reason.as_deref(),
                    item.note.as_deref(),
                )?;
            }
        }
        plans::end_day(tx, plan.id)?;
        let score = scores::compute_day_score(tx, &date)?;
        scores::store_day_score(tx, &score)?;
        engine_data::upsert_review(tx, plan.id, None)?;
        if let Some(focus) = engine_data::active_focus(tx)? {
            if expected.contains(&focus.commitment_id) {
                engine_data::end_focus_for_commitment(tx, focus.commitment_id, "day_end")?;
            }
        }
        Ok(score)
    })?;

    {
        let mut engine = state.engine.lock();
        if engine
            .active_commitment
            .as_ref()
            .is_some_and(|commitment| expected.contains(&commitment.id))
        {
            engine.active_commitment = None;
            engine.focus_session_id = None;
        }
        engine.tracker.resolve();
    }
    emit_event(&app, &AppEvent::DayEnded { plan_id: plan.id });
    emit_event(&app, &AppEvent::ScoresUpdated);

    Ok(score)
}

/// Delay the end-of-day review by 30 minutes (spec §21).
#[tauri::command]
pub fn delay_review(state: State<'_, AppState>, minutes: Option<u32>) -> AppResult<()> {
    let mut engine = state.engine.lock();
    engine.review_delay_until = Some(now() + minutes.unwrap_or(30).clamp(5, 240) as i64 * 60);
    engine.review_prompted_date = None;
    Ok(())
}

// -- Analytics (spec §19 distraction stats, §23 patterns) -------------------

#[derive(Serialize)]
pub struct PatternsReport {
    pub days: u32,
    pub hourly: Vec<HourStat>,
    pub top_distractions: Vec<(String, i64)>,
    pub top_apps: Vec<(String, i64)>,
    pub deep_work_blocks: usize,
    pub longest_deep_block_secs: i64,
    pub context_switches: u32,
    pub estimation_bias: Option<f64>,
    pub completion_before_noon: Option<f64>,
    pub completion_after_noon: Option<f64>,
    pub avg_recovery_secs: Option<i64>,
    pub distraction_stats: aos_core::scoring::DistractionStats,
}

#[derive(Serialize)]
pub struct HourStat {
    pub hour: u8,
    pub focused_secs: i64,
    pub distracted_secs: i64,
    pub total_secs: i64,
}

#[tauri::command]
pub fn get_patterns(state: State<'_, AppState>, days: u32) -> AppResult<PatternsReport> {
    let days = days.clamp(1, 365);
    let to_ts = now();
    let from_ts = to_ts - days as i64 * 86400;
    state.db.with(|conn| {
        let facts = scores::session_facts(conn, from_ts, to_ts)?;
        let recovery = engine_data::recovery_secs_for_range(conn, from_ts, to_ts)?;
        let est_pairs = scores::estimate_pairs(conn, from_ts)?;
        let starts = scores::commitment_starts(conn, from_ts)?;
        Ok(build_report(days, &facts, &recovery, &est_pairs, &starts))
    })
}

fn build_report(
    days: u32,
    facts: &[SessionFact],
    recovery: &[i64],
    est_pairs: &[(i64, i64)],
    starts: &[(u8, bool)],
) -> PatternsReport {
    let profile = patterns::hourly_profile(facts);
    let hourly = profile
        .iter()
        .enumerate()
        .map(|(h, b)| HourStat {
            hour: h as u8,
            focused_secs: b.focused_secs,
            distracted_secs: b.distracted_secs,
            total_secs: b.total_secs,
        })
        .collect();
    let blocks = patterns::deep_work_blocks(facts, 25 * 60, 120);
    let (before, after) = patterns::completion_by_start(starts, 12);
    let dstats = aos_core::scoring::distraction_stats(
        facts
            .iter()
            .map(|f| (f.source_label.as_str(), f.classification, f.duration_secs)),
        recovery,
    );
    PatternsReport {
        days,
        hourly,
        top_distractions: patterns::top_sources(facts, aos_core::types::Classification::Distracted, 5),
        top_apps: {
            let mut by: std::collections::HashMap<&str, i64> = Default::default();
            for f in facts {
                if f.classification != aos_core::types::Classification::Idle {
                    *by.entry(f.source_label.as_str()).or_default() += f.duration_secs;
                }
            }
            let mut v: Vec<(String, i64)> = by.into_iter().map(|(k, s)| (k.to_string(), s)).collect();
            v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            v.truncate(8);
            v
        },
        deep_work_blocks: blocks.len(),
        longest_deep_block_secs: blocks.iter().map(|(s, e)| e - s).max().unwrap_or(0),
        context_switches: patterns::context_switches(facts),
        estimation_bias: patterns::estimation_bias(est_pairs),
        completion_before_noon: before,
        completion_after_noon: after,
        avg_recovery_secs: dstats.avg_recovery_secs,
        distraction_stats: dstats,
    }
}

/// Deterministic insights, recomputed on demand and persisted (spec §23).
#[tauri::command]
pub fn get_insights(state: State<'_, AppState>, days: u32) -> AppResult<Vec<InsightRow>> {
    let days = days.clamp(7, 365);
    let period = format!("{days}d");
    let to_ts = now();
    let from_ts = to_ts - days as i64 * 86400;
    state.db.with(|conn| {
        let facts = scores::session_facts(conn, from_ts, to_ts)?;
        let est_pairs = scores::estimate_pairs(conn, from_ts)?;
        let starts = scores::commitment_starts(conn, from_ts)?;
        let insights = patterns::generate_insights(&facts, &starts, &est_pairs);
        scores::replace_insights(conn, &period, &insights, "deterministic")?;
        scores::list_insights(conn, &period)
    })
}

/// Sessions for one local date, bounds included for the timeline axis.
#[derive(Serialize)]
pub struct TimelineData {
    pub date: String,
    pub day_start_ts: i64,
    pub day_end_ts: i64,
    pub sessions: Vec<crate::db::models::ActivitySessionRow>,
}

#[tauri::command]
pub fn get_timeline(state: State<'_, AppState>, date: Option<String>) -> AppResult<TimelineData> {
    let date = date.unwrap_or_else(today_local);
    local_day_bounds(&date)
        .ok_or_else(|| AppError::invalid("Timeline date must use YYYY-MM-DD."))?;
    let (day_start_ts, day_end_ts) =
        local_day_bounds(&date).ok_or_else(|| AppError::invalid("bad date"))?;
    let sessions = state
        .db
        .with(|conn| crate::db::sessions::list_for_date(conn, &date))?;
    Ok(TimelineData {
        date,
        day_start_ts,
        day_end_ts,
        sessions,
    })
}
