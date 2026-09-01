//! Settings, monitoring control, rules management, data lifecycle
//! (spec §41, §50–52), demo mode (spec §47–48), auxiliary windows.

use serde::Serialize;
use tauri::{Manager, State};

use aos_core::events::AppEvent;
use aos_core::types::{Classification, ClassificationSource, ClassifyOutcome};

use crate::db::models::{AppRuleRow, DomainRuleRow};
use crate::db::settings::Settings;
use crate::db::{engine_data, now, plans, rules, sessions, settings as settings_db, today_local};
use crate::engine::emit_event;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

fn validate_settings(settings: &Settings) -> AppResult<()> {
    if [
        settings.work_start_min,
        settings.work_end_min,
        settings.interview_time_min,
        settings.review_time_min,
    ]
    .into_iter()
    .any(|minutes| minutes >= 24 * 60)
    {
        return Err(AppError::invalid("Work and prompt times must be within the day."));
    }
    if settings.work_start_min == settings.work_end_min {
        return Err(AppError::invalid("Workday start and end cannot be the same time."));
    }
    if !(15..=8 * 60).contains(&settings.checkin_cadence_min) {
        return Err(AppError::invalid("Check-in cadence must be between 15 minutes and 8 hours."));
    }
    if settings.distraction_warn_secs < 30
        || settings.distraction_intervene_secs <= settings.distraction_warn_secs
        || settings.distraction_intervene_secs > 24 * 60 * 60
    {
        return Err(AppError::invalid(
            "Intervention threshold must be above the warning threshold (min 30s, max 24h).",
        ));
    }
    if !(30..=24 * 60 * 60).contains(&settings.idle_threshold_secs) {
        return Err(AppError::invalid("Idle threshold must be between 30 seconds and 24 hours."));
    }
    if !(1..=3650).contains(&settings.activity_retention_days) {
        return Err(AppError::invalid("Activity retention must be between 1 day and 10 years."));
    }
    if settings.ai_classify_model.trim().is_empty() || settings.ai_coach_model.trim().is_empty() {
        return Err(AppError::invalid("AI model names cannot be empty."));
    }
    if settings.ai_classify_model.chars().count() > 200
        || settings.ai_coach_model.chars().count() > 200
        || settings.ai_base_url.chars().count() > 2_048
    {
        return Err(AppError::invalid("AI model names or endpoint are too long."));
    }
    crate::ai::validate_base_url(&settings.ai_base_url)?;
    for entries in [
        &settings.excluded_apps,
        &settings.excluded_domains,
        &settings.private_apps,
    ] {
        if entries.len() > 500
            || entries
                .iter()
                .any(|entry| entry.trim().is_empty() || entry.chars().count() > 300)
        {
            return Err(AppError::invalid(
                "Privacy lists may contain up to 500 non-empty entries of 300 characters each.",
            ));
        }
    }
    Ok(())
}

fn normalize_entries(entries: &mut Vec<String>) {
    for entry in entries.iter_mut() {
        *entry = entry.trim().to_lowercase();
    }
    entries.sort();
    entries.dedup();
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppResult<Settings> {
    state.db.with(settings_db::load)
}

#[tauri::command]
pub fn update_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> AppResult<Settings> {
    validate_settings(&settings)?;
    let mut to_store = settings;
    normalize_entries(&mut to_store.excluded_apps);
    normalize_entries(&mut to_store.excluded_domains);
    normalize_entries(&mut to_store.private_apps);
    to_store.ai_base_url = to_store.ai_base_url.trim().trim_end_matches('/').to_string();
    to_store.ai_classify_model = to_store.ai_classify_model.trim().to_string();
    to_store.ai_coach_model = to_store.ai_coach_model.trim().to_string();

    let existing = state.db.with(settings_db::load)?;
    // The running loopback bridge owns these process-lifetime values. Do not
    // let arbitrary renderer input rotate the credential or advertise a port
    // that the already-running server is not actually listening on.
    to_store.extension_token = existing.extension_token;
    to_store.extension_port = existing.extension_port;

    // Any settings edit can change a privacy/classification boundary. Store
    // the pre-change portion under the old policy before installing the new
    // settings (especially when consent is revoked or demo mode changes).
    let history_guard = state.activity_history_boundary.lock();
    crate::engine::flush_open_session(&app);
    state.invalidate_activity_tasks();
    state.db.with_tx(|tx| {
        settings_db::save(tx, &to_store)?;
        tx.execute("UPDATE activity_sessions SET pending_ai=0 WHERE pending_ai=1", [])?;
        Ok(())
    })?;
    {
        let mut engine = state.engine.lock();
        engine.apply_settings(to_store.clone());
        if !to_store.browser_monitoring_enabled {
            engine.last_extension_report = None;
        }
    }
    drop(history_guard);
    emit_event(&app, &AppEvent::ScoresUpdated);
    Ok(to_store)
}

#[tauri::command]
pub fn pause_monitoring(app: tauri::AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let _history_guard = state.activity_history_boundary.lock();
    {
        let mut engine = state.engine.lock();
        engine.monitoring_paused = true;
        engine.tracker.resolve();
    }
    // Close the open session through the normal classification path so the
    // timeline stays truthful — pausing must not erase what is already known.
    crate::engine::flush_open_session(&app);
    emit_event(
        &app,
        &AppEvent::MonitoringStatus {
            state: aos_core::events::MonitoringState::Paused,
        },
    );
    crate::tray::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn resume_monitoring(app: tauri::AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.engine.lock().monitoring_paused = false;
    crate::tray::refresh(&app);
    Ok(())
}

/// First-run consent (spec §40 step 3): monitoring never starts silently.
#[tauri::command]
pub fn grant_monitoring_consent(state: State<'_, AppState>) -> AppResult<Settings> {
    let mut settings = state.db.with(settings_db::load)?;
    settings.monitoring_consent = true;
    state.db.with(|conn| settings_db::save(conn, &settings))?;
    state.engine.lock().apply_settings(settings.clone());
    Ok(settings)
}

#[tauri::command]
pub fn set_demo_mode(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<Settings> {
    let history_guard = state.activity_history_boundary.lock();
    crate::engine::flush_open_session(&app);
    let mut settings = state.db.with(settings_db::load)?;
    settings.demo_mode = enabled;
    state.invalidate_activity_tasks();
    state.db.with_tx(|tx| {
        settings_db::save(tx, &settings)?;
        tx.execute("UPDATE activity_sessions SET pending_ai=0 WHERE pending_ai=1", [])?;
        Ok(())
    })?;
    state.engine.lock().apply_settings(settings.clone());
    drop(history_guard);
    Ok(settings)
}

/// Seed the spec §48 sample day so timeline + scoring are demonstrable.
/// Refuses to touch a day that already has a locked plan.
#[tauri::command]
pub fn seed_demo_data(app: tauri::AppHandle, state: State<'_, AppState>) -> AppResult<String> {
    let _history_guard = state.activity_history_boundary.lock();
    let date = today_local();
    let existing = state.db.with(|conn| plans::get_plan_by_date(conn, &date))?;
    if existing.is_some_and(|p| p.locked_at.is_some()) {
        return Err(AppError::invalid(
            "Today already has a locked plan — demo data would corrupt it.",
        ));
    }
    let (plan, commitments) = state.db.with_tx(|tx| {
        plans::lock_day(
            tx,
            &plans::LockDayInput {
                date: date.clone(),
                commitments: vec![plans::CommitmentInput {
                    task_id: None,
                    title: "Finish Commercial Sales Playbook".into(),
                    done_definition: "Finish the 10-page PA commercial sales rep playbook and send it to the team.".into(),
                    estimated_minutes: Some(90),
                    priority: "must".into(),
                    steps: vec![
                        "Review the current draft".into(),
                        "Finish the missing sections".into(),
                        "Send the playbook to the team".into(),
                    ],
                }],
                likely_distraction: "Email + reactive operations".into(),
                countermeasure: "Capture issues and return to current priority.".into(),
                most_important_when: "now".into(),
                interview_answers: serde_json::json!({"demo": true}),
            },
        )
    })?;
    let commitment_id = commitments.first().map(|c| c.id);

    // 9:00–9:42 Docs Focused / 9:42–9:50 Gmail Supporting / 9:50–10:04 X
    // Distracted / 10:04–10:09 Docs Focused / 10:09–10:21 Idle (spec §48).
    let (day_start, _) = crate::db::local_day_bounds(&date).ok_or_else(|| AppError::invalid("bad date"))?;
    let t = |h: i64, m: i64| day_start + h * 3600 + m * 60;
    type DemoEntry<'a> = (&'a str, &'a str, &'a str, Option<&'a str>, i64, i64, &'a str);
    let entries: &[DemoEntry<'_>] = &[
        ("Chrome", "chrome.exe", "Commercial Sales Playbook - Google Docs", Some("docs.google.com"), t(9, 0), t(9, 42), "focused"),
        ("Chrome", "chrome.exe", "Inbox - Gmail", Some("mail.google.com"), t(9, 42), t(9, 50), "supporting"),
        ("Chrome", "chrome.exe", "Home / X", Some("x.com"), t(9, 50), t(10, 4), "distracted"),
        ("Chrome", "chrome.exe", "Commercial Sales Playbook - Google Docs", Some("docs.google.com"), t(10, 4), t(10, 9), "focused"),
        ("Idle", "", "", None, t(10, 9), t(10, 21), "idle"),
    ];
    state.db.with(|conn| {
        for (application_name, process, title, domain, start, end, class) in entries {
            let draft = aos_core::types::SessionDraft {
                started_at: *start,
                ended_at: *end,
                app_name: (*application_name).into(),
                process_name: (*process).into(),
                window_title: (*title).into(),
                browser_domain: domain.map(String::from),
                browser_title: domain.map(|_| (*title).to_string()),
                is_idle: *class == "idle",
            };
            let outcome = ClassifyOutcome {
                classification: Classification::parse(class).expect("valid demo class"),
                confidence: 1.0,
                source: ClassificationSource::Rule,
                reason: "Demo data".into(),
            };
            sessions::insert(conn, &draft, &outcome, commitment_id, false)?;
        }
        Ok(())
    })?;
    emit_event(&app, &AppEvent::DayLocked { plan_id: plan.id });
    emit_event(&app, &AppEvent::SessionsUpdated);
    emit_event(&app, &AppEvent::ScoresUpdated);
    Ok("Demo day seeded: 1 commitment + 5 activity sessions.".into())
}

// -- Rules ------------------------------------------------------------------

#[derive(Serialize)]
pub struct RulesList {
    pub domain_rules: Vec<DomainRuleRow>,
    pub app_rules: Vec<AppRuleRow>,
}

#[tauri::command]
pub fn list_rules(state: State<'_, AppState>) -> AppResult<RulesList> {
    state.db.with(|conn| {
        Ok(RulesList {
            domain_rules: rules::list_domain_rules(conn)?,
            app_rules: rules::list_app_rules(conn)?,
        })
    })
}

#[tauri::command]
pub fn add_domain_rule(
    state: State<'_, AppState>,
    domain: String,
    classification: String,
    only_in_focus: bool,
) -> AppResult<()> {
    state.db.with(|conn| {
        rules::upsert_domain_rule(conn, &domain, &classification, None, None, only_in_focus).map(|_| ())
    })?;
    state.engine.lock().pipeline_dirty = true;
    Ok(())
}

#[tauri::command]
pub fn add_app_rule(
    state: State<'_, AppState>,
    process_name: String,
    classification: String,
    only_in_focus: bool,
) -> AppResult<()> {
    state
        .db
        .with(|conn| {
            rules::upsert_app_rule(
                conn,
                &process_name,
                &classification,
                None,
                None,
                only_in_focus,
            )
            .map(|_| ())
        })?;
    state.engine.lock().pipeline_dirty = true;
    Ok(())
}

#[tauri::command]
pub fn delete_rule(state: State<'_, AppState>, kind: String, id: i64) -> AppResult<()> {
    state.db.with(|conn| match kind.as_str() {
        "domain" => rules::delete_domain_rule(conn, id),
        "app" => rules::delete_app_rule(conn, id),
        other => Err(AppError::invalid(format!("Invalid rule kind: {other}"))),
    })?;
    state.engine.lock().pipeline_dirty = true;
    Ok(())
}

// -- Data lifecycle (spec §50) ----------------------------------------------

fn deletion_covers_today(
    scope: &str,
    from_date: Option<&str>,
    to_date: Option<&str>,
    today: &str,
) -> AppResult<bool> {
    match scope {
        "today" | "all" => Ok(true),
        "range" => {
            let from = from_date.ok_or_else(|| {
                AppError::invalid("from_date and to_date are required for a range.")
            })?;
            let to = to_date.ok_or_else(|| {
                AppError::invalid("from_date and to_date are required for a range.")
            })?;
            sessions::deletion_range_bounds(from, to)?;
            Ok(from <= today && to >= today)
        }
        other => Err(AppError::invalid(format!("Invalid scope: {other}"))),
    }
}

fn durable_open_interruption(
    conn: &rusqlite::Connection,
    current_pointer: Option<i64>,
) -> AppResult<Option<i64>> {
    if let Some(id) = current_pointer {
        let still_open: bool = conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM interruptions
                WHERE id=?1 AND kind='intervention' AND acknowledged_at IS NULL
             )",
            [id],
            |row| row.get(0),
        )?;
        if still_open {
            return Ok(Some(id));
        }
    }
    Ok(engine_data::open_interruption(conn)?.map(|row| row.id))
}

fn durable_runtime_history(
    conn: &rusqlite::Connection,
    focus_id: Option<i64>,
    break_id: Option<i64>,
) -> AppResult<(bool, bool)> {
    let focus_survived = match focus_id {
        Some(id) => conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM focus_sessions WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?,
        None => false,
    };
    let break_survived = match break_id {
        Some(id) => conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM breaks WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?,
        None => false,
    };
    Ok((focus_survived, break_survived))
}

#[tauri::command]
pub fn delete_activity(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    scope: String,
    from_date: Option<String>,
    to_date: Option<String>,
) -> AppResult<usize> {
    // If the deletion covers "now", the open in-memory draft is part of what
    // the user is deleting — discard it unstored, or the next app switch
    // would resurrect activity they explicitly removed.
    let today = today_local();
    // Validate the request before invalidating async work or mutating the
    // open in-memory session. A rejected deletion must be a true no-op.
    let covers_now = deletion_covers_today(
        &scope,
        from_date.as_deref(),
        to_date.as_deref(),
        &today,
    )?;
    // Keep focus/break creation, its SQLite commit, and runtime publication
    // on one side of the privacy boundary. A start that wins this lock is
    // deleted and reconciled; one that follows creates a genuinely new row.
    let history_guard = state.activity_history_boundary.lock();
    {
        let mut engine = state.engine.lock();
        // Increment the async generation while holding the same engine lock
        // used to produce accountability signals. A signal is therefore
        // either wholly before this boundary or sees the reconciled tracker.
        state.invalidate_activity_tasks_with_engine(&mut engine);
        if covers_now {
            // Drop the deleted draft but retain this privacy boundary so an
            // idle sample cannot backdate new history into the erased range.
            let _ = engine.aggregator.flush_at(crate::db::now());
            engine.current_activity = None;
            engine.recovering_interruption_id = None;
            engine.tracker.resolve();
        }
    }
    let n = state.db.with_tx(|tx| {
        let removed = match scope.as_str() {
            "today" => sessions::delete_range(tx, &today, &today)?,
            "range" => sessions::delete_range(
                tx,
                from_date.as_deref().expect("validated above"),
                to_date.as_deref().expect("validated above"),
            )?,
            "all" => sessions::delete_all(tx)?,
            _ => unreachable!("scope validated above"),
        };
        // A privacy boundary cancels every in-flight classifier. Remaining
        // sessions stay user-correctable rather than waiting forever.
        tx.execute("UPDATE activity_sessions SET pending_ai=0 WHERE pending_ai=1", [])?;
        Ok(removed)
    })?;
    // Snapshot every live history pointer after deletion and verify its exact
    // row in SQLite. The boundary lock prevents focus/break publication from
    // slipping between this verification and reconciliation; interruption
    // producers use the activity generation for the same guarantee.
    let (pointer_after_deletion, focus_after_deletion, break_after_deletion) = {
        let engine = state.engine.lock();
        (
            engine.open_interruption,
            engine.focus_session_id,
            engine.current_break.as_ref().map(|(id, _)| *id),
        )
    };
    let (durable_interruption, focus_survived, break_survived) = state.db.with(|conn| {
        let interruption = durable_open_interruption(conn, pointer_after_deletion)?;
        let (focus_survived, break_survived) =
            durable_runtime_history(conn, focus_after_deletion, break_after_deletion)?;
        Ok((interruption, focus_survived, break_survived))
    })?;
    let (focus_cleared, cleared_commitment_id, break_cleared) = {
        let mut engine = state.engine.lock();
        engine.pipeline_dirty = true;
        engine.classification_cache.clear();
        if engine.open_interruption == pointer_after_deletion {
            engine.open_interruption = durable_interruption;
            if durable_interruption.is_none()
                && (covers_now || pointer_after_deletion.is_some())
            {
                engine.tracker.resolve();
            }
        }
        let mut focus_cleared = false;
        let mut cleared_commitment_id = None;
        if focus_after_deletion.is_some()
            && engine.focus_session_id == focus_after_deletion
            && !focus_survived
        {
            engine.focus_session_id = None;
            cleared_commitment_id = engine.active_commitment.take().map(|item| item.id);
            engine.tracker.resolve();
            focus_cleared = true;
        }
        let mut break_cleared = false;
        let current_break_id = engine.current_break.as_ref().map(|(id, _)| *id);
        if break_after_deletion.is_some()
            && current_break_id == break_after_deletion
            && !break_survived
        {
            engine.current_break = None;
            engine.break_over_pending = false;
            break_cleared = true;
        } else if covers_now {
            // A pending "break over" prompt has no row pointer of its own,
            // but it belongs to the current day's break history.
            engine.break_over_pending = false;
        }
        (focus_cleared, cleared_commitment_id, break_cleared)
    };
    drop(history_guard);
    if focus_cleared {
        if let Some(commitment_id) = cleared_commitment_id {
            emit_event(&app, &AppEvent::FocusEnded { commitment_id });
        }
        emit_event(&app, &AppEvent::CommitmentChanged { commitment_id: None });
    }
    if break_cleared {
        emit_event(&app, &AppEvent::BreakEnded);
    }
    emit_event(&app, &AppEvent::SessionsUpdated);
    emit_event(&app, &AppEvent::ScoresUpdated);
    if focus_cleared || break_cleared {
        crate::tray::refresh(&app);
    }
    Ok(n)
}

/// Export everything to a JSON file at a user-chosen path (spec §50).
#[tauri::command]
pub fn export_data(state: State<'_, AppState>, path: String) -> AppResult<String> {
    let export_path = std::path::PathBuf::from(path.trim());
    if export_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("json"))
    {
        return Err(AppError::invalid("Exports must use a .json filename."));
    }
    let export = state.db.with(|conn| {
        let mut out = serde_json::Map::new();
        let dump = |conn: &rusqlite::Connection, table: &str| -> AppResult<serde_json::Value> {
            let mut stmt = conn.prepare(&format!("SELECT * FROM {table}"))?;
            let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
            let rows = stmt.query_map([], |row| {
                let mut obj = serde_json::Map::new();
                for (i, col) in cols.iter().enumerate() {
                    let v: rusqlite::types::Value = row.get(i)?;
                    let jv = match v {
                        rusqlite::types::Value::Null => serde_json::Value::Null,
                        rusqlite::types::Value::Integer(n) => serde_json::Value::from(n),
                        rusqlite::types::Value::Real(f) => serde_json::Value::from(f),
                        rusqlite::types::Value::Text(s) => serde_json::Value::from(s),
                        rusqlite::types::Value::Blob(_) => serde_json::Value::Null,
                    };
                    obj.insert(col.clone(), jv);
                }
                Ok(serde_json::Value::Object(obj))
            })?;
            Ok(serde_json::Value::Array(rows.collect::<Result<Vec<_>, _>>()?))
        };
        for table in [
            "projects", "tasks", "daily_plans", "daily_commitments", "activity_sessions",
            "activity_corrections", "domain_rules", "application_rules", "focus_sessions",
            "checkins", "checkin_responses", "interruptions", "breaks", "daily_reviews",
            "daily_scores", "ai_insights",
        ] {
            out.insert(table.into(), dump(conn, table)?);
        }
        // Settings minus the extension token (it is a local credential).
        let mut settings = settings_db::load(conn)?;
        settings.extension_token = String::new();
        out.insert("settings".into(), serde_json::to_value(&settings)?);
        out.insert(
            "exported_at".into(),
            serde_json::Value::from(chrono::Utc::now().to_rfc3339()),
        );
        Ok(serde_json::Value::Object(out))
    })?;
    // Never overwrite an existing file from an IPC argument. The save dialog
    // gives the user a fresh path; create_new preserves that trust boundary if
    // a compromised renderer tries to target an existing document.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&export_path)?;
    std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&export)?)?;
    Ok(export_path.to_string_lossy().into_owned())
}

#[derive(Serialize)]
pub struct ExtensionInfo {
    pub port: u16,
    pub token: String,
    pub connected: bool,
    pub last_report_at: Option<i64>,
}

#[tauri::command]
pub fn get_extension_info(state: State<'_, AppState>) -> ExtensionInfo {
    let engine = state.engine.lock();
    ExtensionInfo {
        port: engine.settings.extension_port,
        token: engine.settings.extension_token.clone(),
        connected: engine
            .last_extension_report
            .as_ref()
            .is_some_and(|r| now() - r.at <= 60),
        last_report_at: engine.last_extension_report.as_ref().map(|r| r.at),
    }
}

// -- Auxiliary windows ------------------------------------------------------

/// Toggle the always-on-top mini focus widget (spec §25).
#[tauri::command]
pub fn set_widget_visible(app: tauri::AppHandle, state: State<'_, AppState>, visible: bool) -> AppResult<()> {
    let always_on_top = { state.engine.lock().settings.widget_always_on_top };
    if let Some(w) = app.get_webview_window("widget") {
        w.set_resizable(true)
            .map_err(|e| AppError::Internal(format!("widget window: {e}")))?;
        w.set_min_size(Some(tauri::LogicalSize::new(320.0, 360.0)))
            .map_err(|e| AppError::Internal(format!("widget window: {e}")))?;
        w.set_always_on_top(always_on_top)
            .map_err(|e| AppError::Internal(format!("widget window: {e}")))?;
        if visible {
            let _ = w.show();
        } else {
            // Keep the webview alive when the user closes the widget. Reusing a
            // closed WebviewWindow handle can produce a blank surface on the
            // next show; hiding preserves the loaded React document and its
            // persisted geometry while still removing the widget from view.
            let _ = w.hide();
        }
        return Ok(());
    }
    if !visible {
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        "widget",
        tauri::WebviewUrl::App("index.html#window=widget".into()),
    )
    .title("Focus")
    .inner_size(340.0, 430.0)
    .min_inner_size(320.0, 360.0)
    .resizable(true)
    .decorations(false)
    .always_on_top(always_on_top)
    .skip_taskbar(true)
    .position(40.0, 40.0)
    .build()
    .map_err(|e| AppError::Internal(format!("widget window: {e}")))?;
    Ok(())
}

/// Quick Capture window (spec §4: Ctrl+Shift+Space).
#[tauri::command]
pub fn open_quick_capture(app: tauri::AppHandle) -> AppResult<()> {
    if let Some(w) = app.get_webview_window("capture") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        "capture",
        tauri::WebviewUrl::App("index.html#window=capture".into()),
    )
    .title("Quick Capture")
    .inner_size(560.0, 190.0)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .center()
    .build()
    .map_err(|e| AppError::Internal(format!("capture window: {e}")))?;
    Ok(())
}

#[tauri::command]
pub fn close_window(app: tauri::AppHandle, label: String) -> AppResult<()> {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.close();
    }
    Ok(())
}

#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) -> AppResult<()> {
    crate::engine::show_main(&app);
    Ok(())
}

/// End-of-day data for the tray "End day" item and review trigger.
#[tauri::command]
pub fn trigger_review_now(app: tauri::AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.engine.lock().review_prompted_date = None;
    state.engine.lock().review_delay_until = None;
    emit_event(&app, &AppEvent::ReviewDue);
    crate::engine::show_main(&app);
    Ok(())
}

/// Unblock helper referenced by the popup flows.
#[tauri::command]
pub fn get_active_focus(state: State<'_, AppState>) -> AppResult<Option<crate::db::models::FocusSessionRow>> {
    state.db.with(engine_data::active_focus)
}

#[cfg(test)]
mod tests {
    use super::{deletion_covers_today, durable_open_interruption, durable_runtime_history};
    use crate::db::{engine_data, Db};

    #[test]
    fn malformed_or_reversed_deletion_ranges_are_rejected_before_side_effects() {
        assert!(deletion_covers_today("range", Some("0000"), Some("zzzz"), "2026-08-29")
            .is_err());
        assert!(deletion_covers_today(
            "range",
            Some("2026-08-30"),
            Some("2026-08-01"),
            "2026-08-29",
        )
        .is_err());
    }

    #[test]
    fn validated_deletion_ranges_report_whether_they_cover_today() {
        assert!(deletion_covers_today(
            "range",
            Some("2026-08-01"),
            Some("2026-08-29"),
            "2026-08-29",
        )
        .unwrap());
        assert!(!deletion_covers_today(
            "range",
            Some("2026-08-01"),
            Some("2026-08-28"),
            "2026-08-29",
        )
        .unwrap());
    }

    #[test]
    fn deletion_reconciliation_rejects_a_published_pointer_to_a_deleted_row() {
        let db = Db::open_in_memory().unwrap();
        let (older, newer) = db
            .with(|conn| {
                let context = engine_data::InterruptionContext {
                    app_name: "Browser",
                    process_name: "browser.exe",
                    browser_domain: Some("example.test"),
                    window_title: "Prompt",
                };
                let older = engine_data::create_interruption(
                    conn,
                    "intervention",
                    None,
                    &context,
                    420,
                    None,
                )?;
                let newer = engine_data::create_interruption(
                    conn,
                    "intervention",
                    None,
                    &context,
                    480,
                    None,
                )?;
                conn.execute(
                    "UPDATE interruptions SET started_at=id WHERE id IN (?1, ?2)",
                    rusqlite::params![older, newer],
                )?;
                Ok((older, newer))
            })
            .unwrap();

        assert_eq!(
            db.with(|conn| durable_open_interruption(conn, Some(newer)))
                .unwrap(),
            Some(newer)
        );
        db.with(|conn| {
            conn.execute("DELETE FROM interruptions WHERE id=?1", [newer])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            db.with(|conn| durable_open_interruption(conn, Some(newer)))
                .unwrap(),
            Some(older)
        );
        db.with(|conn| engine_data::respond_interruption(conn, older, "dismissed", None))
            .unwrap();
        assert_eq!(
            db.with(|conn| durable_open_interruption(conn, Some(older)))
                .unwrap(),
            None
        );
    }

    #[test]
    fn deletion_reconciliation_rejects_deleted_focus_and_break_pointers() {
        let db = Db::open_in_memory().unwrap();
        let (focus_id, break_id) = db
            .with(|conn| {
                conn.execute(
                    "INSERT INTO daily_plans(date, created_at) VALUES('2026-08-29', 1)",
                    [],
                )?;
                let plan_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO daily_commitments(plan_id, title, created_at)
                     VALUES(?1, 'Race-safe focus', 1)",
                    [plan_id],
                )?;
                let commitment_id = conn.last_insert_rowid();
                let focus_id = engine_data::start_focus(conn, commitment_id)?.id;
                let break_id = engine_data::start_break(conn, 300)?.id;
                Ok((focus_id, break_id))
            })
            .unwrap();

        assert_eq!(
            db.with(|conn| durable_runtime_history(conn, Some(focus_id), Some(break_id)))
                .unwrap(),
            (true, true)
        );
        db.with(|conn| {
            conn.execute("DELETE FROM focus_sessions WHERE id=?1", [focus_id])?;
            conn.execute("DELETE FROM breaks WHERE id=?1", [break_id])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            db.with(|conn| durable_runtime_history(conn, Some(focus_id), Some(break_id)))
                .unwrap(),
            (false, false)
        );
    }
}
