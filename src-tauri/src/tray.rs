//! System tray (spec §26): the app keeps monitoring with the main window
//! closed; the tray is the always-available control surface.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

use crate::state::AppState;

pub struct TrayHandles {
    pub current: MenuItem<tauri::Wry>,
    pub pause: MenuItem<tauri::Wry>,
}

pub fn setup(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Accountability OS", true, None::<&str>)?;
    let current = MenuItem::with_id(app, "current", "No active commitment", false, None::<&str>)?;
    let start_focus = MenuItem::with_id(app, "start_focus", "Start focus…", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause monitoring", true, None::<&str>)?;
    let take_break = MenuItem::with_id(app, "break", "Take a break (10 min)", true, None::<&str>)?;
    let quick = MenuItem::with_id(app, "quick", "Quick task\tCtrl+Shift+Space", true, None::<&str>)?;
    let end_day = MenuItem::with_id(app, "end_day", "End day", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);

    let menu = Menu::with_items(
        app,
        &[
            &open,
            &current,
            &sep()?,
            &start_focus,
            &pause,
            &take_break,
            &quick,
            &sep()?,
            &end_day,
            &quit,
        ],
    )?;

    app.manage(TrayHandles {
        current: current.clone(),
        pause: pause.clone(),
    });

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundled window icon");
    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Accountability OS")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu_event(app, event.id.as_ref()))
        .build(app)?;
    Ok(())
}

fn on_menu_event(app: &tauri::AppHandle, id: &str) {
    match id {
        "open" | "start_focus" => {
            crate::engine::show_main(app);
        }
        "pause" => {
            let state = app.state::<AppState>();
            let paused = { state.engine.lock().monitoring_paused };
            let result = if paused {
                crate::commands::resume_monitoring(app.clone(), app.state())
            } else {
                crate::commands::pause_monitoring(app.clone(), app.state())
            };
            if result.is_ok() {
                refresh(app);
            }
        }
        "break" => {
            let _ = crate::commands::start_break(app.clone(), app.state(), 10);
        }
        "quick" => {
            let _ = crate::commands::open_quick_capture(app.clone());
        }
        "end_day" => {
            let _ = crate::commands::trigger_review_now(app.clone(), app.state());
        }
        "quit" => {
            // Store the open activity session (classified normally) before
            // exiting; a spawned AI call that can't finish is cleaned up as
            // stale pending_ai on the next start.
            crate::engine::flush_open_session(app);
            app.exit(0);
        }
        _ => {}
    }
}

/// Reflect live state in the tray (called after commitment/monitoring
/// changes — spec §26 "make monitoring state obvious").
pub fn refresh(app: &tauri::AppHandle) {
    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    let state = app.state::<AppState>();
    let engine = state.engine.lock();
    let current_text = match &engine.active_commitment {
        Some(c) => format!("▶ {}", truncate(&c.title, 40)),
        None => "No active commitment".into(),
    };
    let pause_text = if engine.monitoring_paused {
        "Resume monitoring"
    } else {
        "Pause monitoring"
    };
    drop(engine);
    let _ = handles.current.set_text(current_text);
    let _ = handles.pause.set_text(pause_text);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max - 1).collect::<String>() + "…"
    }
}
