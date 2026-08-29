//! Activity timeline, corrections, search (spec §4 Activity, §42–44).

use serde::{Deserialize, Serialize};
use tauri::State;

use aos_core::events::MonitoringState;

use crate::db::models::ActivitySessionRow;
use crate::db::{local_day_bounds, sessions, today_local};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, CurrentActivity};

#[tauri::command]
pub fn get_activity_for_date(
    state: State<'_, AppState>,
    date: Option<String>,
) -> AppResult<Vec<ActivitySessionRow>> {
    let date = date.unwrap_or_else(today_local);
    local_day_bounds(&date)
        .ok_or_else(|| AppError::invalid("Activity date must use YYYY-MM-DD."))?;
    state.db.with(|conn| sessions::list_for_date(conn, &date))
}

#[tauri::command]
pub fn search_activity(state: State<'_, AppState>, query: String) -> AppResult<Vec<ActivitySessionRow>> {
    if query.trim().len() < 2 {
        return Ok(vec![]);
    }
    if query.chars().count() > 500 {
        return Err(AppError::invalid("Activity searches must be 500 characters or fewer."));
    }
    state.db.with(|conn| sessions::search(conn, &query, 200))
}

#[derive(Deserialize)]
pub struct RuleRequest {
    /// "domain" | "app"
    pub kind: String,
    /// Scope the rule to the project of the current commitment.
    #[serde(default)]
    pub project_scoped: bool,
    #[serde(default)]
    pub only_in_focus: bool,
}

#[derive(Deserialize)]
pub struct CorrectionInput {
    pub session_id: i64,
    /// focused | supporting | neutral | distracted
    pub new_classification: String,
    pub reason: Option<String>,
    /// Optionally also create a standing rule (spec §42 "Always classify
    /// this domain as…").
    pub create_rule: Option<RuleRequest>,
}

#[tauri::command]
pub fn correct_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: CorrectionInput,
) -> AppResult<ActivitySessionRow> {
    let valid = ["focused", "supporting", "neutral", "distracted"];
    if !valid.contains(&input.new_classification.as_str()) {
        return Err(AppError::invalid("Pick focused, supporting, neutral or distracted."));
    }
    if let Some(rule) = &input.create_rule {
        if !matches!(rule.kind.as_str(), "domain" | "app") {
            return Err(AppError::invalid(format!("Invalid rule kind: {}", rule.kind)));
        }
    }
    // Correction, optional standing rule, cache invalidation, and stored-score
    // refresh are one unit. A bad rule request must not leave a half-applied
    // correction behind.
    let updated = state.db.with_tx(|tx| {
        let session = crate::db::sessions::get(tx, input.session_id)?;
        let commitment_id = session.related_commitment_id;
        let project_id = match commitment_id {
            Some(cid) => match crate::db::plans::get_commitment(tx, cid)?.task_id {
                Some(task_id) => crate::db::tasks::get(tx, task_id)?.project_id,
                None => None,
            },
            None => None,
        };
        // Validate the complete rule request before applying the correction,
        // while keeping every write in this transaction.
        if let Some(rule) = &input.create_rule {
            match rule.kind.as_str() {
                "domain" => {
                    let Some(domain) = session.browser_domain.as_deref() else {
                        return Err(AppError::invalid(
                            "This session has no browser domain to build a rule from.",
                        ));
                    };
                    crate::db::rules::normalize_valid_domain(domain)?;
                }
                "app" => {
                    if session.is_idle
                        || session.process_name.trim().is_empty()
                        || session.process_name == aos_core::types::PRIVATE_PROCESS_SENTINEL
                    {
                        return Err(AppError::invalid(
                            "This session has no application to build a rule from.",
                        ));
                    }
                }
                _ => unreachable!("rule kind validated above"),
            }
            if rule.project_scoped && project_id.is_none() {
                return Err(AppError::invalid(
                    "This session's commitment has no project, so a project-scoped rule isn't possible. \
                     Link the commitment's task to a project, or create the rule without project scope.",
                ));
            }
        }

        let updated = sessions::apply_correction(
            tx,
            &sessions::CorrectionRecord {
                session_id: input.session_id,
                new_classification: input.new_classification.clone(),
                reason: input.reason.clone(),
                commitment_id,
                project_id,
            },
        )?;
        if let Some(rule) = &input.create_rule {
            let scope_project = if rule.project_scoped { project_id } else { None };
            match rule.kind.as_str() {
                "domain" => {
                let Some(domain) = updated.browser_domain.as_deref() else {
                    return Err(AppError::invalid("This session has no browser domain to build a rule from."));
                };
                crate::db::rules::upsert_domain_rule(
                    tx,
                    domain,
                    &input.new_classification,
                    scope_project,
                    None,
                    rule.only_in_focus,
                )
                .map(|_| ())
                }
                "app" => crate::db::rules::upsert_app_rule(
                    tx,
                    &updated.process_name,
                    &input.new_classification,
                    scope_project,
                    None,
                    rule.only_in_focus,
                )
                .map(|_| ()),
                _ => unreachable!("rule kind validated before transaction"),
            }?;
        }
        crate::db::scores::refresh_stored_score(tx, &updated.local_date)?;
        Ok(updated)
    })?;

    // Corrections must teach the classifier (spec §42).
    state.engine.lock().pipeline_dirty = true;
    crate::engine::emit_event(&app, &aos_core::events::AppEvent::SessionsUpdated);
    crate::engine::emit_event(&app, &aos_core::events::AppEvent::ScoresUpdated);
    Ok(updated)
}

#[derive(Serialize)]
pub struct MonitoringStatus {
    pub state: MonitoringState,
    pub message: Option<String>,
    pub extension_connected: bool,
    pub current: Option<CurrentActivity>,
    pub distracted_secs: i64,
    pub warned: bool,
}

#[tauri::command]
pub fn get_monitoring_status(state: State<'_, AppState>) -> MonitoringStatus {
    let engine = state.engine.lock();
    let extension_connected = engine
        .last_extension_report
        .as_ref()
        .is_some_and(|r| crate::db::now() - r.at <= 60);
    MonitoringStatus {
        state: engine.monitoring_state,
        message: engine.monitoring_message.clone(),
        extension_connected,
        current: engine.current_activity.clone(),
        distracted_secs: engine.tracker.current_distracted_secs(),
        warned: engine.tracker.is_warned(),
    }
}
