mod ai;
mod commands;
mod db;
mod engine;
mod error;
mod monitor;
mod server;
mod state;
mod tray;

use parking_lot::Mutex;
use tauri::Manager;

use state::{AppState, EngineState};

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            engine::show_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        let _ = commands::open_quick_capture(app.clone());
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();

            // Database in the per-user app data directory (spec §3: local-first).
            let data_dir = handle.path().app_data_dir()?;
            let db_path = data_dir.join("accountability.sqlite3");
            match db::recovery::recover_codex_virtualized_database(&db_path) {
                Ok(Some(report)) => log::info!(
                    target: "recovery",
                    "restored database from {}; preserved {} newer activity sessions; rollback snapshot removed",
                    report.source.display(),
                    report.imported_activity_sessions
                ),
                Ok(None) => {}
                Err(error) => log::error!(
                    target: "recovery",
                    "legacy database recovery did not complete: {error}"
                ),
            }
            let db = db::Db::open(&db_path)
                .map_err(|e| format!("failed to open database: {e}"))?;
            let settings = db
                .with(db::settings::load)
                .map_err(|e| format!("failed to load settings: {e}"))?;
            let start_minimized = settings.start_minimized && settings.onboarding_completed;
            let widget_enabled = settings.widget_enabled && settings.onboarding_completed;

            app.manage(AppState {
                db,
                engine: Mutex::new(EngineState::new(settings)),
                activity_history_boundary: Mutex::new(()),
                activity_generation: std::sync::atomic::AtomicU64::new(0),
                ai_key: Mutex::new(None),
                http: reqwest::Client::new(),
            });

            tray::setup(&handle)?;
            engine::restore(&handle);
            engine::spawn(handle.clone());
            server::spawn(handle.clone());
            tray::refresh(&handle);

            // Quick Capture global shortcut (spec §4).
            {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                if let Err(e) = handle.global_shortcut().register("CmdOrCtrl+Shift+Space") {
                    log::warn!(target: "app", "global shortcut unavailable: {e}");
                }
            }

            if start_minimized {
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            if widget_enabled {
                let _ = commands::set_widget_visible(handle.clone(), handle.state(), true);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window hides it; monitoring continues from the
            // tray (spec §26, §56.6).
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            // tasks
            commands::list_tasks,
            commands::create_task,
            commands::create_task_steps,
            commands::update_task,
            commands::set_task_status,
            commands::delete_task,
            commands::list_projects,
            commands::create_project,
            commands::archive_project,
            // plan / interview
            commands::get_today_plan,
            commands::get_plan_for_date,
            commands::lock_day,
            commands::revise_day,
            commands::mark_day_off,
            commands::snooze_interview,
            commands::commitment_limit_check,
            commands::set_commitment_step_completed,
            commands::add_commitment_steps,
            // focus / commitments
            commands::start_commitment,
            commands::start_task,
            commands::pause_focus,
            commands::complete_commitment,
            commands::block_commitment,
            commands::switch_commitment,
            // activity
            commands::get_activity_for_date,
            commands::search_activity,
            commands::correct_session,
            commands::get_monitoring_status,
            commands::get_timeline,
            // prompts / breaks
            commands::get_pending_prompt,
            commands::respond_intervention,
            commands::respond_checkin,
            commands::start_break,
            commands::end_break_now,
            commands::acknowledge_break_over,
            commands::get_commitment_title,
            // scores / review / analytics
            commands::get_today_snapshot,
            commands::get_day_score,
            commands::get_scorecard,
            commands::get_review_data,
            commands::submit_review,
            commands::delay_review,
            commands::get_patterns,
            commands::get_insights,
            // settings / data / windows
            commands::get_settings,
            commands::update_settings,
            commands::pause_monitoring,
            commands::resume_monitoring,
            commands::grant_monitoring_consent,
            commands::set_demo_mode,
            commands::seed_demo_data,
            commands::list_rules,
            commands::add_domain_rule,
            commands::add_app_rule,
            commands::delete_rule,
            commands::delete_activity,
            commands::export_data,
            commands::get_extension_info,
            commands::set_widget_visible,
            commands::open_quick_capture,
            commands::close_window,
            commands::show_main_window,
            commands::trigger_review_now,
            commands::get_active_focus,
            // ai
            commands::set_ai_key,
            commands::has_ai_key,
            commands::test_ai_connection,
            commands::break_down_goal,
            commands::get_morning_coach,
            commands::generate_daily_ai_review,
            commands::generate_ai_insights,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Accountability OS");
}
