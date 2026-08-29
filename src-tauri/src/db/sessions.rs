//! Activity sessions + manual corrections (spec §8, §30, §42).

use rusqlite::{params, Connection, Row};

use aos_core::aggregator::normalize_title;
use aos_core::types::{Classification, ClassifyOutcome, SessionDraft};

use super::models::ActivitySessionRow;
use super::{local_date_of, now};
use crate::error::{AppError, AppResult};

fn session_from_row(row: &Row) -> rusqlite::Result<ActivitySessionRow> {
    Ok(ActivitySessionRow {
        id: row.get("id")?,
        local_date: row.get("local_date")?,
        started_at: row.get("started_at")?,
        ended_at: row.get("ended_at")?,
        duration_seconds: row.get("duration_seconds")?,
        application_name: row.get("application_name")?,
        process_name: row.get("process_name")?,
        window_title: row.get("window_title")?,
        browser_domain: row.get("browser_domain")?,
        browser_title: row.get("browser_title")?,
        classification: row.get("classification")?,
        classification_confidence: row.get("classification_confidence")?,
        classification_source: row.get("classification_source")?,
        classification_reason: row.get("classification_reason")?,
        related_commitment_id: row.get("related_commitment_id")?,
        is_idle: row.get::<_, i64>("is_idle")? != 0,
        pending_ai: row.get::<_, i64>("pending_ai")? != 0,
    })
}

/// Remove the legacy cache key plus every semantic-versioned variant. The
/// base key can contain user-controlled title characters, so use `substr`
/// instead of treating it as a LIKE/GLOB pattern.
fn delete_cache_key_variants(conn: &Connection, key: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM classification_cache
         WHERE cache_key=?1
            OR substr(cache_key, 1, length(?1) + 2) = ?1 || '|s'",
        [key],
    )?;
    Ok(())
}

/// Insert a session, splitting it at local-midnight boundaries so each
/// day's totals only ever count time that belongs to that local date.
/// Returns every inserted segment id (chronological order) — a pending-AI
/// result must patch all of them, not just the newest.
pub fn insert(
    conn: &Connection,
    draft: &SessionDraft,
    outcome: &ClassifyOutcome,
    commitment_id: Option<i64>,
    pending_ai: bool,
) -> AppResult<Vec<i64>> {
    let end = draft.ended_at.max(draft.started_at);
    let mut seg_start = draft.started_at;
    let mut ids = Vec::with_capacity(1);
    loop {
        let date = local_date_of(seg_start);
        let day_end = super::local_day_bounds(&date).map(|(_, e)| e).unwrap_or(end);
        let seg_end = if day_end > seg_start { end.min(day_end) } else { end };
        conn.execute(
            "INSERT INTO activity_sessions(local_date, started_at, ended_at, duration_seconds,
                application_name, process_name, window_title, browser_domain, browser_title,
                classification, classification_confidence, classification_source,
                classification_reason, related_commitment_id, is_idle, pending_ai, created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![
                date,
                seg_start,
                seg_end,
                (seg_end - seg_start).max(0),
                draft.app_name,
                draft.process_name,
                draft.window_title,
                draft.browser_domain,
                draft.browser_title,
                outcome.classification.as_str(),
                outcome.confidence,
                outcome.source.as_str(),
                outcome.reason,
                commitment_id,
                draft.is_idle as i64,
                pending_ai as i64,
                now(),
            ],
        )?;
        ids.push(conn.last_insert_rowid());
        if seg_end >= end {
            break;
        }
        seg_start = seg_end;
    }
    Ok(ids)
}

/// Apply an async AI/cache result to a session — but ONLY while it is still
/// awaiting one. A manual correction made in the meantime cleared
/// `pending_ai` and must never be overwritten by a late AI response.
/// Returns whether the row was still pending and got updated.
pub fn update_classification(
    conn: &Connection,
    session_id: i64,
    outcome: &ClassifyOutcome,
) -> AppResult<bool> {
    let changed = conn.execute(
        "UPDATE activity_sessions SET classification=?1, classification_confidence=?2,
            classification_source=?3, classification_reason=?4, pending_ai=0
         WHERE id=?5 AND pending_ai=1 AND classification_source<>'manual'",
        params![
            outcome.classification.as_str(),
            outcome.confidence,
            outcome.source.as_str(),
            outcome.reason,
            session_id
        ],
    )?;
    Ok(changed > 0)
}

/// The AI request failed or can't run: stop showing these rows as awaiting
/// AI (classification stays Unknown, user-correctable). Returns how many
/// rows were still pending.
pub fn clear_pending_ai(conn: &Connection, session_ids: &[i64]) -> AppResult<usize> {
    let mut cleared = 0;
    for id in session_ids {
        cleared += conn.execute(
            "UPDATE activity_sessions SET pending_ai=0,
                classification_reason='AI unavailable — mark it in the Activity view'
             WHERE id=?1 AND pending_ai=1",
            [id],
        )?;
    }
    Ok(cleared)
}

pub fn get(conn: &Connection, id: i64) -> AppResult<ActivitySessionRow> {
    conn.query_row("SELECT * FROM activity_sessions WHERE id=?1", [id], session_from_row)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound(format!("session {id}")),
            other => other.into(),
        })
}

pub fn list_for_date(conn: &Connection, date: &str) -> AppResult<Vec<ActivitySessionRow>> {
    let mut stmt = conn
        .prepare("SELECT * FROM activity_sessions WHERE local_date=?1 ORDER BY started_at")?;
    let rows = stmt.query_map([date], session_from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn list_range(conn: &Connection, from_ts: i64, to_ts: i64) -> AppResult<Vec<ActivitySessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM activity_sessions WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at",
    )?;
    let rows = stmt.query_map([from_ts, to_ts], session_from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Free-text search across activity history (spec §43).
pub fn search(conn: &Connection, query: &str, limit: u32) -> AppResult<Vec<ActivitySessionRow>> {
    let like = format!("%{}%", query.trim());
    let mut stmt = conn.prepare(
        "SELECT * FROM activity_sessions
         WHERE application_name LIKE ?1 OR window_title LIKE ?1 OR browser_domain LIKE ?1
         ORDER BY started_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![like, limit], session_from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Apply a manual correction (spec §42): update the session, remember the
/// correction so classification learns, optionally spawn a rule.
pub struct CorrectionRecord {
    pub session_id: i64,
    pub new_classification: String,
    pub reason: Option<String>,
    pub commitment_id: Option<i64>,
    pub project_id: Option<i64>,
}

pub fn apply_correction(conn: &Connection, rec: &CorrectionRecord) -> AppResult<ActivitySessionRow> {
    let new_classification = Classification::parse(rec.new_classification.trim())
        .filter(|classification| {
            matches!(
                classification,
                Classification::Focused
                    | Classification::Supporting
                    | Classification::Neutral
                    | Classification::Distracted
            )
        })
        .ok_or_else(|| AppError::invalid("Invalid manual classification."))?;
    let session = get(conn, rec.session_id)?;
    conn.execute(
        "UPDATE activity_sessions SET classification=?1, classification_confidence=1.0,
            classification_source='manual', classification_reason=?2, pending_ai=0 WHERE id=?3",
        params![
            new_classification.as_str(),
            rec.reason.as_deref().unwrap_or("Manually corrected"),
            rec.session_id
        ],
    )?;
    conn.execute(
        "INSERT INTO activity_corrections(session_id, process_name, browser_domain,
            normalized_title, commitment_id, project_id, old_classification,
            new_classification, reason, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            rec.session_id,
            session.process_name,
            session.browser_domain,
            normalize_title(&session.window_title),
            rec.commitment_id.or(session.related_commitment_id),
            rec.project_id,
            session.classification,
            new_classification.as_str(),
            rec.reason,
            now(),
        ],
    )?;
    // Invalidate any cached AI answer for this exact context.
    let key = aos_core::classify::cache_key(
        rec.commitment_id.or(session.related_commitment_id),
        &session.process_name,
        session.browser_domain.as_deref(),
        &session.window_title,
    );
    delete_cache_key_variants(conn, &key)?;
    get(conn, rec.session_id)
}

pub fn load_corrections(conn: &Connection) -> AppResult<Vec<aos_core::classify::Correction>> {
    let mut stmt = conn.prepare(
        "SELECT id, process_name, browser_domain, normalized_title, commitment_id, project_id,
                new_classification
         FROM activity_corrections ORDER BY created_at DESC LIMIT 2000",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, Option<i64>>(5)?,
            r.get::<_, String>(6)?,
        ))
    })?;
    let mut out = vec![];
    for row in rows {
        let (id, process_name, browser_domain, normalized_title, commitment_id, project_id, class) = row?;
        if let Some(classification) = aos_core::types::Classification::parse(&class) {
            out.push(aos_core::classify::Correction {
                id,
                process_name,
                browser_domain,
                normalized_title,
                commitment_id,
                project_id,
                classification,
            });
        }
    }
    Ok(out)
}

/// Validate a deletion range and return its inclusive local-day bounds.
/// Commands call this before crossing the in-memory privacy boundary; the
/// database operation calls it again so lower-level callers stay protected.
pub fn deletion_range_bounds(from_date: &str, to_date: &str) -> AppResult<(i64, i64)> {
    if from_date > to_date {
        return Err(AppError::invalid("The deletion start date must not be after the end date."));
    }
    let (from_ts, _) = super::local_day_bounds(from_date)
        .ok_or_else(|| AppError::invalid("Deletion dates must use YYYY-MM-DD."))?;
    let (_, to_ts) = super::local_day_bounds(to_date)
        .ok_or_else(|| AppError::invalid("Deletion dates must use YYYY-MM-DD."))?;
    Ok((from_ts, to_ts))
}

/// Delete monitoring history (spec §50). Corrections derived from the
/// deleted sessions carry the same process/domain/title metadata, so they
/// are deleted too — "delete my history" must not leave it exportable or
/// still teaching the classifier. Returns session rows removed.
pub fn delete_range(conn: &Connection, from_date: &str, to_date: &str) -> AppResult<usize> {
    let (from_ts, to_ts) = deletion_range_bounds(from_date, to_date)?;
    let observed_at = now();
    delete_cache_for_sessions(
        conn,
        "SELECT related_commitment_id, process_name, browser_domain, window_title
         FROM activity_sessions WHERE local_date >= ?1 AND local_date <= ?2",
        params![from_date, to_date],
    )?;
    // Grouped rows are copies of ONE memory, anchored once per date it was
    // learned from: deleting any of those dates must remove every copy, or
    // the survivors keep the same metadata exportable and still teaching
    // the classifier.
    conn.execute(
        "DELETE FROM activity_corrections WHERE group_id IS NOT NULL AND group_id IN (
            SELECT group_id FROM activity_corrections
            WHERE group_id IS NOT NULL AND session_id IN (
                SELECT id FROM activity_sessions WHERE local_date >= ?1 AND local_date <= ?2))",
        params![from_date, to_date],
    )?;
    conn.execute(
        "DELETE FROM activity_corrections
         WHERE session_id IN (
           SELECT id FROM activity_sessions WHERE local_date >= ?1 AND local_date <= ?2
         ) OR (session_id IS NULL AND created_at >= ?3 AND created_at < ?4)",
        params![from_date, to_date, from_ts, to_ts],
    )?;
    let n = conn.execute(
        "DELETE FROM activity_sessions WHERE local_date >= ?1 AND local_date <= ?2",
        params![from_date, to_date],
    )?;
    conn.execute(
        "DELETE FROM classification_cache WHERE created_at >= ?1 AND created_at < ?2",
        params![from_ts, to_ts],
    )?;
    conn.execute(
        "DELETE FROM interruptions
         WHERE COALESCE(episode_started_at, started_at) < ?2
           AND started_at >= ?1",
        params![from_ts, to_ts],
    )?;
    conn.execute(
        "DELETE FROM checkins WHERE due_at >= ?1 AND due_at < ?2",
        params![from_ts, to_ts],
    )?;
    // Focus and break rows are timestamped monitoring history too. Delete
    // every row whose observed interval overlaps the selected local days;
    // an open row only contributes history through the time of deletion.
    conn.execute(
        "DELETE FROM focus_sessions
         WHERE started_at < ?2 AND COALESCE(ended_at, ?3) > ?1",
        params![from_ts, to_ts, observed_at],
    )?;
    conn.execute(
        "DELETE FROM breaks
         WHERE started_at < ?2 AND COALESCE(actual_end_at, ?3) > ?1",
        params![from_ts, to_ts, observed_at],
    )?;
    conn.execute(
        "DELETE FROM daily_scores WHERE date >= ?1 AND date <= ?2",
        params![from_date, to_date],
    )?;
    conn.execute(
        "UPDATE daily_reviews SET ai_summary=NULL
         WHERE plan_id IN (SELECT id FROM daily_plans WHERE date >= ?1 AND date <= ?2)",
        params![from_date, to_date],
    )?;
    // Insights aggregate arbitrary periods, so any activity deletion makes
    // every stored narrative stale. They are cheap to regenerate on demand.
    conn.execute("DELETE FROM ai_insights", [])?;
    Ok(n)
}

pub fn delete_all(conn: &Connection) -> AppResult<usize> {
    conn.execute("DELETE FROM activity_corrections", [])?;
    let n = conn.execute("DELETE FROM activity_sessions", [])?;
    conn.execute("DELETE FROM classification_cache", [])?;
    conn.execute("DELETE FROM interruptions", [])?;
    conn.execute("DELETE FROM checkins", [])?;
    conn.execute("DELETE FROM focus_sessions", [])?;
    conn.execute("DELETE FROM breaks", [])?;
    conn.execute("DELETE FROM daily_scores", [])?;
    conn.execute("DELETE FROM ai_insights", [])?;
    conn.execute("UPDATE daily_reviews SET ai_summary=NULL", [])?;
    Ok(n)
}

/// Retention cleanup: drop sessions older than N days. The retention
/// period applies to correction memory too — a correction carries the same
/// process/domain/title metadata as the sessions it was learned from, so
/// it must not outlive them (nor keep teaching the classifier). Grouped
/// copies die as a unit, exactly as in delete_range.
pub fn prune_older_than(conn: &Connection, days: u32) -> AppResult<usize> {
    let cutoff = now() - days as i64 * 86400;
    delete_cache_for_sessions(
        conn,
        "SELECT related_commitment_id, process_name, browser_domain, window_title
         FROM activity_sessions WHERE ended_at < ?1",
        [cutoff],
    )?;
    conn.execute(
        "DELETE FROM activity_corrections WHERE group_id IS NOT NULL AND group_id IN (
            SELECT group_id FROM activity_corrections
            WHERE group_id IS NOT NULL AND session_id IN (
                SELECT id FROM activity_sessions WHERE ended_at < ?1))",
        [cutoff],
    )?;
    conn.execute(
        "DELETE FROM activity_corrections
         WHERE session_id IN (SELECT id FROM activity_sessions WHERE ended_at < ?1)
            OR (session_id IS NULL AND created_at < ?1)",
        [cutoff],
    )?;
    let n = conn.execute(
        "DELETE FROM activity_sessions WHERE ended_at < ?1",
        [cutoff],
    )?;
    conn.execute("DELETE FROM classification_cache WHERE created_at < ?1", [cutoff])?;
    conn.execute(
        "DELETE FROM interruptions
         WHERE COALESCE(episode_started_at, started_at) < ?1",
        [cutoff],
    )?;
    conn.execute("DELETE FROM checkins WHERE due_at < ?1", [cutoff])?;
    // These rows cannot be split at an arbitrary timestamp. If their start
    // has crossed the privacy cutoff, remove the whole history record rather
    // than retain an out-of-window timestamp in exports.
    conn.execute("DELETE FROM focus_sessions WHERE started_at < ?1", [cutoff])?;
    conn.execute("DELETE FROM breaks WHERE started_at < ?1", [cutoff])?;
    let cutoff_date = local_date_of(cutoff);
    // The timestamp cutoff falls partway through cutoff_date, so that day's
    // derived totals and narrative are already stale even though newer rows
    // from the same local day remain. Drop them for safe recomputation.
    conn.execute("DELETE FROM daily_scores WHERE date <= ?1", [&cutoff_date])?;
    conn.execute(
        "UPDATE daily_reviews SET ai_summary=NULL
         WHERE plan_id IN (SELECT id FROM daily_plans WHERE date <= ?1)",
        [&cutoff_date],
    )?;
    if n > 0 {
        conn.execute("DELETE FROM ai_insights", [])?;
    }
    Ok(n)
}

/// Remove cached AI classifications derived from the selected sessions even
/// when the cache row was created on a different day. Cache keys contain the
/// normalized process, domain and title, so they are monitoring data too.
fn delete_cache_for_sessions<P: rusqlite::Params>(
    conn: &Connection,
    query: &str,
    params: P,
) -> AppResult<()> {
    let keys = {
        let mut stmt = conn.prepare(query)?;
        let rows = stmt.query_map(params, |row| {
            Ok(aos_core::classify::cache_key(
                row.get::<_, Option<i64>>(0)?,
                &row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.as_deref(),
                &row.get::<_, String>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for key in keys {
        delete_cache_key_variants(conn, &key)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_a_base_cache_key_removes_semantic_variants_only() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE classification_cache(cache_key TEXT PRIMARY KEY);
             INSERT INTO classification_cache(cache_key) VALUES
               ('c42|peditor|d|tbrief'),
               ('c42|peditor|d|tbrief|s0123456789abcdef'),
               ('c42|peditor|d|tbriefing|s0123456789abcdef'),
               ('c43|peditor|d|tbrief|s0123456789abcdef');",
        )
        .unwrap();

        delete_cache_key_variants(&conn, "c42|peditor|d|tbrief").unwrap();

        let remaining = conn
            .prepare("SELECT cache_key FROM classification_cache ORDER BY cache_key")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            remaining,
            vec![
                "c42|peditor|d|tbriefing|s0123456789abcdef",
                "c43|peditor|d|tbrief|s0123456789abcdef"
            ]
        );
    }
}
