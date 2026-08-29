//! Task + project CRUD (spec §4 Tasks).

use tauri::State;

use crate::db::models::{Project, Task};
use crate::db::tasks::{self, TaskInput};
use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub fn list_tasks(
    state: State<'_, AppState>,
    status: Option<String>,
    search: Option<String>,
) -> AppResult<Vec<Task>> {
    state
        .db
        .with(|conn| tasks::list(conn, status.as_deref(), search.as_deref()))
}

#[tauri::command]
pub fn create_task(state: State<'_, AppState>, input: TaskInput) -> AppResult<Task> {
    state.db.with(|conn| tasks::create(conn, &input))
}

#[tauri::command]
pub fn create_task_steps(
    state: State<'_, AppState>,
    id: i64,
    steps: Vec<String>,
) -> AppResult<Vec<Task>> {
    state.db.with_tx(|tx| tasks::create_steps(tx, id, &steps))
}

#[tauri::command]
pub fn update_task(state: State<'_, AppState>, id: i64, input: TaskInput) -> AppResult<Task> {
    state.db.with(|conn| tasks::update(conn, id, &input))
}

#[tauri::command]
pub fn set_task_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
    status: String,
) -> AppResult<Task> {
    let task = state.db.with(|conn| tasks::set_status(conn, id, &status))?;
    if status == "completed" {
        crate::engine::emit_event(&app, &aos_core::events::AppEvent::TaskCompleted { task_id: id });
    }
    Ok(task)
}

#[tauri::command]
pub fn delete_task(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    state.db.with(|conn| tasks::delete(conn, id))
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> AppResult<Vec<Project>> {
    state.db.with(tasks::list_projects)
}

#[tauri::command]
pub fn create_project(
    state: State<'_, AppState>,
    name: String,
    color: Option<String>,
) -> AppResult<Project> {
    state
        .db
        .with(|conn| tasks::create_project(conn, &name, color.as_deref()))
}

#[tauri::command]
pub fn archive_project(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    state.db.with(|conn| tasks::archive_project(conn, id))
}
