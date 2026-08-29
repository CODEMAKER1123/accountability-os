use rusqlite::{params, Connection, Row};
use serde::Deserialize;

use super::models::{Project, Task};
use super::now;
use crate::error::{AppError, AppResult};

fn task_from_row(row: &Row) -> rusqlite::Result<Task> {
    let tags_json: String = row.get("tags")?;
    Ok(Task {
        id: row.get("id")?,
        title: row.get("title")?,
        description: row.get("description")?,
        project_id: row.get("project_id")?,
        parent_task_id: row.get("parent_task_id")?,
        status: row.get("status")?,
        priority: row.get("priority")?,
        estimated_minutes: row.get("estimated_minutes")?,
        due_date: row.get("due_date")?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        created_at: row.get("created_at")?,
        completed_at: row.get("completed_at")?,
    })
}

const VALID_STATUSES: &[&str] = &[
    "inbox", "planned", "committed", "active", "completed", "deferred", "cancelled",
];
const VALID_PRIORITIES: &[&str] = &["must", "should", "could"];
const MAX_TASK_STEPS: usize = 12;

#[derive(Debug, Clone, Deserialize)]
pub struct TaskInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub project_id: Option<i64>,
    pub parent_task_id: Option<i64>,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    pub estimated_minutes: Option<i64>,
    pub due_date: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_status() -> String {
    "inbox".into()
}
fn default_priority() -> String {
    "should".into()
}

fn validate(input: &TaskInput) -> AppResult<()> {
    if input.title.trim().is_empty() {
        return Err(AppError::invalid("Task title is required"));
    }
    if input.title.trim().chars().count() > 300 {
        return Err(AppError::invalid("Task titles must be 300 characters or fewer."));
    }
    if input.description.chars().count() > 20_000 {
        return Err(AppError::invalid("Task descriptions must be 20,000 characters or fewer."));
    }
    if !VALID_STATUSES.contains(&input.status.as_str()) {
        return Err(AppError::invalid(format!("Invalid status: {}", input.status)));
    }
    if !VALID_PRIORITIES.contains(&input.priority.as_str()) {
        return Err(AppError::invalid(format!("Invalid priority: {}", input.priority)));
    }
    if input.estimated_minutes.is_some_and(|minutes| !(1..=24 * 60).contains(&minutes)) {
        return Err(AppError::invalid("Task estimates must be between 1 minute and 24 hours."));
    }
    if let Some(due_date) = input.due_date.as_deref() {
        chrono::NaiveDate::parse_from_str(due_date, "%Y-%m-%d")
            .map_err(|_| AppError::invalid("Due date must use YYYY-MM-DD."))?;
    }
    if input.tags.len() > 100
        || input
            .tags
            .iter()
            .any(|tag| tag.trim().is_empty() || tag.chars().count() > 100)
    {
        return Err(AppError::invalid(
            "Use at most 100 non-empty tags of 100 characters each.",
        ));
    }
    Ok(())
}

pub fn create(conn: &Connection, input: &TaskInput) -> AppResult<Task> {
    validate(input)?;
    if let Some(parent_id) = input.parent_task_id {
        get(conn, parent_id)?;
    }
    conn.execute(
        "INSERT INTO tasks(title, description, project_id, parent_task_id, status, priority,
                           estimated_minutes, due_date, tags, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            input.title.trim(),
            input.description,
            input.project_id,
            input.parent_task_id,
            input.status,
            input.priority,
            input.estimated_minutes,
            input.due_date,
            serde_json::to_string(&input.tags)?,
            now(),
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> AppResult<Task> {
    conn.query_row("SELECT * FROM tasks WHERE id = ?1", [id], task_from_row)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound(format!("task {id}")),
            other => other.into(),
        })
}

pub fn update(conn: &Connection, id: i64, input: &TaskInput) -> AppResult<Task> {
    validate(input)?;
    get(conn, id)?;
    validate_parent(conn, id, input.parent_task_id)?;
    let completed_at: Option<i64> = if input.status == "completed" {
        Some(get(conn, id)?.completed_at.unwrap_or_else(now))
    } else {
        None
    };
    let n = conn.execute(
        "UPDATE tasks SET title=?1, description=?2, project_id=?3, parent_task_id=?4, status=?5,
                          priority=?6, estimated_minutes=?7, due_date=?8, tags=?9, completed_at=?10
         WHERE id=?11",
        params![
            input.title.trim(),
            input.description,
            input.project_id,
            input.parent_task_id,
            input.status,
            input.priority,
            input.estimated_minutes,
            input.due_date,
            serde_json::to_string(&input.tags)?,
            completed_at,
            id,
        ],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("task {id}")));
    }
    get(conn, id)
}

fn validate_parent(conn: &Connection, id: i64, parent_id: Option<i64>) -> AppResult<()> {
    let Some(parent_id) = parent_id else {
        return Ok(());
    };
    if parent_id == id {
        return Err(AppError::invalid("A task cannot be its own parent."));
    }
    get(conn, parent_id)?;
    let is_descendant: bool = conn.query_row(
        "WITH RECURSIVE descendants(id) AS (
           SELECT id FROM tasks WHERE parent_task_id=?1
           UNION
           SELECT task.id FROM tasks task
           JOIN descendants parent ON task.parent_task_id=parent.id
         )
         SELECT EXISTS(SELECT 1 FROM descendants WHERE id=?2)",
        params![id, parent_id],
        |row| row.get(0),
    )?;
    if is_descendant {
        return Err(AppError::invalid("A task parent cannot be one of its descendants."));
    }
    Ok(())
}

pub fn set_status(conn: &Connection, id: i64, status: &str) -> AppResult<Task> {
    if !VALID_STATUSES.contains(&status) {
        return Err(AppError::invalid(format!("Invalid status: {status}")));
    }
    let completed_at: Option<i64> = (status == "completed").then(now);
    let n = conn.execute(
        "UPDATE tasks SET status=?1, completed_at=?2 WHERE id=?3",
        params![status, completed_at, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("task {id}")));
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
    Ok(())
}

/// Add checklist-like child tasks beneath a parent task. The caller wraps
/// this in a transaction so either every requested step is created or none
/// are. Child tasks inherit planning context but not the outcome's estimate.
pub fn create_steps(conn: &Connection, parent_id: i64, steps: &[String]) -> AppResult<Vec<Task>> {
    let parent = get(conn, parent_id)?;
    if matches!(parent.status.as_str(), "completed" | "cancelled") {
        return Err(AppError::invalid(
            "Completed or cancelled tasks cannot be broken into new steps.",
        ));
    }
    if steps.is_empty() {
        return Err(AppError::invalid("Add at least one action step."));
    }

    let existing: Vec<String> = {
        let mut stmt = conn.prepare("SELECT title FROM tasks WHERE parent_task_id=?1")?;
        let rows = stmt.query_map([parent_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    if existing.len() + steps.len() > MAX_TASK_STEPS {
        return Err(AppError::invalid(format!(
            "A task can have at most {MAX_TASK_STEPS} direct action steps."
        )));
    }

    let mut seen = existing
        .iter()
        .map(|title| title.trim().to_lowercase())
        .collect::<std::collections::HashSet<_>>();
    let mut prepared = Vec::with_capacity(steps.len());
    for step in steps {
        let title = step.trim();
        if title.is_empty() {
            return Err(AppError::invalid("Action steps cannot be empty."));
        }
        if title.chars().count() > 300 {
            return Err(AppError::invalid(
                "Action steps must be 300 characters or fewer.",
            ));
        }
        if !seen.insert(title.to_lowercase()) {
            return Err(AppError::invalid(format!(
                "This task already has an action step named \"{title}\"."
            )));
        }
        prepared.push(title.to_string());
    }

    prepared
        .into_iter()
        .map(|title| {
            create(
                conn,
                &TaskInput {
                    title,
                    description: String::new(),
                    project_id: parent.project_id,
                    parent_task_id: Some(parent_id),
                    status: "planned".into(),
                    priority: parent.priority.clone(),
                    estimated_minutes: None,
                    due_date: parent.due_date.clone(),
                    tags: vec![],
                },
            )
        })
        .collect()
}

pub fn list(conn: &Connection, status: Option<&str>, search: Option<&str>) -> AppResult<Vec<Task>> {
    let mut sql = String::from("SELECT task.* FROM tasks task WHERE 1=1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![];
    if let Some(s) = status {
        sql.push_str(" AND task.status = ?");
        args.push(Box::new(s.to_string()));
    } else {
        // Keep checked steps visible while their parent outcome remains open.
        sql.push_str(
            " AND (task.status NOT IN ('completed','cancelled')
                    OR EXISTS(
                      SELECT 1 FROM tasks parent
                      WHERE parent.id=task.parent_task_id
                        AND parent.status NOT IN ('completed','cancelled')
                    ))",
        );
    }
    if let Some(q) = search {
        if !q.trim().is_empty() {
            sql.push_str(" AND (task.title LIKE ? OR task.description LIKE ?)");
            let like = format!("%{}%", q.trim());
            args.push(Box::new(like.clone()));
            args.push(Box::new(like));
        }
    }
    sql.push_str(" ORDER BY CASE task.priority WHEN 'must' THEN 0 WHEN 'should' THEN 1 ELSE 2 END, task.created_at DESC, task.id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args), task_from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

// -- Projects ---------------------------------------------------------------

pub fn create_project(conn: &Connection, name: &str, color: Option<&str>) -> AppResult<Project> {
    if name.trim().is_empty() {
        return Err(AppError::invalid("Project name is required"));
    }
    if name.trim().chars().count() > 200 {
        return Err(AppError::invalid("Project names must be 200 characters or fewer."));
    }
    if color.is_some_and(|value| {
        let value = value.trim();
        value.len() != 7
            || !value.starts_with('#')
            || !value[1..].chars().all(|character| character.is_ascii_hexdigit())
    }) {
        return Err(AppError::invalid("Project colors must use #RRGGBB."));
    }
    conn.execute(
        "INSERT INTO projects(name, color, created_at) VALUES(?1,?2,?3)",
        params![name.trim(), color.map(str::trim), now()],
    )?;
    let id = conn.last_insert_rowid();
    get_project(conn, id)
}

pub fn get_project(conn: &Connection, id: i64) -> AppResult<Project> {
    conn.query_row("SELECT * FROM projects WHERE id=?1", [id], |r| {
        Ok(Project {
            id: r.get("id")?,
            name: r.get("name")?,
            color: r.get("color")?,
            archived: r.get::<_, i64>("archived")? != 0,
            created_at: r.get("created_at")?,
        })
    })
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound(format!("project {id}")),
        other => other.into(),
    })
}

pub fn list_projects(conn: &Connection) -> AppResult<Vec<Project>> {
    let mut stmt = conn.prepare("SELECT * FROM projects WHERE archived=0 ORDER BY name")?;
    let rows = stmt.query_map([], |r| {
        Ok(Project {
            id: r.get("id")?,
            name: r.get("name")?,
            color: r.get("color")?,
            archived: r.get::<_, i64>("archived")? != 0,
            created_at: r.get("created_at")?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn archive_project(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("UPDATE projects SET archived=1 WHERE id=?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_input(title: &str) -> TaskInput {
        TaskInput {
            title: title.into(),
            description: String::new(),
            project_id: None,
            parent_task_id: None,
            status: "inbox".into(),
            priority: "must".into(),
            estimated_minutes: Some(90),
            due_date: Some("2026-09-01".into()),
            tags: vec!["launch".into()],
        }
    }

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::apply(&conn).unwrap();
        conn
    }

    #[test]
    fn task_steps_are_children_and_stay_visible_after_completion() {
        let conn = connection();
        let parent = create(&conn, &task_input("Publish launch page")).unwrap();
        let steps = create_steps(
            &conn,
            parent.id,
            &["Write the copy".into(), "Publish the page".into()],
        )
        .unwrap();

        assert_eq!(steps.len(), 2);
        assert!(steps.iter().all(|step| step.parent_task_id == Some(parent.id)));
        assert!(steps.iter().all(|step| step.project_id == parent.project_id));
        assert!(steps.iter().all(|step| step.due_date == parent.due_date));
        assert!(steps.iter().all(|step| step.estimated_minutes.is_none()));

        set_status(&conn, steps[0].id, "completed").unwrap();
        let open = list(&conn, None, None).unwrap();
        assert!(open.iter().any(|task| task.id == steps[0].id));

        set_status(&conn, parent.id, "completed").unwrap();
        let open = list(&conn, None, None).unwrap();
        assert!(!open.iter().any(|task| task.id == parent.id));
        assert!(!open.iter().any(|task| task.id == steps[0].id));
    }

    #[test]
    fn task_steps_reject_duplicates_before_writing() {
        let conn = connection();
        let parent = create(&conn, &task_input("Prepare the review")).unwrap();
        let error = create_steps(
            &conn,
            parent.id,
            &["Gather notes".into(), " gather NOTES ".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("already has an action step"), "{error}");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE parent_task_id=?1",
                [parent.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
