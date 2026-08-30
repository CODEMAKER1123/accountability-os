//! Daily plans + commitments (spec §5–7, §15).

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use super::models::{Commitment, CommitmentStep, DailyPlan, Task};
use super::{now, tasks};
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RevisionImpact {
    /// Existing commitments whose AI classification context changed.
    pub semantic_commitment_ids: Vec<i64>,
}

impl RevisionImpact {
    pub fn semantics_changed(&self) -> bool {
        !self.semantic_commitment_ids.is_empty()
    }
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

fn quick_start_done_definition(task: &Task) -> String {
    let description = task.description.trim();
    if description.chars().count() >= 10 {
        description.chars().take(2_000).collect()
    } else {
        format!(
            "{} is finished and the result is ready to verify.",
            task.title.trim()
        )
    }
}

struct TaskForTodayInspection {
    task: Task,
    date: String,
    plan: Option<DailyPlan>,
    existing: Option<Commitment>,
}

fn inspect_task_for_today(conn: &Connection, task_id: i64) -> AppResult<TaskForTodayInspection> {
    let task = tasks::get(conn, task_id)?;
    if matches!(task.status.as_str(), "completed" | "cancelled") {
        return Err(AppError::invalid(
            "Completed or cancelled tasks cannot be started.",
        ));
    }

    let date = super::today_local();
    let plan = get_plan_by_date(conn, &date)?;
    let existing = if let Some(plan) = &plan {
        if plan.is_day_off {
            return Err(AppError::invalid(
                "Today is marked off. Reopen the day before starting a task.",
            ));
        }
        if plan.ended_at.is_some() {
            return Err(AppError::invalid(
                "Today's review is complete. Start this task on a new day.",
            ));
        }
        let existing = conn
            .query_row(
                "SELECT * FROM daily_commitments
                 WHERE plan_id=?1 AND task_id=?2
                 ORDER BY id LIMIT 1",
                params![plan.id, task_id],
                commitment_from_row,
            )
            .optional()?;
        if let Some(commitment) = &existing {
            if matches!(
                commitment.status.as_str(),
                "completed" | "deferred" | "dropped" | "cancelled"
            ) {
                return Err(AppError::invalid(
                    "That task is already closed in today's accountability record.",
                ));
            }
        } else {
            let commitment_count: usize = conn.query_row(
                "SELECT COUNT(*) FROM daily_commitments WHERE plan_id=?1",
                [plan.id],
                |row| row.get(0),
            )?;
            if let Some(message) = too_many_commitments_message(commitment_count + 1) {
                return Err(AppError::Invalid(format!(
                    "{message} Edit today's plan before starting another task."
                )));
            }
        }
        existing
    } else {
        None
    };

    Ok(TaskForTodayInspection {
        task,
        date,
        plan,
        existing,
    })
}

/// Validate a task-list quick start without changing the plan. Switch flows
/// use this before ending the current focus session, then repeat the same
/// checks inside their transaction.
pub fn validate_task_for_today(conn: &Connection, task_id: i64) -> AppResult<()> {
    inspect_task_for_today(conn, task_id).map(|_| ())
}

/// Ensure an open backlog task has an actionable commitment in today's plan.
///
/// Starting work from the Tasks page is explicit planning intent. If the user
/// has not planned today yet, create a minimal locked plan containing this one
/// task. If a plan is already open, append the task without disturbing the
/// existing contract. The three-outcome limit and closed/day-off boundaries
/// still apply.
pub fn prepare_task_for_today(
    tx: &rusqlite::Transaction<'_>,
    task_id: i64,
) -> AppResult<(Commitment, Option<i64>)> {
    let inspected = inspect_task_for_today(tx, task_id)?;
    let task = inspected.task;
    let ts = now();
    let (plan_id, newly_locked_plan_id) = match inspected.plan {
        Some(plan) => {
            if plan.locked_at.is_none() {
                tx.execute(
                    "UPDATE daily_plans SET locked_at=?1, most_important_when='now' WHERE id=?2",
                    params![ts, plan.id],
                )?;
                (plan.id, Some(plan.id))
            } else {
                (plan.id, None)
            }
        }
        None => {
            tx.execute(
                "INSERT INTO daily_plans(
                    date, locked_at, likely_distraction, countermeasure,
                    most_important_when, interview_answers, is_day_off, created_at
                 ) VALUES(?1,?2,'','','now',?3,0,?2)",
                params![
                    inspected.date,
                    ts,
                    serde_json::to_string(&serde_json::json!({
                        "source": "task_list_quick_start"
                    }))?
                ],
            )?;
            let id = tx.last_insert_rowid();
            (id, Some(id))
        }
    };

    if let Some(commitment) = inspected.existing {
        tx.execute(
            "UPDATE tasks SET status='committed', completed_at=NULL WHERE id=?1",
            [task_id],
        )?;
        return Ok((commitment, newly_locked_plan_id));
    }

    let rank: i64 = tx.query_row(
        "SELECT COALESCE(MAX(rank), 0) + 1 FROM daily_commitments WHERE plan_id=?1",
        [plan_id],
        |row| row.get(0),
    )?;
    let steps = {
        let mut stmt = tx.prepare(
            "SELECT title, status FROM tasks
             WHERE parent_task_id=?1
             ORDER BY CASE priority WHEN 'must' THEN 0 WHEN 'should' THEN 1 ELSE 2 END,
                      created_at, id
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![task_id, MAX_COMMITMENT_STEPS as i64], |row| {
            Ok(CommitmentStep {
                title: row.get(0)?,
                completed: row.get::<_, String>(1)? == "completed",
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    tx.execute(
        "INSERT INTO daily_commitments(
            plan_id, task_id, title, done_definition, estimated_minutes,
            priority, rank, status, created_at, steps
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9)",
        params![
            plan_id,
            task.id,
            task.title.trim(),
            quick_start_done_definition(&task),
            task.estimated_minutes,
            task.priority,
            rank,
            ts,
            serde_json::to_string(&steps)?
        ],
    )?;
    let commitment = get_commitment(tx, tx.last_insert_rowid())?;
    tx.execute(
        "UPDATE tasks SET status='committed', completed_at=NULL WHERE id=?1",
        [task_id],
    )?;
    Ok((commitment, newly_locked_plan_id))
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

type PreparedRevision = (
    DailyPlan,
    Vec<Commitment>,
    Vec<Vec<String>>,
    RevisionImpact,
);

fn prepare_revision(
    conn: &Connection,
    input: &ReviseDayInput,
) -> AppResult<PreparedRevision> {
    let validation_input = input.validation_input();
    let prepared_steps = validate_day_input(&validation_input)?;
    let plan = get_plan_by_date(conn, &input.date)?
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

    let existing = list_commitments(conn, plan.id)?;
    let existing_by_id: HashMap<i64, &Commitment> =
        existing.iter().map(|commitment| (commitment.id, commitment)).collect();
    let mut submitted_ids = HashSet::new();
    let mut semantic_commitment_ids = Vec::new();
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
        if current.title != item.commitment.title.trim()
            || current.done_definition != item.commitment.done_definition.trim()
        {
            semantic_commitment_ids.push(id);
        }
    }

    if existing.iter().any(|commitment| {
        !submitted_ids.contains(&commitment.id)
            && (commitment.status != "pending" || commitment.started_at.is_some())
    }) {
        return Err(AppError::invalid(
            "Started, completed, or otherwise closed commitments must stay in today's record. You can still edit their details.",
        ));
    }

    Ok((
        plan,
        existing,
        prepared_steps,
        RevisionImpact {
            semantic_commitment_ids,
        },
    ))
}

/// Validate a proposed revision without mutating the plan. Commands use this
/// before flushing the open activity boundary so rejected edits are no-ops.
pub fn validate_revision(conn: &Connection, input: &ReviseDayInput) -> AppResult<RevisionImpact> {
    let (_, _, _, impact) = prepare_revision(conn, input)?;
    Ok(impact)
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
) -> AppResult<(DailyPlan, Vec<Commitment>, RevisionImpact)> {
    let (plan, existing, prepared_steps, impact) = prepare_revision(tx, input)?;
    let existing_by_id: HashMap<i64, &Commitment> =
        existing.iter().map(|commitment| (commitment.id, commitment)).collect();
    let submitted_ids = input
        .commitments
        .iter()
        .filter_map(|item| item.id)
        .collect::<HashSet<_>>();
    let removed = existing
        .iter()
        .filter(|commitment| !submitted_ids.contains(&commitment.id))
        .collect::<Vec<_>>();

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

    Ok((
        get_plan(tx, plan.id)?,
        list_commitments(tx, plan.id)?,
        impact,
    ))
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

pub fn add_commitment_steps(conn: &Connection, id: i64, steps: &[String]) -> AppResult<Commitment> {
    let mut commitment = actionable_commitment(conn, id)?;
    if steps.is_empty() {
        return Err(AppError::invalid("Add at least one action step."));
    }
    let prepared = validated_step_titles(steps)?;
    if commitment.steps.len() + prepared.len() > MAX_COMMITMENT_STEPS {
        return Err(AppError::invalid(format!(
            "A commitment can have at most {MAX_COMMITMENT_STEPS} action steps."
        )));
    }
    let mut seen = commitment
        .steps
        .iter()
        .map(|step| step.title.trim().to_lowercase())
        .collect::<HashSet<_>>();
    for title in prepared {
        if !seen.insert(title.to_lowercase()) {
            return Err(AppError::invalid(format!(
                "This commitment already has an action step named \"{title}\"."
            )));
        }
        commitment.steps.push(CommitmentStep { title, completed: false });
    }
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

    fn backlog_task(conn: &Connection, title: &str) -> Task {
        tasks::create(
            conn,
            &tasks::TaskInput {
                title: title.into(),
                description: format!("{title} has a reviewed, verifiable result."),
                project_id: None,
                parent_task_id: None,
                status: "inbox".into(),
                priority: "must".into(),
                estimated_minutes: Some(45),
                due_date: None,
                tags: vec![],
            },
        )
        .unwrap()
    }

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
    fn action_steps_can_be_added_after_the_day_is_locked() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let (_, commitments) = {
            let tx = conn.transaction().unwrap();
            let result = lock_day(&tx, &input(crate::db::today_local(), vec![])).unwrap();
            tx.commit().unwrap();
            result
        };
        let id = commitments[0].id;

        let updated = add_commitment_steps(
            &conn,
            id,
            &["Open the source file".into(), "Review the draft".into()],
        )
        .unwrap();
        assert_eq!(updated.steps.len(), 2);

        set_commitment_step_completed(&conn, id, 0, true).unwrap();
        let updated = add_commitment_steps(&conn, id, &["Publish the result".into()]).unwrap();
        assert_eq!(updated.steps.len(), 3);
        assert!(updated.steps[0].completed);
        assert!(!updated.steps[2].completed);

        assert!(add_commitment_steps(&conn, id, &[" review THE draft ".into()]).is_err());
        assert_eq!(get_commitment(&conn, id).unwrap().steps.len(), 3);
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
    fn preparing_a_backlog_task_creates_one_locked_plan_and_preserves_steps() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let task = backlog_task(&conn, "Prepare the client proposal");
        let children = tasks::create_steps(
            &conn,
            task.id,
            &["Check the scope".into(), "Send the proposal".into()],
        )
        .unwrap();
        tasks::set_status(&conn, children[0].id, "completed").unwrap();

        let (commitment, locked_plan_id) = {
            let tx = conn.transaction().unwrap();
            let result = prepare_task_for_today(&tx, task.id).unwrap();
            tx.commit().unwrap();
            result
        };
        assert_eq!(locked_plan_id, Some(commitment.plan_id));
        assert_eq!(commitment.task_id, Some(task.id));
        assert_eq!(commitment.title, task.title);
        assert_eq!(commitment.done_definition, task.description);
        assert_eq!(commitment.status, "pending");
        assert_eq!(commitment.steps.len(), 2);
        assert!(commitment.steps[0].completed);
        assert!(!commitment.steps[1].completed);
        let plan = get_plan(&conn, commitment.plan_id).unwrap();
        assert_eq!(plan.date, crate::db::today_local());
        assert!(plan.locked_at.is_some());
        assert_eq!(tasks::get(&conn, task.id).unwrap().status, "committed");

        let (same_commitment, second_lock) = {
            let tx = conn.transaction().unwrap();
            let result = prepare_task_for_today(&tx, task.id).unwrap();
            tx.commit().unwrap();
            result
        };
        assert_eq!(same_commitment.id, commitment.id);
        assert_eq!(second_lock, None);
        assert_eq!(list_commitments(&conn, plan.id).unwrap().len(), 1);
    }

    #[test]
    fn preparing_a_backlog_task_keeps_the_three_outcome_limit() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let tasks_for_today = [
            backlog_task(&conn, "First outcome"),
            backlog_task(&conn, "Second outcome"),
            backlog_task(&conn, "Third outcome"),
        ];
        let extra = backlog_task(&conn, "Fourth outcome");
        let commitments = tasks_for_today
            .iter()
            .map(|task| CommitmentInput {
                task_id: Some(task.id),
                title: task.title.clone(),
                done_definition: task.description.clone(),
                estimated_minutes: task.estimated_minutes,
                priority: task.priority.clone(),
                steps: vec![],
            })
            .collect();
        let (_, locked) = {
            let tx = conn.transaction().unwrap();
            let result = lock_day(
                &tx,
                &LockDayInput {
                    date: crate::db::today_local(),
                    commitments,
                    likely_distraction: String::new(),
                    countermeasure: String::new(),
                    most_important_when: "now".into(),
                    interview_answers: serde_json::Value::Null,
                },
            )
            .unwrap();
            tx.commit().unwrap();
            result
        };
        assert_eq!(locked.len(), 3);

        let tx = conn.transaction().unwrap();
        let error = prepare_task_for_today(&tx, extra.id)
            .unwrap_err()
            .to_string();
        drop(tx);
        assert!(error.contains("3"), "{error}");
        assert!(error.contains("Edit today's plan"), "{error}");
        assert_eq!(tasks::get(&conn, extra.id).unwrap().status, "inbox");

        let tx = conn.transaction().unwrap();
        let existing = prepare_task_for_today(&tx, tasks_for_today[0].id).unwrap().0;
        tx.commit().unwrap();
        assert_eq!(existing.id, locked[0].id);
    }

    #[test]
    fn preparing_a_closed_task_is_rejected_without_creating_a_plan() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let task = backlog_task(&conn, "Already finished");
        tasks::set_status(&conn, task.id, "completed").unwrap();

        let tx = conn.transaction().unwrap();
        let error = prepare_task_for_today(&tx, task.id)
            .unwrap_err()
            .to_string();
        drop(tx);
        assert!(error.contains("cannot be started"), "{error}");
        assert!(get_plan_by_date(&conn, &crate::db::today_local())
            .unwrap()
            .is_none());
    }

    #[test]
    fn preparing_a_task_closed_in_todays_record_is_rejected() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let task = backlog_task(&conn, "Already deferred today");
        let commitment = {
            let tx = conn.transaction().unwrap();
            let result = prepare_task_for_today(&tx, task.id).unwrap().0;
            tx.commit().unwrap();
            result
        };
        set_commitment_status(
            &conn,
            commitment.id,
            "deferred",
            Some("priorities_changed"),
            Some("Waiting until tomorrow"),
        )
        .unwrap();

        let error = validate_task_for_today(&conn, task.id)
            .unwrap_err()
            .to_string();
        assert!(error.contains("already closed"), "{error}");

        let tx = conn.transaction().unwrap();
        assert!(prepare_task_for_today(&tx, task.id).is_err());
        drop(tx);
        assert_eq!(
            list_commitments(&conn, commitment.plan_id).unwrap().len(),
            1
        );
    }

    #[test]
    fn preparing_a_switch_target_rolls_back_with_its_transaction() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        let source = backlog_task(&conn, "Current focus");
        let target = backlog_task(&conn, "Potential replacement");
        let source_commitment = {
            let tx = conn.transaction().unwrap();
            let result = prepare_task_for_today(&tx, source.id).unwrap().0;
            tx.commit().unwrap();
            result
        };

        let rolled_back_commitment_id = {
            let tx = conn.transaction().unwrap();
            let target_commitment = prepare_task_for_today(&tx, target.id).unwrap().0;
            let id = target_commitment.id;
            drop(tx);
            id
        };

        assert!(get_commitment(&conn, rolled_back_commitment_id).is_err());
        assert_eq!(
            list_commitments(&conn, source_commitment.plan_id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(tasks::get(&conn, target.id).unwrap().status, "inbox");
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

        let (_, revised, impact) = {
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
        assert_eq!(impact.semantic_commitment_ids, vec![kept_id]);
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
