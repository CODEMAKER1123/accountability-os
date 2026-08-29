//! Focus sessions, check-ins, interruptions, breaks, daily reviews.

use rusqlite::{params, Connection};

use super::models::{BreakRow, CheckinRow, FocusSessionRow, InterruptionRow};
use super::now;
use crate::error::{AppError, AppResult};

// -- Focus sessions ---------------------------------------------------------

pub fn start_focus(conn: &Connection, commitment_id: i64) -> AppResult<FocusSessionRow> {
    // Close any dangling focus session first: one active commitment at a time.
    conn.execute(
        "UPDATE focus_sessions SET ended_at=?1, outcome='switched' WHERE ended_at IS NULL",
        [now()],
    )?;
    conn.execute(
        "INSERT INTO focus_sessions(commitment_id, started_at, created_at) VALUES(?1,?2,?2)",
        params![commitment_id, now()],
    )?;
    get_focus(conn, conn.last_insert_rowid())
}

pub fn get_focus(conn: &Connection, id: i64) -> AppResult<FocusSessionRow> {
    conn.query_row("SELECT * FROM focus_sessions WHERE id=?1", [id], |r| {
        Ok(FocusSessionRow {
            id: r.get("id")?,
            commitment_id: r.get("commitment_id")?,
            started_at: r.get("started_at")?,
            ended_at: r.get("ended_at")?,
            outcome: r.get("outcome")?,
        })
    })
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound(format!("focus session {id}")),
        other => other.into(),
    })
}

pub fn active_focus(conn: &Connection) -> AppResult<Option<FocusSessionRow>> {
    let row = conn.query_row(
        "SELECT * FROM focus_sessions WHERE ended_at IS NULL ORDER BY started_at DESC LIMIT 1",
        [],
        |r| {
            Ok(FocusSessionRow {
                id: r.get("id")?,
                commitment_id: r.get("commitment_id")?,
                started_at: r.get("started_at")?,
                ended_at: r.get("ended_at")?,
                outcome: r.get("outcome")?,
            })
        },
    );
    match row {
        Ok(f) => Ok(Some(f)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn end_focus(conn: &Connection, outcome: &str) -> AppResult<Option<FocusSessionRow>> {
    let active = active_focus(conn)?;
    if let Some(f) = &active {
        conn.execute(
            "UPDATE focus_sessions SET ended_at=?1, outcome=?2 WHERE id=?3",
            params![now(), outcome, f.id],
        )?;
        return Ok(Some(get_focus(conn, f.id)?));
    }
    Ok(None)
}

/// End the open focus session only when it belongs to `commitment_id`.
/// Completing or blocking a different row must never close the user's real
/// active focus session.
pub fn end_focus_for_commitment(
    conn: &Connection,
    commitment_id: i64,
    outcome: &str,
) -> AppResult<Option<FocusSessionRow>> {
    let active = active_focus(conn)?;
    match active {
        Some(f) if f.commitment_id == commitment_id => {
            conn.execute(
                "UPDATE focus_sessions SET ended_at=?1, outcome=?2 WHERE id=?3",
                params![now(), outcome, f.id],
            )?;
            Ok(Some(get_focus(conn, f.id)?))
        }
        _ => Ok(None),
    }
}

// -- Check-ins (spec §18) ---------------------------------------------------

pub fn create_checkin(
    conn: &Connection,
    due_at: i64,
    commitment_id: Option<i64>,
    window_stats: &serde_json::Value,
) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO checkins(due_at, shown_at, commitment_id, window_stats, created_at)
         VALUES(?1,?2,?3,?4,?5)",
        params![due_at, now(), commitment_id, window_stats.to_string(), now()],
    )?;
    Ok(conn.last_insert_rowid())
}

fn checkin_from_row(r: &rusqlite::Row) -> rusqlite::Result<CheckinRow> {
    let stats: String = r.get("window_stats")?;
    Ok(CheckinRow {
        id: r.get("id")?,
        due_at: r.get("due_at")?,
        shown_at: r.get("shown_at")?,
        commitment_id: r.get("commitment_id")?,
        window_stats: serde_json::from_str(&stats).unwrap_or(serde_json::Value::Null),
        response: r.get("response")?,
        response_note: r.get("response_note")?,
    })
}

pub fn unanswered_checkin(conn: &Connection) -> AppResult<Option<CheckinRow>> {
    let row = conn.query_row(
        "SELECT c.*, NULL AS response, NULL AS response_note FROM checkins c
         WHERE NOT EXISTS (SELECT 1 FROM checkin_responses r WHERE r.checkin_id = c.id)
         ORDER BY c.due_at DESC LIMIT 1",
        [],
        checkin_from_row,
    );
    match row {
        Ok(c) => Ok(Some(c)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn answer_checkin(conn: &Connection, checkin_id: i64, response: &str, note: Option<&str>) -> AppResult<()> {
    const VALID: &[&str] = &["yes", "priority_changed", "blocked", "break"];
    if !VALID.contains(&response) {
        return Err(AppError::invalid(format!("Invalid check-in response: {response}")));
    }
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM checkins WHERE id=?1)",
        [checkin_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(AppError::NotFound(format!("check-in {checkin_id}")));
    }
    // Idempotent for UI retries/double-clicks; the first answer is final.
    conn.execute(
        "INSERT INTO checkin_responses(checkin_id, response, note, created_at)
         VALUES(?1,?2,?3,?4)
         ON CONFLICT(checkin_id) DO NOTHING",
        params![checkin_id, response, note, now()],
    )?;
    Ok(())
}

pub fn get_checkin(conn: &Connection, id: i64) -> AppResult<CheckinRow> {
    conn.query_row(
        "SELECT c.*, r.response, r.note AS response_note
         FROM checkins c
         LEFT JOIN checkin_responses r ON r.checkin_id = c.id
         WHERE c.id=?1",
        [id],
        checkin_from_row,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound(format!("check-in {id}")),
        other => other.into(),
    })
}

// -- Interruptions (spec §13–14) --------------------------------------------

pub struct InterruptionContext<'a> {
    pub app_name: &'a str,
    pub process_name: &'a str,
    pub browser_domain: Option<&'a str>,
    pub window_title: &'a str,
}

pub fn create_interruption(
    conn: &Connection,
    kind: &str,
    commitment_id: Option<i64>,
    ctx: &InterruptionContext,
    distracted_secs: i64,
    episode_started_at: Option<i64>,
) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO interruptions(kind, commitment_id, app_name, process_name, browser_domain,
            window_title, distracted_secs, episode_started_at, started_at, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
        params![
            kind,
            commitment_id,
            ctx.app_name,
            ctx.process_name,
            ctx.browser_domain,
            ctx.window_title,
            distracted_secs,
            episode_started_at,
            now()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_interruption(conn: &Connection, id: i64) -> AppResult<InterruptionRow> {
    conn.query_row("SELECT * FROM interruptions WHERE id=?1", [id], interruption_from_row)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound(format!("interruption {id}")),
            other => other.into(),
        })
}

pub fn respond_interruption(
    conn: &Connection,
    id: i64,
    response: &str,
    note: Option<&str>,
) -> AppResult<()> {
    const VALID: &[&str] = &[
        "return", "actually_work", "planned_break", "priority_changed", "blocked", "dismissed",
    ];
    if !VALID.contains(&response) {
        return Err(AppError::invalid(format!("Invalid intervention response: {response}")));
    }
    let n = conn.execute(
        "UPDATE interruptions SET acknowledged_at=?1, response=?2, response_note=?3
         WHERE id=?4 AND acknowledged_at IS NULL",
        params![now(), response, note, id],
    )?;
    if n == 0 {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM interruptions WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?;
        return Err(if exists {
            AppError::invalid("That intervention was already answered.")
        } else {
            AppError::NotFound(format!("interruption {id}"))
        });
    }
    Ok(())
}

pub fn record_recovery(conn: &Connection, id: i64, recovery_secs: i64) -> AppResult<bool> {
    let updated = conn.execute(
        "UPDATE interruptions SET returned_at=?1, recovery_secs=?2
         WHERE id=?3 AND kind='intervention' AND response='return' AND returned_at IS NULL",
        params![now(), recovery_secs, id],
    )?;
    Ok(updated > 0)
}

pub fn open_interruption(conn: &Connection) -> AppResult<Option<InterruptionRow>> {
    let row = conn.query_row(
        "SELECT * FROM interruptions WHERE kind='intervention' AND acknowledged_at IS NULL
         ORDER BY started_at DESC LIMIT 1",
        [],
        interruption_from_row,
    );
    match row {
        Ok(i) => Ok(Some(i)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn interruption_from_row(r: &rusqlite::Row) -> rusqlite::Result<InterruptionRow> {
    Ok(InterruptionRow {
        id: r.get("id")?,
        kind: r.get("kind")?,
        commitment_id: r.get("commitment_id")?,
        app_name: r.get("app_name")?,
        process_name: r.get("process_name")?,
        browser_domain: r.get("browser_domain")?,
        window_title: r.get("window_title")?,
        distracted_secs: r.get("distracted_secs")?,
        episode_started_at: r.get("episode_started_at")?,
        started_at: r.get("started_at")?,
        acknowledged_at: r.get("acknowledged_at")?,
        response: r.get("response")?,
        response_note: r.get("response_note")?,
        returned_at: r.get("returned_at")?,
        recovery_secs: r.get("recovery_secs")?,
    })
}

pub fn recovery_secs_for_range(conn: &Connection, from_ts: i64, to_ts: i64) -> AppResult<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT recovery_secs FROM interruptions
         WHERE recovery_secs IS NOT NULL AND started_at >= ?1 AND started_at < ?2",
    )?;
    let rows = stmt.query_map([from_ts, to_ts], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

// -- Breaks (spec §17) ------------------------------------------------------

pub fn start_break(conn: &Connection, duration_secs: i64) -> AppResult<BreakRow> {
    let ts = now();
    // Starting a new break is a boundary, not a second concurrently-open
    // break. This also repairs a dangling row from an interrupted command.
    conn.execute(
        "UPDATE breaks SET actual_end_at=?1 WHERE actual_end_at IS NULL",
        [ts],
    )?;
    conn.execute(
        "INSERT INTO breaks(started_at, planned_end_at, created_at) VALUES(?1,?2,?1)",
        params![ts, ts + duration_secs.max(60)],
    )?;
    let id = conn.last_insert_rowid();
    conn.query_row("SELECT * FROM breaks WHERE id=?1", [id], break_from_row)
        .map_err(Into::into)
}

/// The break row still marked open, if any (survives restarts).
pub fn open_break(conn: &Connection) -> AppResult<Option<BreakRow>> {
    match conn.query_row(
        "SELECT * FROM breaks WHERE actual_end_at IS NULL ORDER BY started_at DESC LIMIT 1",
        [],
        break_from_row,
    ) {
        Ok(b) => Ok(Some(b)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Close a break at a specific instant (used when a restart finds a break
/// whose planned end already passed).
pub fn close_break_at(conn: &Connection, id: i64, at: i64) -> AppResult<()> {
    conn.execute("UPDATE breaks SET actual_end_at=?1 WHERE id=?2", params![at, id])?;
    Ok(())
}

pub fn close_open_break_at(conn: &Connection, id: i64, at: i64) -> AppResult<bool> {
    let updated = conn.execute(
        "UPDATE breaks SET actual_end_at=?1 WHERE id=?2 AND actual_end_at IS NULL",
        params![at, id],
    )?;
    Ok(updated > 0)
}

pub fn end_break(conn: &Connection) -> AppResult<Option<BreakRow>> {
    let open: Option<i64> = match conn.query_row(
        "SELECT id FROM breaks WHERE actual_end_at IS NULL ORDER BY started_at DESC LIMIT 1",
        [],
        |r| r.get(0),
    ) {
        Ok(id) => Some(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.into()),
    };
    if let Some(id) = open {
        conn.execute("UPDATE breaks SET actual_end_at=?1 WHERE id=?2", params![now(), id])?;
        let b = conn.query_row("SELECT * FROM breaks WHERE id=?1", [id], break_from_row)?;
        return Ok(Some(b));
    }
    Ok(None)
}

fn break_from_row(r: &rusqlite::Row) -> rusqlite::Result<BreakRow> {
    Ok(BreakRow {
        id: r.get("id")?,
        started_at: r.get("started_at")?,
        planned_end_at: r.get("planned_end_at")?,
        actual_end_at: r.get("actual_end_at")?,
    })
}

// -- Daily reviews (spec §21) -----------------------------------------------

pub fn upsert_review(conn: &Connection, plan_id: i64, ai_summary: Option<&str>) -> AppResult<i64> {
    let existing: Option<i64> = match conn.query_row(
        "SELECT id FROM daily_reviews WHERE plan_id=?1",
        [plan_id],
        |r| r.get(0),
    ) {
        Ok(id) => Some(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.into()),
    };
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE daily_reviews SET reviewed_at=?1, ai_summary=COALESCE(?2, ai_summary) WHERE id=?3",
                params![now(), ai_summary, id],
            )?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO daily_reviews(plan_id, reviewed_at, ai_summary, created_at) VALUES(?1,?2,?3,?2)",
                params![plan_id, now(), ai_summary],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }
}

pub fn review_summary(conn: &Connection, plan_id: i64) -> AppResult<Option<String>> {
    match conn.query_row(
        "SELECT ai_summary FROM daily_reviews WHERE plan_id=?1",
        [plan_id],
        |r| r.get::<_, Option<String>>(0),
    ) {
        Ok(s) => Ok(s),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
