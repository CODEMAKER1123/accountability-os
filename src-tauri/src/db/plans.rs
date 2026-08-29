//! Daily plans + commitments (spec §5–7, §15).

use rusqlite::{params, Connection, Row};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use super::models::{Commitment, CommitmentStep, DailyPlan};
use super::now;
use crate::error::{AppError, AppResult};
use aos_core::accountability::{too_many_commitments_message, MAX_COMMITMENTS};

fn plan_from_row(row: &Row) -> rusqlite::Result<DailyPlan> {
    Ok(DailyPlan {
        id: row.get("id")?,
        date: row.get("date")?,
        locked_at: row.get("locked_at")?,
        ended_at: row.get("ended_at")?,
        likely_distraction: row.get("likely_distraction")?,
        countermeasure: row.get("countermeasure")?,
        most_important_when: row.get("most_important_when")?,
        is_day_off: row.get::<_, i64>("is_day_off")? != 0,
        created_at: row.get("created_at")?,
    })
}

fn commitment_from_row(row: &Row) -> rusqlite::Result<Commitment> {
    let steps_json: String = row.get("steps")?;
    Ok(Commitment {
        id: row.get("id")?,
        plan_id: row.get("plan_id")?,
        task_id: row.get("task_id")?,
        title: row.get("title")?,
        done_definition: row.get("done_definition")?,
        estimated_minutes: row.get("estimated_minutes")?,
        priority: row.get("priority")?,
        rank: row.get("rank")?,
        status: row.get("status")?,
        started_at: row.get("started_at")?,
        completed_at: row.get("completed_at")?,
        outcome_reason: row.get("outcome_reason")?,
        outcome_note: row.get("outcome_note")?,
        steps: serde_json::from_str(&steps_json).unwrap_or_default(),
    })
}

pub fn get_plan_by_date(conn: &Connection, date: &str) -> AppResult<Option<DailyPlan>> {
    match conn.query_row("SELECT * FROM daily_plans WHERE date=?1", [date], plan_from_row) {
        Ok(p) => Ok(Some(p)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn get_plan(conn: &Connection, id: i64) -> AppResult<DailyPlan> {
    conn.query_row("SELECT * FROM daily_plans WHERE id=?1", [id], plan_from_row)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound(format!("plan {id}")),
            other => other.into(),
        })
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitmentInput {
    pub task_id: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub done_definition: String,
    pub estimated_minutes: Option<i64>,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub steps: Vec<String>,
}

fn default_priority() -> String {
    "must".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct LockDayInput {
    pub date: String,
    pub commitments: Vec<CommitmentInput>,
    #[serde(default)]
    pub likely_distraction: String,
    #[serde(default)]
    pub countermeasure: String,
    #[serde(default = "default_when")]
    pub most_important_when: String,
    /// Raw interview answers, kept for the record.
    #[serde(default)]
    pub interview_answers: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviseCommitmentInput {
    pub id: Option<i64>,
    #[serde(flatten)]
    pub commitment: CommitmentInput,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviseDayInput {
    pub date: String,
    pub commitments: Vec<ReviseCommitmentInput>,
    #[serde(default)]
    pub likely_distraction: String,
    #[serde(default)]
    pub countermeasure: String,
    #[serde(default = "default_when")]
    pub most_important_when: String,
    #[serde(default)]
    pub interview_answers: serde_json::Value,
}

impl ReviseDayInput {
    fn validation_input(&self) -> LockDayInput {
        LockDayInput {
            date: self.date.clone(),
            commitments: self
                .commitments
                .iter()
                .map(|item| item.commitment.clone())
                .collect(),
            likely_distraction: self.likely_distraction.clone(),
            countermeasure: self.countermeasure.clone(),
            most_important_when: self.most_important_when.clone(),
            interview_answers: self.interview_answers.clone(),
        }
    }
}

fn default_when() -> String {
    "flexible".into()
}

const MAX_COMMITMENT_STEPS: usize = 12;
const MAX_STEP_CHARACTERS: usize = 300;

fn validated_step_titles(steps: &[String]) -> AppResult<Vec<String>> {
    if steps.len() > MAX_COMMITMENT_STEPS {
        return Err(AppError::invalid(format!(
            "A commitment can have at most {MAX_COMMITMENT_STEPS} action steps."
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let mut validated = Vec::with_capacity(steps.len());
    for step in steps {
        let title = step.trim();
        if title.is_empty() {
            return Err(AppError::invalid("Action steps cannot be empty."));
        }
        if title.chars().count() > MAX_STEP_CHARACTERS {
            return Err(AppError::invalid(format!(
                "Action steps must be {MAX_STEP_CHARACTERS} characters or fewer."
            )));
        }
        if seen.insert(title.to_lowercase()) {
            validated.push(title.to_string());
        }
    }
    Ok(validated)
}

fn validate_day_input(input: &LockDayInput) -> AppResult<Vec<Vec<String>>> {
    chrono::NaiveDate::parse_from_str(&input.date, "%Y-%m-%d")
        .map_err(|_| AppError::invalid("Plan date must use YYYY-MM-DD."))?;
    if input.commitments.is_empty() {
        return Err(AppError::invalid(
            "Commit to at least one outcome before locking the day.",
        ));
    }
    if let Some(msg) = too_many_commitments_message(input.commitments.len()) {
        return Err(AppError::Invalid(msg));
    }
    if input.likely_distraction.chars().count() > 2_000
        || input.countermeasure.chars().count() > 2_000
    {
        return Err(AppError::invalid(
            "Distraction and countermeasure notes must be 2,000 characters or fewer.",
        ));
    }
    if serde_json::to_vec(&input.interview_answers)?.len() > 100_000 {
        return Err(AppError::invalid("Interview answers are too large."));
    }
    let mut prepared_steps = Vec::with_capacity(input.commitments.len());
    for c in &input.commitments {
        if c.title.trim().is_empty() {
            return Err(AppError::invalid("Every commitment needs a title."));
        }
        if c.title.trim().chars().count() > 300 {
            return Err(AppError::invalid(
                "Commitment titles must be 300 characters or fewer.",
            ));
        }
        if c.done_definition.trim().len() < 10 {
            return Err(AppError::invalid(format!(
                "\"{}\" needs a clear definition of DONE (at least a sentence).",
                c.title.trim()
            )));
        }
        if c.done_definition.trim().chars().count() > 2_000 {
            return Err(AppError::invalid(
                "Definitions of done must be 2,000 characters or fewer.",
            ));
        }
        if !matches!(c.priority.as_str(), "must" | "should" | "could") {
            return Err(AppError::invalid(format!(
                "Invalid commitment priority: {}",
                c.priority
            )));
        }
        if c
            .estimated_minutes
            .is_some_and(|minutes| !(1..=24 * 60).contains(&minutes))
        {
            return Err(AppError::invalid(
                "Commitment estimates must be between 1 minute and 24 hours.",
            ));
        }
        prepared_steps.push(validated_step_titles(&c.steps)?);
    }
    let valid_when = matches!(
        input.most_important_when.as_str(),
        "now" | "before_lunch" | "flexible"
    ) || input
        .most_important_when
        .strip_prefix("specific:")
        .is_some_and(|time| chrono::NaiveTime::parse_from_str(time, "%H:%M").is_ok());
    if !valid_when {
        return Err(AppError::invalid(
            "Invalid time for the most important commitment.",
        ));
    }
    Ok(prepared_steps)
}

fn normalized_step_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Create + lock the daily plan (spec §6 "LOCK MY DAY").
pub fn lock_day(tx: &rusqlite::Transaction, input: &LockDayInput) -> AppResult<(DailyPlan, Vec<Commitment>)> {
    let prepared_steps = validate_day_input(input)?;

    let ts = now();
    let existing = get_plan_by_date(tx, &input.date)?;
    let plan_id = match existing {
        Some(p) if p.locked_at.is_some() => {
            return Err(AppError::invalid("Today's plan is already locked."));
        }
        Some(p) => {
            tx.execute(
                "UPDATE daily_plans SET locked_at=?1, likely_distraction=?2, countermeasure=?3,
                        most_important_when=?4, interview_answers=?5, is_day_off=0 WHERE id=?6",
                params![
                    ts,
                    input.likely_distraction,
                    input.countermeasure,
                    input.most_important_when,
                    serde_json::to_string(&input.interview_answers)?,
                    p.id
                ],
            )?;
            tx.execute("DELETE FROM daily_commitments WHERE plan_id=?1", [p.id])?;
            p.id
        }
        None => {
            tx.execute(
                "INSERT INTO daily_plans(date, locked_at, likely_distraction, countermeasure,
                        most_important_when, interview_answers, created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    input.date,
                    ts,
                    input.likely_distraction,
                    input.countermeasure,
                    input.most_important_when,
                    serde_json::to_string(&input.interview_answers)?,
                    ts
                ],
            )?;
            tx.last_insert_rowid()
        }
    };

    for (i, (c, step_titles)) in input
        .commitments
        .iter()
        .zip(prepared_steps.iter())
        .take(MAX_COMMITMENTS)
        .enumerate()
    {
        let steps = step_titles
            .iter()
            .map(|title| CommitmentStep {
                title: title.clone(),
                completed: false,
            })
            .collect::<Vec<_>>();
        tx.execute(
            "INSERT INTO daily_commitments(plan_id, task_id, title, done_definition,
                    estimated_minutes, priority, rank, status, created_at, steps)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9)",
            params![
                plan_id,
                c.task_id,
                c.title.trim(),
                c.done_definition.trim(),
                c.estimated_minutes,
                c.priority,
                (i + 1) as i64,
                ts,
                serde_json::to_string(&steps)?
            ],
        )?;
        // Committing a backlog task moves it to `committed` (spec task statuses).
        if let Some(task_id) = c.task_id {
            tx.execute(
                "UPDATE tasks SET status='committed' WHERE id=?1 AND status IN ('inbox','planned')",
                [task_id],
            )?;
        }
    }

    let plan = get_plan(tx, plan_id)?;
    let commitments = list_commitments(tx, plan_id)?;
    Ok((plan, commitments))
}

/// Safely revise a locked day without replacing rows that already own focus
/// history, outcomes, or checked action steps.
pub fn revise_day(
    tx: &rusqlite::Transaction,
    input: &ReviseDayInput,
) -> AppResult<(DailyPlan, Vec<Commitment>)> {
    let validation_input = input.validation_input();
    let prepared_steps = validate_day_input(&validation_input)?;
    let plan = get_plan_by_date(tx, &input.date)?
        .ok_or_else(|| AppError::invalid("There is no plan to edit for this day."))?;
    if plan.locked_at.is_none() {
        return Err(AppError::invalid("This day is not locked yet. Lock it instead."));
    }
    if plan.ended_at.is_some() {
        return Err(AppError::invalid("A completed day can no longer be edited."));
    }
    if plan.is_day_off {
        return Err(AppError::invalid("A day marked off has no active plan to edit."));
    }

    let existing = list_commitments(tx, plan.id)?;
    let existing_by_id: HashMap<i64, &Commitment> =
        existing.iter().map(|commitment| (commitment.id, commitment)).collect();
    let mut submitted_ids = HashSet::new();
    for item in &input.commitments {
        let Some(id) = item.id else {
            continue;
        };
        let Some(current) = existing_by_id.get(&id) else {
            return Err(AppError::invalid(
                "One of those commitments does not belong to today's plan.",
            ));
        };
        if !submitted_ids.insert(id) {
            return Err(AppError::invalid(
                "The revised plan contains the same commitment more than once.",
            ));
        }
        if current.task_id != item.commitment.task_id {
            return Err(AppError::invalid(
                "An existing commitment cannot be linked to a different backlog task.",
            ));
        }
    }

    let removed = existing
        .iter()
        .filter(|commitment| !submitted_ids.contains(&commitment.id))
        .collect::<Vec<_>>();
    if removed
        .iter()
        .any(|commitment| commitment.status != "pending" || commitment.started_at.is_some())
    {
        return Err(AppError::invalid(
            "Started, completed, or otherwise closed commitments must stay in today's record. You can still edit their details.",
        ));
    }

    tx.execute(
        "UPDATE daily_plans SET likely_distraction=?1, countermeasure=?2,
                most_important_when=?3, interview_answers=?4 WHERE id=?5",
        params![
            input.likely_distraction.trim(),
            input.countermeasure.trim(),
            input.most_important_when,
            serde_json::to_string(&input.interview_answers)?,
            plan.id
        ],
    )?;

    for commitment in removed {
        tx.execute("DELETE FROM daily_commitments WHERE id=?1", [commitment.id])?;
        if let Some(task_id) = commitment.task_id {
            tx.execute(
                "UPDATE tasks SET status='planned'
                 WHERE id=?1 AND status='committed' AND NOT EXISTS(
                   SELECT 1 FROM daily_commitments
                   WHERE task_id=?1 AND status IN ('pending','active')
                 )",
                [task_id],
            )?;
        }
    }

    let ts = now();
    for (index, (item, step_titles)) in input
        .commitments
        .iter()
        .zip(prepared_steps.iter())
        .enumerate()
    {
        let rank = (index + 1) as i64;
        match item.id {
            Some(id) => {
                let current = existing_by_id[&id];
                let completed_steps: HashMap<String, bool> = current
                    .steps
                    .iter()
                    .map(|step| (normalized_step_title(&step.title), step.completed))
                    .collect();
                let steps = step_titles
                    .iter()
                    .map(|title| CommitmentStep {
                        title: title.clone(),
                        completed: completed_steps
                            .get(&normalized_step_title(title))
                            .copied()
                            .unwrap_or(false),
                    })
                    .collect::<Vec<_>>();
                tx.execute(
                    "UPDATE daily_commitments SET title=?1, done_definition=?2,
                            estimated_minutes=?3, priority=?4, rank=?5, steps=?6
                     WHERE id=?7 AND plan_id=?8",
                    params![
                        item.commitment.title.trim(),
                        item.commitment.done_definition.trim(),
                        item.commitment.estimated_minutes,
                        item.commitment.priority,
                        rank,
                        serde_json::to_string(&steps)?,
                        id,
                        plan.id
                    ],
                )?;
            }
            None => {
                let steps = step_titles
                    .iter()
                    .map(|title| CommitmentStep {
                        title: title.clone(),
                        completed: false,
                    })
                    .collect::<Vec<_>>();
                tx.execute(
                    "INSERT INTO daily_commitments(plan_id, task_id, title, done_definition,
                            estimated_minutes, priority, rank, status, created_at, steps)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9)",
                    params![
                        plan.id,
                        item.commitment.task_id,
                        item.commitment.title.trim(),
                        item.commitment.done_definition.trim(),
                        item.commitment.estimated_minutes,
                        item.commitment.priority,
                        rank,
                        ts,
                        serde_json::to_string(&steps)?
                    ],
                )?;
                if let Some(task_id) = item.commitment.task_id {
                    tx.execute(
                        "UPDATE tasks SET status='committed'
                         WHERE id=?1 AND status IN ('inbox','planned')",
                        [task_id],
                    )?;
                }
            }
        }
    }

    Ok((get_plan(tx, plan.id)?, list_commitments(tx, plan.id)?))
}

/// Mark today as off / vacation (spec §5).
pub fn mark_day_off(conn: &Connection, date: &str) -> AppResult<DailyPlan> {
    let ts = now();
    match get_plan_by_date(conn, date)? {
        Some(p) => {
            conn.execute("UPDATE daily_plans SET is_day_off=1 WHERE id=?1", [p.id])?;
        }
        None => {
            conn.execute(
                "INSERT INTO daily_plans(date, is_day_off, created_at) VALUES(?1,1,?2)",
                params![date, ts],
            )?;
        }
    }
    Ok(get_plan_by_date(conn, date)?.expect("just created"))
}

pub fn list_commitments(conn: &Connection, plan_id: i64) -> AppResult<Vec<Commitment>> {
    let mut stmt =
        conn.prepare("SELECT * FROM daily_commitments WHERE plan_id=?1 ORDER BY rank")?;
    let rows = stmt.query_map([plan_id], commitment_from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get_commitment(conn: &Connection, id: i64) -> AppResult<Commitment> {
    conn.query_row("SELECT * FROM daily_commitments WHERE id=?1", [id], commitment_from_row)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound(format!("commitment {id}")),
            other => other.into(),
        })
}

/// Ensure a commitment can be activated by the live focus workflow.
pub fn actionable_commitment(conn: &Connection, id: i64) -> AppResult<Commitment> {
    let commitment = get_commitment(conn, id)?;
    let plan = get_plan(conn, commitment.plan_id)?;
    if plan.locked_at.is_none() || plan.ended_at.is_some() || plan.is_day_off {
        return Err(AppError::invalid("That commitment is not in an open, locked plan."));
    }
    if plan.date != super::today_local() {
        return Err(AppError::invalid("Only a commitment from today's plan can be started."));
    }
    if matches!(
        commitment.status.as_str(),
        "completed" | "deferred" | "dropped" | "cancelled"
    ) {
        return Err(AppError::invalid("That commitment is already closed."));
    }
    Ok(commitment)
}

/// Make one commitment active and demote any previous active row. Call this
/// in the same transaction that rotates the open focus session.
pub fn activate_commitment(conn: &Connection, id: i64) -> AppResult<Commitment> {
    actionable_commitment(conn, id)?;
    conn.execute(
        "UPDATE daily_commitments SET status='pending' WHERE status='active' AND id<>?1",
        [id],
    )?;
    set_commitment_status(conn, id, "active", None, None)
}

pub fn set_commitment_status(
    conn: &Connection,
    id: i64,
    status: &str,
    outcome_reason: Option<&str>,
    outcome_note: Option<&str>,
) -> AppResult<Commitment> {
    const VALID: &[&str] = &["pending", "active", "completed", "deferred", "dropped", "cancelled"];
    if !VALID.contains(&status) {
        return Err(AppError::invalid(format!("Invalid commitment status: {status}")));
    }
    let previous = get_commitment(conn, id)?;
    let ts = now();
    let started_sql = if status == "active" { Some(ts) } else { None };
    let completed_sql = if status == "completed" { Some(ts) } else { None };
    let n = conn.execute(
        "UPDATE daily_commitments SET
            status=?1,
            started_at=COALESCE(started_at, ?2),
            completed_at=CASE WHEN ?1='completed' THEN COALESCE(completed_at, ?3) ELSE NULL END,
            outcome_reason=COALESCE(?4, outcome_reason),
            outcome_note=COALESCE(?5, outcome_note)
         WHERE id=?6",
        params![status, started_sql, completed_sql, outcome_reason, outcome_note, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("commitment {id}")));
    }
    let c = get_commitment(conn, id)?;
    // Completing a commitment completes its linked task.
    if status == "completed" {
        if let Some(task_id) = c.task_id {
            conn.execute(
                "UPDATE tasks SET status='completed', completed_at=?1 WHERE id=?2",
                params![ts, task_id],
            )?;
        }
    } else if previous.status == "completed" {
        if let Some(task_id) = c.task_id {
            let other_completed: bool = conn.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM daily_commitments
                   WHERE task_id=?1 AND status='completed' AND id<>?2
                 )",
                params![task_id, id],
                |row| row.get(0),
            )?;
            if !other_completed {
                conn.execute(
                    "UPDATE tasks SET status='committed', completed_at=NULL
                     WHERE id=?1 AND status='completed'",
                    [task_id],
                )?;
            }
        }
    }
    Ok(c)
}

pub fn set_commitment_step_completed(
    conn: &Connection,
    id: i64,
    step_index: usize,
    completed: bool,
) -> AppResult<Commitment> {
    let mut commitment = actionable_commitment(conn, id)?;
    let step = commitment
        .steps
        .get_mut(step_index)
        .ok_or_else(|| AppError::invalid("That action step does not exist."))?;
    step.completed = completed;
    conn.execute(
        "UPDATE daily_commitments SET steps=?1 WHERE id=?2",
        params![serde_json::to_string(&commitment.steps)?, id],
    )?;
    get_commitment(conn, id)
}

pub fn end_day(conn: &Connection, plan_id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE daily_plans SET ended_at=?1 WHERE id=?2 AND ended_at IS NULL",
        params![now(), plan_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(date: String, steps: Vec<String>) -> LockDayInput {
        LockDayInput {
            date,
            commitments: vec![CommitmentInput {
                task_id: None,
                title: "Publish the launch brief".into(),
                done_definition: "The final launch brief is approved and published.".into(),
                estimated_minutes: Some(60),
                priority: "must".into(),
                steps,
            }],
            likely_distraction: "Email".into(),
            countermeasure: "Capture requests and return to the brief.".into(),
            most_important_when: "now".into(),
            interview_answers: serde_json::Value::Null,
        }
    }

    #[test]
    fn action_steps_round_trip_and_can_be_checked() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let date = crate::db::today_local();
        let (_, commitments) = {
            let tx = conn.transaction().unwrap();
            let result = lock_day(
                &tx,
                &input(
                    date,
                    vec!["Review the draft".into(), "Publish the brief".into()],
                ),
            )
            .unwrap();
            tx.commit().unwrap();
            result
        };

        let commitment = &commitments[0];
        assert_eq!(commitment.steps.len(), 2);
        assert_eq!(commitment.steps[0].title, "Review the draft");
        assert!(!commitment.steps[0].completed);

        let updated = set_commitment_step_completed(&conn, commitment.id, 0, true).unwrap();
        assert!(updated.steps[0].completed);
        assert!(!updated.steps[1].completed);
        assert!(set_commitment_step_completed(&conn, commitment.id, 99, true).is_err());
    }

    #[test]
    fn action_step_limits_are_validated_before_locking() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let too_many = (0..=MAX_COMMITMENT_STEPS)
            .map(|index| format!("Step {index}"))
            .collect();
        let tx = conn.transaction().unwrap();
        assert!(lock_day(&tx, &input(crate::db::today_local(), too_many)).is_err());
    }

    #[test]
    fn revising_a_day_preserves_started_rows_and_checked_steps() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let date = crate::db::today_local();
        let mut original = input(
            date.clone(),
            vec!["Review the draft".into(), "Publish the brief".into()],
        );
        original.commitments.push(CommitmentInput {
            task_id: None,
            title: "Clear the finance queue".into(),
            done_definition: "Every approved finance request has a recorded decision.".into(),
            estimated_minutes: Some(30),
            priority: "should".into(),
            steps: vec!["Open the finance queue".into()],
        });
        let (_, original_commitments) = {
            let tx = conn.transaction().unwrap();
            let result = lock_day(&tx, &original).unwrap();
            tx.commit().unwrap();
            result
        };
        let kept_id = original_commitments[0].id;
        let removed_id = original_commitments[1].id;
        set_commitment_step_completed(&conn, kept_id, 0, true).unwrap();
        set_commitment_status(&conn, kept_id, "active", None, None).unwrap();

        let revision = ReviseDayInput {
            date,
            commitments: vec![
                ReviseCommitmentInput {
                    id: Some(kept_id),
                    commitment: CommitmentInput {
                        task_id: None,
                        title: "Publish the revised launch brief".into(),
                        done_definition: "The revised launch brief is approved and published.".into(),
                        estimated_minutes: Some(75),
                        priority: "must".into(),
                        steps: vec!["  Review   the draft ".into(), "Send approval note".into()],
                    },
                },
                ReviseCommitmentInput {
                    id: None,
                    commitment: CommitmentInput {
                        task_id: None,
                        title: "Send the customer recap".into(),
                        done_definition: "The customer receives a concise written recap today.".into(),
                        estimated_minutes: Some(20),
                        priority: "should".into(),
                        steps: vec![],
                    },
                },
            ],
            likely_distraction: "Email".into(),
            countermeasure: "Capture requests and return to the brief.".into(),
            most_important_when: "now".into(),
            interview_answers: serde_json::json!({"revised": true}),
        };

        let (_, revised) = {
            let tx = conn.transaction().unwrap();
            let result = revise_day(&tx, &revision).unwrap();
            tx.commit().unwrap();
            result
        };

        assert_eq!(revised.len(), 2);
        assert_eq!(revised[0].id, kept_id);
        assert_eq!(revised[0].status, "active");
        assert!(revised[0].started_at.is_some());
        assert_eq!(revised[0].title, "Publish the revised launch brief");
        assert_eq!(revised[0].steps[0].title, "Review   the draft");
        assert!(revised[0].steps[0].completed);
        assert!(!revised[0].steps[1].completed);
        assert!(matches!(
            get_commitment(&conn, removed_id),
            Err(AppError::NotFound(_))
        ));
        assert_ne!(revised[1].id, kept_id);
        assert_eq!(revised[1].status, "pending");
    }

    #[test]
    fn revising_a_day_cannot_remove_started_commitments() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let date = crate::db::today_local();
        let (_, commitments) = {
            let tx = conn.transaction().unwrap();
            let result = lock_day(&tx, &input(date.clone(), vec![])).unwrap();
            tx.commit().unwrap();
            result
        };
        set_commitment_status(&conn, commitments[0].id, "active", None, None).unwrap();

        let revision = ReviseDayInput {
            date,
            commitments: vec![ReviseCommitmentInput {
                id: None,
                commitment: CommitmentInput {
                    task_id: None,
                    title: "Use a replacement outcome".into(),
                    done_definition: "The replacement outcome is fully complete today.".into(),
                    estimated_minutes: Some(30),
                    priority: "must".into(),
                    steps: vec![],
                },
            }],
            likely_distraction: "Email".into(),
            countermeasure: "Capture it and return to the outcome.".into(),
            most_important_when: "flexible".into(),
            interview_answers: serde_json::Value::Null,
        };
        let tx = conn.transaction().unwrap();
        let error = revise_day(&tx, &revision).unwrap_err().to_string();
        assert!(error.contains("must stay in today's record"), "{error}");
    }
}
