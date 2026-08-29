//! Score computation feeds + persistence (spec §19–20, §23).

use rusqlite::{params, Connection};

use aos_core::patterns::{Insight, SessionFact};
use aos_core::scoring::{self, CommitmentOutcome, DayTotals};
use aos_core::types::{Classification, Priority};

use super::models::DailyScoreRow;
use super::{local_day_bounds, local_hour_of, now};
use crate::error::AppResult;

pub fn day_totals(conn: &Connection, date: &str) -> AppResult<DayTotals> {
    let mut totals = DayTotals::default();
    let mut stmt = conn.prepare(
        "SELECT classification, duration_seconds FROM activity_sessions WHERE local_date=?1",
    )?;
    let rows = stmt.query_map([date], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (class, secs) = row?;
        if let Some(c) = Classification::parse(&class) {
            totals.add(c, secs);
        }
    }
    Ok(totals)
}

pub fn session_facts(conn: &Connection, from_ts: i64, to_ts: i64) -> AppResult<Vec<SessionFact>> {
    let mut stmt = conn.prepare(
        "SELECT started_at, duration_seconds, classification, application_name, browser_domain
         FROM activity_sessions WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at",
    )?;
    let rows = stmt.query_map([from_ts, to_ts], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut facts = vec![];
    for row in rows {
        let (started_at, duration_secs, class, app, domain) = row?;
        let Some(classification) = Classification::parse(&class) else {
            continue;
        };
        let label = domain.unwrap_or(app);
        // Pre-split each session at local hour boundaries, labeling every
        // chunk from its own timestamp via chrono. This is DST-exact — on
        // transition days the hour LABEL jumps but absolute time doesn't,
        // so per-chunk lookup lands in the right bucket, unlike cycling
        // hour indices in fixed 3600s steps. Consecutive chunks share a
        // source label, so context-switch and deep-work analyses see no
        // artificial transitions.
        let mut seg_start = started_at;
        let mut remaining = duration_secs.max(0);
        loop {
            let into = super::local_secs_into_hour(seg_start) as i64;
            let chunk = remaining.min((3600 - into.min(3599)).max(1));
            facts.push(SessionFact {
                started_at: seg_start,
                duration_secs: chunk,
                classification,
                source_label: label.clone(),
                local_hour: local_hour_of(seg_start),
                secs_into_hour: into as u32,
            });
            remaining -= chunk;
            if remaining <= 0 {
                break;
            }
            seg_start += chunk;
        }
    }
    Ok(facts)
}

/// Focused+supporting seconds attributed to one commitment.
pub fn commitment_focused_secs(conn: &Connection, commitment_id: i64) -> AppResult<i64> {
    let secs: i64 = conn.query_row(
        "SELECT COALESCE(SUM(duration_seconds), 0) FROM activity_sessions
         WHERE related_commitment_id=?1 AND classification IN ('focused','supporting')",
        [commitment_id],
        |r| r.get(0),
    )?;
    Ok(secs)
}

/// Compute (without storing) the full score row for a local date.
pub fn compute_day_score(conn: &Connection, date: &str) -> AppResult<DailyScoreRow> {
    let totals = day_totals(conn, date)?;
    let (from_ts, to_ts) = local_day_bounds(date).unwrap_or((0, 0));
    let facts = session_facts(conn, from_ts, to_ts)?;
    let switches = aos_core::patterns::context_switches(&facts);

    // Commitment outcomes + planning accuracy for the day's plan.
    let mut outcomes: Vec<CommitmentOutcome> = vec![];
    let mut est_total: i64 = 0;
    let mut actual_total: i64 = 0;
    let mut have_estimates = false;
    let plan_id: Option<i64> = match conn.query_row(
        "SELECT id FROM daily_plans WHERE date=?1 AND locked_at IS NOT NULL",
        [date],
        |r| r.get(0),
    ) {
        Ok(id) => Some(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.into()),
    };
    if let Some(plan_id) = plan_id {
        let mut stmt = conn.prepare(
            "SELECT id, priority, status, estimated_minutes FROM daily_commitments WHERE plan_id=?1",
        )?;
        let rows = stmt.query_map([plan_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?;
        for row in rows {
            let (cid, priority, status, est_min) = row?;
            // Intentionally deferred/cancelled commitments don't count against
            // execution (spec §15: reprioritization is not failure).
            if status == "deferred" || status == "cancelled" {
                continue;
            }
            outcomes.push(CommitmentOutcome {
                priority: Priority::parse(&priority).unwrap_or(Priority::Should),
                completed: status == "completed",
            });
            if let Some(est) = est_min {
                if est > 0 {
                    have_estimates = true;
                    est_total += est * 60;
                    actual_total += commitment_focused_secs(conn, cid)?;
                }
            }
        }
    }

    let completion = scoring::execution_score(&outcomes);
    let alignment = scoring::commitment_alignment(&totals);
    let focus_quality = scoring::focus_score(&totals, switches);
    let planning = if have_estimates {
        scoring::planning_accuracy(est_total, actual_total)
    } else {
        None
    };
    let daily = scoring::daily_score(completion, alignment, focus_quality, planning);

    Ok(DailyScoreRow {
        date: date.to_string(),
        total: daily.map(|d| d.total),
        completion,
        alignment,
        focus_quality,
        planning_accuracy: planning,
        focused_secs: totals.focused_secs,
        supporting_secs: totals.supporting_secs,
        neutral_secs: totals.neutral_secs,
        distracted_secs: totals.distracted_secs,
        idle_secs: totals.idle_secs,
        unknown_secs: totals.unknown_secs,
        context_switches: switches as i64,
    })
}

pub fn store_day_score(conn: &Connection, row: &DailyScoreRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO daily_scores(date, total, completion, alignment, focus_quality, planning_accuracy,
            focused_secs, supporting_secs, neutral_secs, distracted_secs, idle_secs, unknown_secs,
            context_switches, computed_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         ON CONFLICT(date) DO UPDATE SET total=excluded.total, completion=excluded.completion,
            alignment=excluded.alignment, focus_quality=excluded.focus_quality,
            planning_accuracy=excluded.planning_accuracy, focused_secs=excluded.focused_secs,
            supporting_secs=excluded.supporting_secs, neutral_secs=excluded.neutral_secs,
            distracted_secs=excluded.distracted_secs, idle_secs=excluded.idle_secs,
            unknown_secs=excluded.unknown_secs, context_switches=excluded.context_switches,
            computed_at=excluded.computed_at",
        params![
            row.date,
            row.total,
            row.completion,
            row.alignment,
            row.focus_quality,
            row.planning_accuracy,
            row.focused_secs,
            row.supporting_secs,
            row.neutral_secs,
            row.distracted_secs,
            row.idle_secs,
            row.unknown_secs,
            row.context_switches,
            now(),
        ],
    )?;
    Ok(())
}

/// Recompute + overwrite the stored score for a date, if one exists.
/// Manual corrections and late AI classifications change session data after
/// a day was finalized; the stored score must follow.
pub fn refresh_stored_score(conn: &Connection, date: &str) -> AppResult<()> {
    let exists = match conn.query_row("SELECT 1 FROM daily_scores WHERE date=?1", [date], |_| Ok(())) {
        Ok(()) => true,
        Err(rusqlite::Error::QueryReturnedNoRows) => false,
        Err(e) => return Err(e.into()),
    };
    if exists {
        let row = compute_day_score(conn, date)?;
        store_day_score(conn, &row)?;
    }
    Ok(())
}

/// `refresh_stored_score` keyed by a session id (used by async AI updates).
pub fn refresh_stored_score_for_session(conn: &Connection, session_id: i64) -> AppResult<()> {
    let date: Option<String> = match conn.query_row(
        "SELECT local_date FROM activity_sessions WHERE id=?1",
        [session_id],
        |r| r.get(0),
    ) {
        Ok(d) => Some(d),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.into()),
    };
    match date {
        Some(d) => refresh_stored_score(conn, &d),
        None => Ok(()),
    }
}

pub fn list_scores_range(conn: &Connection, from_date: &str, to_date: &str) -> AppResult<Vec<DailyScoreRow>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM daily_scores WHERE date >= ?1 AND date <= ?2 ORDER BY date",
    )?;
    let rows = stmt.query_map([from_date, to_date], |r| {
        Ok(DailyScoreRow {
            date: r.get("date")?,
            total: r.get("total")?,
            completion: r.get("completion")?,
            alignment: r.get("alignment")?,
            focus_quality: r.get("focus_quality")?,
            planning_accuracy: r.get("planning_accuracy")?,
            focused_secs: r.get("focused_secs")?,
            supporting_secs: r.get("supporting_secs")?,
            neutral_secs: r.get("neutral_secs")?,
            distracted_secs: r.get("distracted_secs")?,
            idle_secs: r.get("idle_secs")?,
            unknown_secs: r.get("unknown_secs")?,
            context_switches: r.get("context_switches")?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

// -- Insights (spec §23–24) -------------------------------------------------

pub fn replace_insights(conn: &Connection, period: &str, insights: &[Insight], source: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM ai_insights WHERE period=?1 AND source=?2",
        params![period, source],
    )?;
    for i in insights {
        conn.execute(
            "INSERT INTO ai_insights(period, metric, text, source, created_at) VALUES(?1,?2,?3,?4,?5)",
            params![period, i.metric, i.text, source, now()],
        )?;
    }
    Ok(())
}

pub fn list_insights(conn: &Connection, period: &str) -> AppResult<Vec<super::models::InsightRow>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM ai_insights WHERE period=?1 ORDER BY created_at DESC, id",
    )?;
    let rows = stmt.query_map([period], |r| {
        Ok(super::models::InsightRow {
            id: r.get("id")?,
            period: r.get("period")?,
            metric: r.get("metric")?,
            text: r.get("text")?,
            source: r.get("source")?,
            created_at: r.get("created_at")?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Historical estimate-vs-actual pairs for pattern analysis.
pub fn estimate_pairs(conn: &Connection, from_ts: i64) -> AppResult<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.estimated_minutes FROM daily_commitments c
         JOIN daily_plans p ON p.id = c.plan_id
         WHERE c.estimated_minutes IS NOT NULL AND c.estimated_minutes > 0
           AND c.status='completed' AND p.locked_at >= ?1",
    )?;
    let rows = stmt.query_map([from_ts], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut pairs = vec![];
    for row in rows {
        let (cid, est_min) = row?;
        let actual = commitment_focused_secs(conn, cid)?;
        if actual > 0 {
            pairs.push((est_min * 60, actual));
        }
    }
    Ok(pairs)
}

/// (start_hour, completed) pairs for completion-by-start-time analysis.
pub fn commitment_starts(conn: &Connection, from_ts: i64) -> AppResult<Vec<(u8, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT started_at, status FROM daily_commitments
         WHERE started_at IS NOT NULL AND started_at >= ?1 AND status IN ('completed','pending','active','dropped')",
    )?;
    let rows = stmt.query_map([from_ts], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = vec![];
    for row in rows {
        let (started_at, status) = row?;
        out.push((local_hour_of(started_at), status == "completed"));
    }
    Ok(out)
}
