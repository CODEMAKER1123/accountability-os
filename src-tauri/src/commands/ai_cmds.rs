//! AI configuration + coaching commands (spec §22, §24, §35).

use serde::Serialize;
use tauri::State;

use crate::db::{engine_data, now, plans, scores, today_local};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub fn set_ai_key(state: State<'_, AppState>, key: String) -> AppResult<bool> {
    if key.trim().chars().count() > 4_096 {
        return Err(AppError::invalid("The API key is too long."));
    }
    crate::ai::store_api_key(&key)?;
    *state.ai_key.lock() = if key.trim().is_empty() { None } else { Some(key.trim().to_string()) };
    Ok(!key.trim().is_empty())
}

#[tauri::command]
pub fn has_ai_key(state: State<'_, AppState>) -> AppResult<bool> {
    let mut cached = state.ai_key.lock();
    if cached.is_none() {
        *cached = crate::ai::load_api_key()?;
    }
    Ok(cached.is_some())
}

#[tauri::command]
pub async fn break_down_goal(
    state: State<'_, AppState>,
    goal: String,
    detail: String,
) -> AppResult<crate::ai::GoalBreakdown> {
    let goal = goal.trim();
    if goal.chars().count() < 3 || goal.chars().count() > 300 {
        return Err(AppError::invalid(
            "A goal must be between 3 and 300 characters to break it down.",
        ));
    }
    let detail = crate::ai::BreakdownDetail::parse(&detail)
        .ok_or_else(|| AppError::invalid("Breakdown detail must be simple, standard, or detailed."))?;
    if !state.engine.lock().settings.ai_coaching_enabled {
        return Err(AppError::Ai("AI coaching is disabled in Settings.".into()));
    }
    let (base_url, key, _, coach_model) = ai_credentials(&state)?;
    crate::ai::break_down_goal(
        &state.http,
        &base_url,
        &key,
        &coach_model,
        goal,
        detail,
    )
    .await
}

fn ai_credentials(state: &State<'_, AppState>) -> AppResult<(String, String, String, String)> {
    let (base_url, classify_model, coach_model) = {
        let engine = state.engine.lock();
        (
            engine.settings.ai_base_url.clone(),
            engine.settings.ai_classify_model.clone(),
            engine.settings.ai_coach_model.clone(),
        )
    };
    let key = {
        let mut cached = state.ai_key.lock();
        if cached.is_none() {
            *cached = crate::ai::load_api_key()?;
        }
        cached.clone()
    }
    .ok_or_else(|| AppError::Ai("No API key configured. Add one in Settings → AI.".into()))?;
    Ok((base_url, key, classify_model, coach_model))
}

/// Live connectivity test: run a real classification round-trip.
#[tauri::command]
pub async fn test_ai_connection(state: State<'_, AppState>) -> AppResult<String> {
    let (base_url, key, classify_model, _) = ai_credentials(&state)?;
    let req = crate::ai::ClassifyRequest {
        commitment_title: "Write the quarterly report".into(),
        done_definition: "Report drafted and sent for review".into(),
        app_name: "Word".into(),
        window_title: "Quarterly Report.docx - Word".into(),
        browser_domain: None,
        browser_title: None,
    };
    let out = crate::ai::classify_activity(&state.http, &base_url, &key, &classify_model, &req).await?;
    Ok(format!(
        "Connected. Test classification: {} ({:.0}% confidence).",
        out.classification.as_str(),
        out.confidence * 100.0
    ))
}

#[derive(Serialize)]
pub struct MorningCoach {
    pub text: String,
    pub source: String, // "ai" | "deterministic"
    /// The numbers the advice is based on — always shown (spec §20 ethos).
    pub avg_completed_per_day: Option<f64>,
    pub estimation_bias: Option<f64>,
    pub completion_before_noon: Option<f64>,
    pub completion_after_noon: Option<f64>,
}

/// Morning coach (spec §24): history-grounded pushback for the interview.
/// `proposed` is what the user typed as candidate outcomes today.
#[tauri::command]
pub async fn get_morning_coach(
    state: State<'_, AppState>,
    proposed: Vec<String>,
) -> AppResult<MorningCoach> {
    if proposed.len() > 10
        || proposed
            .iter()
            .any(|title| title.trim().is_empty() || title.chars().count() > 300)
    {
        return Err(AppError::invalid(
            "Morning coaching accepts up to 10 non-empty outcomes of 300 characters each.",
        ));
    }
    let activity_generation = state.activity_generation();
    let from_ts = now() - 14 * 86400;
    let (avg_completed, bias, before, after, plans_count) = state.db.with(|conn| {
        let (done, plans_count): (i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(CASE WHEN c.status='completed' THEN 1 ELSE 0 END),0),
                    COUNT(DISTINCT p.id)
             FROM daily_plans p LEFT JOIN daily_commitments c ON c.plan_id = p.id
             WHERE p.locked_at IS NOT NULL AND p.locked_at >= ?1",
            [from_ts],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let pairs = scores::estimate_pairs(conn, from_ts)?;
        let starts = scores::commitment_starts(conn, from_ts)?;
        let (b, a) = aos_core::patterns::completion_by_start(&starts, 12);
        let avg = if plans_count > 0 {
            Some(done as f64 / plans_count as f64)
        } else {
            None
        };
        Ok((avg, aos_core::patterns::estimation_bias(&pairs), b, a, plans_count))
    })?;

    // Deterministic fallback text from the same numbers.
    let mut lines: Vec<String> = vec![];
    if let Some(avg) = avg_completed {
        if proposed.len() > 3 || (avg > 0.0 && proposed.len() as f64 > avg + 1.0) {
            lines.push(format!(
                "You completed an average of {:.1} major commitments per day over the last {} planned days. You are proposing {}.",
                avg,
                plans_count,
                proposed.len()
            ));
        }
    }
    if let Some(b) = bias {
        if b > 1.25 {
            lines.push(format!(
                "Your focused work typically takes {:.0}% longer than you estimate. Pad today's estimates.",
                (b - 1.0) * 100.0
            ));
        }
    }
    if let (Some(bn), Some(an)) = (before, after) {
        if bn - an >= 15.0 {
            lines.push(format!(
                "You complete {bn:.0}% of commitments started before noon vs {an:.0}% after. Start the most important thing first."
            ));
        }
    }

    let ai_enabled = { state.engine.lock().settings.ai_coaching_enabled };
    if ai_enabled {
        if let Ok((base_url, key, _, coach_model)) = ai_credentials(&state) {
            let prompt = format!(
                "The user is planning their day. Their proposed outcomes:\n{}\n\nTheir history (last 14 days): \
                 avg completed commitments/day: {}; estimation bias (actual/estimated): {}; \
                 completion rate before noon: {}; after noon: {}.\n\n\
                 Give at most 3 short, specific pieces of pushback or advice for today's plan. \
                 Only use the numbers provided. If the data is insufficient, say less.",
                if proposed.is_empty() { "(none listed)".to_string() } else { proposed.join("\n") },
                fmt_opt(avg_completed),
                fmt_opt(bias),
                fmt_opt(before),
                fmt_opt(after),
            );
            match crate::ai::coach(&state.http, &base_url, &key, &coach_model, &prompt).await {
                Ok(text) => {
                    if state.activity_generation() != activity_generation {
                        return Err(AppError::Ai(
                            "Activity data changed while the analysis was running. Try again.".into(),
                        ));
                    }
                    return Ok(MorningCoach {
                        text,
                        source: "ai".into(),
                        avg_completed_per_day: avg_completed,
                        estimation_bias: bias,
                        completion_before_noon: before,
                        completion_after_noon: after,
                    })
                }
                Err(e) => log::warn!(target: "ai", "morning coach failed, falling back: {e}"),
            }
        }
    }
    Ok(MorningCoach {
        text: lines.join("\n"),
        source: "deterministic".into(),
        avg_completed_per_day: avg_completed,
        estimation_bias: bias,
        completion_before_noon: before,
        completion_after_noon: after,
    })
}

/// Daily AI review (spec §22): short factual analysis, stored with the review.
#[tauri::command]
pub async fn generate_daily_ai_review(state: State<'_, AppState>, date: Option<String>) -> AppResult<String> {
    let activity_generation = state.activity_generation();
    let date = date.unwrap_or_else(today_local);
    crate::db::local_day_bounds(&date)
        .ok_or_else(|| AppError::invalid("Review date must use YYYY-MM-DD."))?;
    let (base_url, key, _, coach_model) = ai_credentials(&state)?;
    let coaching_on = { state.engine.lock().settings.ai_coaching_enabled };
    if !coaching_on {
        return Err(AppError::Ai("AI coaching is disabled in Settings.".into()));
    }

    let (plan, prompt, database_changes) = state.db.with(|conn| {
        let plan = plans::get_plan_by_date(conn, &date)?
            .ok_or_else(|| AppError::NotFound(format!("no plan for {date}")))?;
        if plan.ended_at.is_none() {
            return Err(AppError::invalid(
                "Close out the day before generating its AI review.",
            ));
        }
        let commitments = plans::list_commitments(conn, plan.id)?;
        let score = scores::compute_day_score(conn, &date)?;
        let mut lines = vec![format!(
            "Day summary for {date}: focused {}m, supporting {}m, neutral {}m, distracted {}m, idle {}m. \
             Context switches: {}. Alignment: {}. Overall score: {}.",
            score.focused_secs / 60,
            score.supporting_secs / 60,
            score.neutral_secs / 60,
            score.distracted_secs / 60,
            score.idle_secs / 60,
            score.context_switches,
            fmt_opt(score.alignment),
            fmt_opt(score.total),
        )];
        for c in &commitments {
            let focused = scores::commitment_focused_secs(conn, c.id)? / 60;
            lines.push(format!(
                "Commitment \"{}\" [{}]: status {}, estimated {}m, actual focused {}m{}",
                c.title,
                c.priority,
                c.status,
                c.estimated_minutes.unwrap_or(0),
                focused,
                c.outcome_reason
                    .as_deref()
                    .map(|r| format!(", miss reason: {r}"))
                    .unwrap_or_default()
            ));
        }
        let dstats = {
            let (from_ts, to_ts) = crate::db::local_day_bounds(&date).unwrap_or((0, 0));
            let facts = scores::session_facts(conn, from_ts, to_ts)?;
            aos_core::scoring::distraction_stats(
                facts.iter().map(|f| (f.source_label.as_str(), f.classification, f.duration_secs)),
                &[],
            )
        };
        if let Some((src, secs)) = dstats.top_sources.first() {
            lines.push(format!("Top distraction: {} ({}m).", src, secs / 60));
        }
        lines.push(
            "Write a concise end-of-day analysis (max 100 words): what actually happened, the single \
             biggest issue, and one concrete adjustment for tomorrow. Numbers over adjectives. \
             No praise, no filler."
                .into(),
        );
        Ok((plan, lines.join("\n"), conn.total_changes()))
    })?;

    let text = crate::ai::coach(&state.http, &base_url, &key, &coach_model, &prompt).await?;
    if state.activity_generation() != activity_generation {
        return Err(AppError::Ai(
            "Activity data changed while the review was running. Generate it again.".into(),
        ));
    }
    state
        .db
        .with(|conn| {
            if state.activity_generation() != activity_generation {
                return Err(AppError::Ai(
                    "Activity data changed while the review was running. Generate it again.".into(),
                ));
            }
            // SQLite serializes every app write through this connection.
            // Unlike activity_generation (which cancels privacy-boundary
            // tasks), total_changes also catches ordinary session inserts
            // and late AI/manual classification updates. Check while the DB
            // lock is held so no change can race the stored narrative.
            ensure_database_unchanged(conn, database_changes)?;
            engine_data::upsert_review(conn, plan.id, Some(&text))
        })?;
    Ok(text)
}

/// AI-narrated long-term insights on top of the deterministic ones (spec §23).
#[tauri::command]
pub async fn generate_ai_insights(state: State<'_, AppState>, days: u32) -> AppResult<Vec<crate::db::models::InsightRow>> {
    let activity_generation = state.activity_generation();
    if !state.engine.lock().settings.ai_coaching_enabled {
        return Err(AppError::Ai("AI coaching is disabled in Settings.".into()));
    }
    let days = days.clamp(7, 365);
    let period = format!("{days}d");
    let (base_url, key, _, coach_model) = ai_credentials(&state)?;

    let (deterministic, database_changes) = state.db.with(|conn| {
        let to_ts = now();
        let from_ts = to_ts - days as i64 * 86400;
        let facts = scores::session_facts(conn, from_ts, to_ts)?;
        let est_pairs = scores::estimate_pairs(conn, from_ts)?;
        let starts = scores::commitment_starts(conn, from_ts)?;
        Ok((
            aos_core::patterns::generate_insights(&facts, &starts, &est_pairs),
            conn.total_changes(),
        ))
    })?;
    if deterministic.is_empty() {
        return Err(AppError::Ai(format!(
            "Not enough history in the last {days} days to generate insights."
        )));
    }
    let prompt = format!(
        "Observed productivity patterns over the last {days} days:\n{}\n\n\
         Rewrite these as at most 4 sharp, specific observations the user should act on. \
         Keep every number exactly as given. One sentence each.",
        deterministic
            .iter()
            .map(|i| format!("- {}", i.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let text = crate::ai::coach(&state.http, &base_url, &key, &coach_model, &prompt).await?;
    if state.activity_generation() != activity_generation {
        return Err(AppError::Ai(
            "Activity data changed while the insight was running. Generate it again.".into(),
        ));
    }
    let insights: Vec<aos_core::patterns::Insight> = text
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•']).trim())
        .filter(|l| l.len() > 10)
        .take(4)
        .map(|l| aos_core::patterns::Insight {
            metric: "ai_narrative".into(),
            text: l.to_string(),
        })
        .collect();
    state.db.with(|conn| {
        if state.activity_generation() != activity_generation {
            return Err(AppError::Ai(
                "Activity data changed while the insight was running. Generate it again.".into(),
            ));
        }
        // Session inserts and classification/correction updates do not cross
        // the privacy generation. Verify the exact database snapshot while
        // its write lock is held before replacing the derived narrative.
        ensure_database_unchanged(conn, database_changes)?;
        scores::replace_insights(conn, &period, &insights, "ai")?;
        scores::list_insights(conn, &period)
    })
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "n/a".into())
}

fn ensure_database_unchanged(conn: &rusqlite::Connection, expected_changes: u64) -> AppResult<()> {
    if conn.total_changes() != expected_changes {
        return Err(AppError::Ai(
            "Activity data changed while the AI analysis was running. Generate it again.".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_analysis_snapshot_detects_late_session_insert_and_classification() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE activity_sessions(
                id INTEGER PRIMARY KEY,
                classification TEXT NOT NULL
             );",
        )
        .unwrap();
        let before_insert = conn.total_changes();
        ensure_database_unchanged(&conn, before_insert).unwrap();
        conn.execute(
            "INSERT INTO activity_sessions(classification) VALUES('unknown')",
            [],
        )
        .unwrap();
        assert!(ensure_database_unchanged(&conn, before_insert).is_err());

        let before_classification = conn.total_changes();
        conn.execute(
            "UPDATE activity_sessions SET classification='focused' WHERE id=1",
            [],
        )
        .unwrap();
        assert!(ensure_database_unchanged(&conn, before_classification).is_err());
    }
}
