//! The accountability engine (spec §34): the background loop that ties the
//! probe, aggregator, classifier, distraction tracker, check-ins, breaks,
//! interview and review triggers together. Runs on its own thread; the UI
//! only renders what this emits.

use tauri::{Emitter, Manager};

use aos_core::accountability::{in_work_hours, DistractionSignal, DistractionTracker};
use aos_core::classify::{cache_key, ClassificationPipeline, CorrectionMatcher, PipelineResult, RulesEngine};
use aos_core::events::{AppEvent, MonitoringState};
use aos_core::types::{
    ActivityContext, ActivitySample, Classification, ClassificationSource, ClassifyOutcome,
    SessionDraft, PLANNED_BREAK_REASON,
};

use crate::db::{self, engine_data, local_minutes_now, plans, rules, sessions, today_local};
use crate::monitor::{demo::DemoProbe, is_browser_process, os_probe, ActivityProbe, ProbeReading};
use crate::state::{ActiveCommitment, AppState, CurrentActivity};

pub const POLL_INTERVAL_SECS: u64 = 3;

/// AI answers depend on the commitment wording as well as its row ID. Keep a
/// stable semantic version in the key so a revised outcome cannot reuse an
/// answer prompted with its previous title or definition of done.
fn activity_cache_key(
    commitment: Option<&ActiveCommitment>,
    process_name: &str,
    browser_domain: Option<&str>,
    window_title: &str,
) -> String {
    let base = cache_key(
        commitment.map(|item| item.id),
        process_name,
        browser_domain,
        window_title,
    );
    let Some(commitment) = commitment else {
        return base;
    };

    // FNV-1a is deliberately simple and stable across app versions. This is
    // a cache namespace, not a security boundary; the full private wording is
    // never stored in the key.
    let mut hash = 0xcbf29ce484222325_u64;
    for text in [&commitment.title, &commitment.done_definition] {
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        for byte in normalized.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{base}|s{hash:016x}")
}

pub fn emit_event(app: &tauri::AppHandle, event: &AppEvent) {
    if let Err(e) = app.emit("app-event", event) {
        log::warn!(target: "engine", "event emit failed: {e}");
    }
}

pub fn notify(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!(target: "engine", "notification failed: {e}");
    }
}

/// Open (or focus) the always-on-top prompt window used for interventions,
/// check-ins and break-over prompts.
pub fn open_popup(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("intervention") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let result = tauri::WebviewWindowBuilder::new(
        app,
        "intervention",
        tauri::WebviewUrl::App("index.html#window=popup".into()),
    )
    .title("Accountability OS")
    .inner_size(520.0, 560.0)
    .resizable(false)
    .always_on_top(true)
    .center()
    .build();
    if let Err(e) = result {
        log::warn!(target: "engine", "failed to open prompt window: {e}");
    }
}

pub fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Spawn the monitoring + accountability thread.
pub fn spawn(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || {
            let mut real_probe: Box<dyn ActivityProbe> = os_probe();
            let mut demo_probe: Option<DemoProbe> = None;
            loop {
                let demo_mode = {
                    let state = app.state::<AppState>();
                    let engine = state.engine.lock();
                    engine.settings.demo_mode
                };
                let reading = if demo_mode {
                    let probe = demo_probe.get_or_insert_with(DemoProbe::new);
                    probe.read()
                } else {
                    demo_probe = None;
                    real_probe.read()
                };
                tick(&app, reading, demo_mode);
                std::thread::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS));
            }
        })
        .expect("spawn engine thread");
}

fn tick(app: &tauri::AppHandle, reading: ProbeReading, demo_mode: bool) {
    let state = app.state::<AppState>();
    let now = db::now();

    // Scheduled break boundaries can fall between polls. Expire the old
    // context before this tick's sample enters the aggregator.
    expire_break_if_due(app, now);

    // Rebuild the classification pipeline when rules/corrections changed.
    let needs_rebuild = { state.engine.lock().pipeline_dirty };
    if needs_rebuild {
        let rebuilt = state.db.with(|conn| {
            let (domain_rules, app_rules) = rules::load_engine_rules(conn)?;
            let corrections = sessions::load_corrections(conn)?;
            let classification_cache = rules::load_cache(conn)?;
            Ok((domain_rules, app_rules, corrections, classification_cache))
        });
        match rebuilt {
            Ok((domain_rules, app_rules, corrections, classification_cache)) => {
                let mut engine = state.engine.lock();
                engine.pipeline = ClassificationPipeline {
                    rules: RulesEngine {
                        domain_rules,
                        app_rules,
                    },
                    corrections: CorrectionMatcher { corrections },
                    private_processes: engine.settings.private_apps.clone(),
                };
                engine.classification_cache = classification_cache;
                engine.pipeline_dirty = false;
            }
            Err(e) => log::error!(target: "engine", "pipeline rebuild failed: {e}"),
        }
    }

    let mut finished_draft: Option<(SessionDraft, SessionContext)> = None;
    let mut live_needs_ai: Option<(String, ActivityContext, Option<ActiveCommitment>, u64)> = None;
    let mut tray_dirty = false;
    // Context-changing commands hold this same boundary from their old draft
    // flush through durable/runtime mutation. Keep monitor ingestion and any
    // resulting session insert on one side of that complete transition.
    let history_guard = state.activity_history_boundary.lock();

    {
        let mut engine = state.engine.lock();

        // Monitoring status bookkeeping.
        let new_state = match (&reading, engine.monitoring_paused, engine.settings.monitoring_consent) {
            (_, true, _) => MonitoringState::Paused,
            (_, _, false) => MonitoringState::Paused,
            (ProbeReading::Unavailable(_), _, _) => MonitoringState::PermissionRequired,
            (ProbeReading::Sample(_), _, _) if demo_mode => MonitoringState::Demo,
            (ProbeReading::Sample(_), _, _) => MonitoringState::Active,
        };
        if new_state != engine.monitoring_state {
            engine.monitoring_state = new_state;
            engine.monitoring_message = match &reading {
                ProbeReading::Unavailable(msg) => Some(msg.clone()),
                _ => None,
            };
            emit_event(app, &AppEvent::MonitoringStatus { state: new_state });
            tray_dirty = true;
        }

        let monitoring_on = matches!(new_state, MonitoringState::Active | MonitoringState::Demo);
        if !monitoring_on {
            // Monitoring stopped (paused, consent off, probe unavailable —
            // e.g. Demo Mode turned off on a non-Windows machine): close the
            // open draft and advance the gap floor on every poll. This stops
            // live time from accruing and prevents later idle backdating into
            // the period when monitoring was disabled.
            finished_draft = engine
                .aggregator
                .flush_at(now)
                .map(|d| (d, SessionContext::capture(&engine, state.activity_generation())));
            engine.current_activity = None;
        }
        if monitoring_on {
            if let ProbeReading::Sample(raw) = reading {
                let mut sample = ActivitySample {
                    timestamp: now,
                    app_name: raw.app_name,
                    process_name: raw.process_name,
                    window_title: raw.window_title,
                    idle_seconds: raw.idle_seconds,
                    locked: raw.locked,
                    browser_domain: raw.browser_domain,
                    browser_title: raw.browser_title,
                };

                // Merge live extension metadata for real browser sessions.
                if sample.browser_domain.is_none()
                    && engine.settings.browser_monitoring_enabled
                    && is_browser_process(&sample.process_name)
                {
                    if let Some(report) = &engine.last_extension_report {
                        // Fresh = within the extension's 30s heartbeat + margin.
                        if now - report.at <= 60 && report.window_focused {
                            sample.browser_domain = Some(report.domain.clone());
                            sample.browser_title = Some(report.title.clone());
                        }
                    }
                }

                let excluded = engine
                    .settings
                    .excluded_apps
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(&sample.process_name))
                    || sample.browser_domain.as_deref().is_some_and(|d| {
                        engine
                            .settings
                            .excluded_domains
                            .iter()
                            .any(|ex| aos_core::classify::domain_matches(ex, d))
                    });

                if excluded {
                    // Excluded = never recorded: close whatever was open and
                    // leave a hole in the timeline.
                    finished_draft = engine
                        .aggregator
                        .flush_at(sample.timestamp)
                        .map(|d| {
                            (
                                d,
                                SessionContext::capture(&engine, state.activity_generation()),
                            )
                        });
                    engine.current_activity = None;
                } else {
                    // Private apps: only "Private Application" + duration is
                    // ever stored (spec §52).
                    let is_private = engine.pipeline.is_private(&sample.process_name);
                    if is_private {
                        // The sentinel is itself recognized as private by the
                        // pipeline, so the redacted sample can never fall
                        // through to the cache/AI path (spec §52).
                        sample.app_name = "Private Application".into();
                        sample.window_title = String::new();
                        sample.browser_domain = None;
                        sample.browser_title = None;
                        sample.process_name = aos_core::types::PRIVATE_PROCESS_SENTINEL.into();
                    }

                    finished_draft = engine
                        .aggregator
                        .ingest(&sample)
                        .map(|d| {
                            (
                                d,
                                SessionContext::capture(&engine, state.activity_generation()),
                            )
                        });

                    // Live classification of the open activity.
                    let draft_is_idle = engine.aggregator.current_draft().map(|d| d.is_idle).unwrap_or(false);
                    let on_break = engine.current_break.is_some();
                    let ctx = ActivityContext {
                        app_name: sample.app_name.clone(),
                        process_name: sample.process_name.clone(),
                        window_title: sample.window_title.clone(),
                        browser_domain: sample.browser_domain.clone(),
                        browser_title: sample.browser_title.clone(),
                        commitment_id: engine.active_commitment.as_ref().map(|c| c.id),
                        project_id: engine.active_commitment.as_ref().and_then(|c| c.project_id),
                        in_focus_session: engine.focus_session_id.is_some(),
                        is_idle: draft_is_idle,
                    };
                    let key = activity_cache_key(
                        engine.active_commitment.as_ref(),
                        &ctx.process_name,
                        ctx.browser_domain.as_deref(),
                        &ctx.window_title,
                    );
                    let outcome = if on_break {
                        // Planned breaks are not distractions and not work:
                        // break time is idle-class for scoring (spec §17),
                        // and it never reaches the AI.
                        break_outcome()
                    } else {
                        match engine.pipeline.evaluate(&ctx) {
                            PipelineResult::Decided(o) => o,
                            PipelineResult::NeedsAi => {
                                let cached = engine.classification_cache.get(&key).cloned();
                                match cached {
                                    Some(o) => o,
                                    None => {
                                        if engine.settings.ai_classification_enabled
                                            && !engine.pending_ai_keys.contains(&key)
                                        {
                                            engine.pending_ai_keys.insert(key.clone());
                                            // Capture the commitment NOW — it may
                                            // change before the async task runs.
                                            live_needs_ai = Some((
                                                key.clone(),
                                                ctx.clone(),
                                                engine.active_commitment.clone(),
                                                state.activity_generation(),
                                            ));
                                        }
                                        ClassifyOutcome {
                                            classification: Classification::Unknown,
                                            confidence: 0.0,
                                            source: ClassificationSource::Default,
                                            reason: "Awaiting classification".into(),
                                        }
                                    }
                                }
                            }
                        }
                    };

                    let identity_changed = engine
                        .current_activity
                        .as_ref()
                        .map(|c| {
                            c.app_name != sample.app_name
                                || c.window_title != sample.window_title
                                || c.outcome.classification != outcome.classification
                        })
                        .unwrap_or(true);
                    let since = engine
                        .aggregator
                        .current_draft()
                        .map(|d| d.started_at)
                        .unwrap_or(now);
                    engine.current_activity = Some(CurrentActivity {
                        app_name: sample.app_name.clone(),
                        process_name: sample.process_name.clone(),
                        window_title: sample.window_title.clone(),
                        browser_domain: sample.browser_domain.clone(),
                        is_idle: draft_is_idle,
                        outcome: outcome.clone(),
                        since,
                    });
                    if identity_changed {
                        emit_event(
                            app,
                            &AppEvent::ActivityChanged {
                                app_name: sample.app_name.clone(),
                                window_title: sample.window_title.clone(),
                                classification: outcome.classification,
                            },
                        );
                    }
                }
            }
        }
    } // engine lock released

    // Store the finished session (and classify it) outside the lock, using
    // the context captured WITH the draft — a concurrent commitment command
    // must not re-attribute it or finalize a score before it is durable.
    if let Some((draft, sctx)) = finished_draft {
        store_finished_session(app, draft, sctx);
    }
    drop(history_guard);

    if tray_dirty {
        crate::tray::refresh(app);
    }

    // Fire the live AI classification if one is needed.
    if let Some((key, ctx, commitment, generation)) = live_needs_ai {
        spawn_ai_classification(app.clone(), key, ctx, commitment, vec![], generation);
    }

    run_accountability_checks(app, now);
}

/// The commitment/focus/break context a finished session is attributed
/// under. Captured while the engine lock is held, in the same moment the
/// draft is closed — never re-read at storage time.
pub struct SessionContext {
    commitment: Option<ActiveCommitment>,
    in_focus: bool,
    on_break: bool,
    generation: u64,
}

impl SessionContext {
    fn capture(engine: &crate::state::EngineState, generation: u64) -> Self {
        Self {
            commitment: engine.active_commitment.clone(),
            in_focus: engine.focus_session_id.is_some(),
            on_break: engine.current_break.is_some(),
            generation,
        }
    }
}

/// Planned-break activity is idle-class: not a distraction, not work,
/// excluded from scoring denominators (spec §17).
fn break_outcome() -> ClassifyOutcome {
    ClassifyOutcome {
        classification: Classification::Idle,
        confidence: 1.0,
        source: ClassificationSource::Rule,
        reason: PLANNED_BREAK_REASON.into(),
    }
}

/// Close the open activity session and run it through the normal
/// classification/storage path. Called when monitoring pauses, the app
/// quits, and — critically — when the commitment or break context changes:
/// those are session boundaries, so the time before them is classified and
/// attributed under the OLD context (call this BEFORE mutating state).
pub fn flush_open_session(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let flushed = {
        let mut engine = state.engine.lock();
        // Capture the context boundary only after excluding the monitor tick.
        // A backward wall-clock adjustment must not truncate a sample that is
        // already present in the aggregator either.
        let sampled_boundary = db::now();
        let boundary = engine
            .aggregator
            .current_draft()
            .map_or(sampled_boundary, |draft| sampled_boundary.max(draft.ended_at));
        engine.current_activity = None;
        engine
            .aggregator
            .flush_at(boundary)
            .map(|d| {
                (
                    d,
                    SessionContext::capture(&engine, state.activity_generation()),
                )
            })
    };
    if let Some((draft, sctx)) = flushed {
        store_finished_session(app, draft, sctx);
    }
}

/// Classify and persist a completed session under its captured context.
fn store_finished_session(app: &tauri::AppHandle, draft: SessionDraft, sctx: SessionContext) {
    let state = app.state::<AppState>();
    if state.activity_generation() != sctx.generation {
        return;
    }
    let generation = sctx.generation;
    let commitment_id = sctx.commitment.as_ref().map(|c| c.id);

    // Time on a planned break is stored as such — never scored, never sent
    // to the AI (spec §17).
    if sctx.on_break {
        let stored = state
            .db
            .with(|conn| {
                if state.activity_generation() != generation {
                    return Ok(vec![]);
                }
                sessions::insert(conn, &draft, &break_outcome(), commitment_id, false)
            });
        if matches!(&stored, Ok(ids) if ids.is_empty()) {
            return;
        }
        if let Err(e) = stored {
            log::error!(target: "engine", "failed to store break session: {e}");
        }
        return_after_emit(app);
        return;
    }

    let ctx = ActivityContext {
        app_name: draft.app_name.clone(),
        process_name: draft.process_name.clone(),
        window_title: draft.window_title.clone(),
        browser_domain: draft.browser_domain.clone(),
        browser_title: draft.browser_title.clone(),
        commitment_id,
        project_id: sctx.commitment.as_ref().and_then(|c| c.project_id),
        in_focus_session: sctx.in_focus,
        is_idle: draft.is_idle,
    };
    let key = activity_cache_key(
        sctx.commitment.as_ref(),
        &ctx.process_name,
        ctx.browser_domain.as_deref(),
        &ctx.window_title,
    );
    let (pipeline_result, ai_enabled) = {
        let engine = state.engine.lock();
        (
            engine.pipeline.evaluate(&ctx),
            engine.settings.ai_classification_enabled,
        )
    };
    // The async AI task must classify against the captured commitment, not
    // whatever is active when the task runs.
    let commitment = sctx.commitment;

    let insert = |outcome: &ClassifyOutcome, pending_ai: bool| {
        state
            .db
            .with(|conn| {
                if state.activity_generation() != generation {
                    return Ok(vec![]);
                }
                sessions::insert(conn, &draft, outcome, commitment_id, pending_ai)
            })
    };

    let stored = match pipeline_result {
        PipelineResult::Decided(outcome) => insert(&outcome, false),
        PipelineResult::NeedsAi => {
            let cached = state.db.with(|conn| rules::cache_get(conn, &key)).ok().flatten();
            match cached {
                Some(outcome) => insert(&outcome, false),
                None => {
                    let placeholder = ClassifyOutcome {
                        classification: Classification::Unknown,
                        confidence: 0.0,
                        source: ClassificationSource::Default,
                        reason: if ai_enabled {
                            "Awaiting AI classification".into()
                        } else {
                            "Ambiguous — mark it in the Activity view".into()
                        },
                    };
                    let ids = insert(&placeholder, ai_enabled);
                    if ai_enabled {
                        if let Ok(session_ids) = ids {
                            if session_ids.is_empty() {
                                return;
                            }
                            let already_pending = {
                                let mut engine = state.engine.lock();
                                !engine.pending_ai_keys.insert(key.clone())
                            };
                            if !already_pending {
                                spawn_ai_classification(
                                    app.clone(),
                                    key,
                                    ctx,
                                    commitment,
                                    session_ids,
                                    generation,
                                );
                            } else {
                                // The in-flight live task will fill the cache;
                                // patch these rows when it lands via the cache.
                                spawn_cache_patch(app.clone(), key, session_ids, generation);
                            }
                        }
                        return_after_emit(app);
                        return;
                    }
                    ids
                }
            }
        }
    };
    match stored {
        Ok(ids) if ids.is_empty() => return,
        Err(e) => log::error!(target: "engine", "failed to store session: {e}"),
        Ok(_) => {}
    }
    return_after_emit(app);
}

fn return_after_emit(app: &tauri::AppHandle) {
    emit_event(app, &AppEvent::SessionsUpdated);
}

/// Wait for an in-flight AI answer (same cache key) and patch the rows —
/// all segments of a midnight-split session, not just the newest.
fn spawn_cache_patch(
    app: tauri::AppHandle,
    key: String,
    session_ids: Vec<i64>,
    generation: u64,
) {
    if session_ids.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        for _ in 0..20 {
            tokio_sleep(3).await;
            let state = app.state::<AppState>();
            if state.activity_generation() != generation {
                return;
            }
            let cached = state.db.with(|conn| rules::cache_get(conn, &key)).ok().flatten();
            if let Some(outcome) = cached {
                let _ = state.db.with(|conn| {
                    if state.activity_generation() != generation {
                        return Ok(());
                    }
                    for id in &session_ids {
                        // Only rows still awaiting AI; a manual correction
                        // in the meantime wins.
                        if sessions::update_classification(conn, *id, &outcome)? {
                            crate::db::scores::refresh_stored_score_for_session(conn, *id)?;
                        }
                    }
                    Ok(())
                });
                emit_event(&app, &AppEvent::SessionsUpdated);
                emit_event(&app, &AppEvent::ScoresUpdated);
                return;
            }
        }
        // The in-flight task never delivered: stop showing "awaiting AI".
        let state = app.state::<AppState>();
        let cleared = state
            .db
            .with(|conn| sessions::clear_pending_ai(conn, &session_ids))
            .unwrap_or(0);
        if cleared > 0 {
            emit_event(&app, &AppEvent::SessionsUpdated);
        }
    });
}

async fn tokio_sleep(secs: u64) {
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_secs(secs))
    })
    .await
    .ok();
}

/// Call the AI provider for one ambiguous context; cache + apply the answer.
/// `commitment` is the commitment that was active when the context was
/// captured — never re-read at task time, or a fast commitment switch would
/// classify one commitment's activity against another.
fn spawn_ai_classification(
    app: tauri::AppHandle,
    key: String,
    ctx: ActivityContext,
    commitment: Option<ActiveCommitment>,
    session_ids: Vec<i64>,
    generation: u64,
) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if state.activity_generation() != generation {
            return;
        }
        let (base_url, model) = {
            let engine = state.engine.lock();
            (
                engine.settings.ai_base_url.clone(),
                engine.settings.ai_classify_model.clone(),
            )
        };
        let api_key = {
            let mut cached = state.ai_key.lock();
            if cached.is_none() {
                *cached = crate::ai::load_api_key().ok().flatten();
            }
            cached.clone()
        };
        let outcome = match (api_key, commitment) {
            (Some(api_key), Some(commitment)) => {
                let req = crate::ai::ClassifyRequest {
                    commitment_title: commitment.title,
                    done_definition: commitment.done_definition,
                    app_name: ctx.app_name.clone(),
                    window_title: ctx.window_title.clone(),
                    browser_domain: ctx.browser_domain.clone(),
                    browser_title: ctx.browser_title.clone(),
                };
                match crate::ai::classify_activity(&state.http, &base_url, &api_key, &model, &req).await {
                    Ok(ai) => Some(ClassificationPipeline::resolve_ai(
                        ai.classification,
                        ai.confidence,
                        ai.reason,
                    )),
                    Err(e) => {
                        log::warn!(target: "ai", "classification failed: {e}");
                        None
                    }
                }
            }
            _ => None,
        };

        let generation_is_current = {
            let mut engine = state.engine.lock();
            let current = state.activity_generation() == generation;
            if current {
                engine.pending_ai_keys.remove(&key);
            }
            current
        };
        if !generation_is_current {
            return;
        }

        let Some(outcome) = outcome else {
            // No answer is coming (request failed / no key / no commitment):
            // stop showing these rows as awaiting AI. They stay Unknown and
            // user-correctable rather than pending forever.
            let cleared = state
                .db
                .with(|conn| {
                    if state.activity_generation() != generation {
                        return Ok(0);
                    }
                    sessions::clear_pending_ai(conn, &session_ids)
                })
                .unwrap_or(0);
            if cleared > 0 {
                emit_event(&app, &AppEvent::SessionsUpdated);
            }
            return;
        };
        let cached_in_db = state
            .db
            .with(|conn| {
                if state.activity_generation() != generation {
                    return Ok(());
                }
                rules::cache_put(conn, &key, &outcome)
            })
            .is_ok();
        if state.activity_generation() != generation {
            return;
        }
        if !session_ids.is_empty() {
            let _ = state.db.with(|conn| {
                if state.activity_generation() != generation {
                    return Ok(());
                }
                for id in &session_ids {
                    // Only rows still awaiting AI; a manual correction made
                    // while the request was in flight wins.
                    if sessions::update_classification(conn, *id, &outcome)? {
                        crate::db::scores::refresh_stored_score_for_session(conn, *id)?;
                    }
                }
                Ok(())
            });
            emit_event(&app, &AppEvent::SessionsUpdated);
            emit_event(&app, &AppEvent::ScoresUpdated);
        }
        // If the user is still on this activity, refresh the live view.
        {
            let mut engine = state.engine.lock();
            if cached_in_db {
                engine.classification_cache.insert(key.clone(), outcome.clone());
            }
            let active_commitment = engine.active_commitment.clone();
            if let Some(current) = &mut engine.current_activity {
                let current_key = activity_cache_key(
                    active_commitment.as_ref(),
                    &current.process_name,
                    current.browser_domain.as_deref(),
                    &current.window_title,
                );
                if current_key == key {
                    current.outcome = outcome.clone();
                }
            }
        }
        emit_event(
            &app,
            &AppEvent::ActivityChanged {
                app_name: ctx.app_name,
                window_title: ctx.window_title,
                classification: outcome.classification,
            },
        );
    });
}

/// Finish an expired planned break before the next monitor sample enters the
/// aggregator. The history boundary serializes this scheduled transition with
/// manual break actions and activity deletion.
fn expire_break_if_due(app: &tauri::AppHandle, now: i64) {
    let state = app.state::<AppState>();
    let expired_break_id = {
        let engine = state.engine.lock();
        engine
            .current_break
            .as_ref()
            .filter(|(_, break_state)| !break_state.active(now))
            .map(|(id, _)| *id)
    };
    let Some(break_id) = expired_break_id else {
        return;
    };

    let _history_guard = state.activity_history_boundary.lock();
    let break_deadline = {
        let engine = state.engine.lock();
        engine
            .current_break
            .as_ref()
            .and_then(|(current_id, break_state)| {
                (*current_id == break_id && !break_state.active(now)).then_some(break_state.ends_at)
            })
    };
    let Some(break_deadline) = break_deadline else {
        return;
    };

    match state
        .db
        .with(|conn| engine_data::close_open_break_at(conn, break_id, break_deadline))
    {
        Ok(true) => {
            // Capture and close the old-context draft only after the exact
            // durable break row transitions. `flush_at` also preserves this
            // deadline as the lower bound for the next (possibly idle) sample.
            let (flushed, transitioned, commitment_title) = {
                let mut engine = state.engine.lock();
                let still_current = engine
                    .current_break
                    .as_ref()
                    .is_some_and(|(current_id, _)| *current_id == break_id);
                if still_current {
                    engine.current_activity = None;
                    let flushed = engine.aggregator.flush_at(break_deadline).map(|draft| {
                        (
                            draft,
                            SessionContext::capture(&engine, state.activity_generation()),
                        )
                    });
                    let commitment_title =
                        engine.active_commitment.as_ref().map(|c| c.title.clone());
                    engine.current_break = None;
                    engine.break_over_pending = true;
                    (flushed, true, commitment_title)
                } else {
                    (None, false, None)
                }
            };
            if let Some((draft, context)) = flushed {
                store_finished_session(app, draft, context);
            }
            if transitioned {
                emit_event(app, &AppEvent::BreakEnded);
                let body = match commitment_title {
                    Some(t) => format!("Break is over. Return to: {t}"),
                    None => "Break is over.".into(),
                };
                notify(app, "Break over", &body);
                open_popup(app);
            }
        }
        Ok(false) => {
            let mut engine = state.engine.lock();
            if engine
                .current_break
                .as_ref()
                .is_some_and(|(current_id, _)| *current_id == break_id)
            {
                // The durable row disappeared without this transition (for
                // example after external repair). Drop the stale old-context
                // draft rather than recreating deleted break history, while
                // retaining the boundary floor for the next sample.
                engine.current_activity = None;
                let _ = engine.aggregator.flush_at(break_deadline);
                engine.current_break = None;
                engine.break_over_pending = false;
            }
        }
        Err(error) => {
            log::error!(target: "engine", "failed to close expired break: {error}");
        }
    }
}

/// Distraction thresholds, check-ins, interview + review triggers.
fn run_accountability_checks(app: &tauri::AppHandle, now: i64) {
    let state = app.state::<AppState>();
    let today = today_local();
    let now_min = local_minutes_now();

    // -- Distraction tracking (spec §13) ------------------------------------
    let signal = {
        let mut engine = state.engine.lock();
        let generation = state.activity_generation();
        let classification = engine
            .current_activity
            .as_ref()
            .map(|c| c.outcome.classification)
            .unwrap_or(Classification::Idle);
        let suppressed = engine.monitoring_paused
            || engine.current_break.is_some()
            || engine.open_interruption.is_some()
            || engine.active_commitment.is_none()
            || !matches!(
                engine.monitoring_state,
                MonitoringState::Active | MonitoringState::Demo
            );
        let signal = engine.tracker.tick(now, classification, suppressed);
        signal.map(|signal| (signal, generation, current_context(&engine)))
    };
    if let Some((signal, generation, flagged)) = signal {
        handle_distraction_signal(app, signal, generation, flagged);
    }

    // -- Periodic check-ins (spec §18) --------------------------------------
    let unanswered_checkin = state
        .db
        .with(engine_data::unanswered_checkin)
        .map_or(true, |checkin| checkin.is_some());
    let checkin_due = {
        let engine = state.engine.lock();
        let generation = state.activity_generation();
        let in_hours = in_work_hours(
            now_min,
            engine.settings.work_start_min,
            engine.settings.work_end_min,
        );
        let suppressed = engine.monitoring_paused
            || engine.current_break.is_some()
            || engine.open_interruption.is_some()
            || engine.active_commitment.is_none()
            || unanswered_checkin;
        engine
            .checkin
            .due(now, in_hours, suppressed)
            .then_some(generation)
    };
    if let Some(generation) = checkin_due {
        trigger_checkin(app, now, generation);
    }

    // -- Morning interview (spec §5) ----------------------------------------
    let interview_candidate = {
        let engine = state.engine.lock();
        let after_time = now_min >= engine.settings.interview_time_min;
        let snoozed = engine.interview_snoozed_until.is_some_and(|t| now < t);
        let already_prompted = engine.interview_prompted_date.as_deref() == Some(today.as_str());
        after_time && !snoozed && !already_prompted
    };
    let interview_due = if interview_candidate {
        let plan_done = state
            .db
            .with(|conn| plans::get_plan_by_date(conn, &today))
            .ok()
            .flatten()
            .map(|plan| plan.locked_at.is_some() || plan.is_day_off)
            .unwrap_or(false);
        if plan_done {
            false
        } else {
            let mut engine = state.engine.lock();
            let still_due = now_min >= engine.settings.interview_time_min
                && !engine.interview_snoozed_until.is_some_and(|time| now < time)
                && engine.interview_prompted_date.as_deref() != Some(today.as_str());
            if still_due {
                engine.interview_prompted_date = Some(today.clone());
            }
            still_due
        }
    } else {
        false
    };
    if interview_due {
        emit_event(app, &AppEvent::InterviewDue);
        notify(
            app,
            "Plan your day",
            "What absolutely must be true by the end of today?",
        );
        show_main(app);
    }

    // -- End-of-day review (spec §21) ---------------------------------------
    let review_candidate = {
        let engine = state.engine.lock();
        let after_time = now_min >= engine.settings.review_time_min;
        let delayed = engine.review_delay_until.is_some_and(|t| now < t);
        let already_prompted = engine.review_prompted_date.as_deref() == Some(today.as_str());
        after_time && !delayed && !already_prompted
    };
    let review_due = if review_candidate {
        let has_open_plan = state
            .db
            .with(|conn| plans::get_plan_by_date(conn, &today))
            .ok()
            .flatten()
            .is_some_and(|plan| plan.locked_at.is_some() && plan.ended_at.is_none());
        if has_open_plan {
            let mut engine = state.engine.lock();
            let still_due = now_min >= engine.settings.review_time_min
                && !engine.review_delay_until.is_some_and(|time| now < time)
                && engine.review_prompted_date.as_deref() != Some(today.as_str());
            if still_due {
                engine.review_prompted_date = Some(today.clone());
            }
            still_due
        } else {
            false
        }
    } else {
        false
    };
    if review_due {
        emit_event(app, &AppEvent::ReviewDue);
        notify(app, "Daily review", "Time to close out the day and see your score.");
        show_main(app);
    }
}

fn handle_distraction_signal(
    app: &tauri::AppHandle,
    signal: DistractionSignal,
    generation: u64,
    flagged: FlaggedActivity,
) {
    let state = app.state::<AppState>();
    // Deletion and settings changes use this same boundary. Keep it through
    // persistence and user-visible publication so a prompt is definitively
    // before the boundary or rejected after it, never opened after deletion.
    let _history_guard = state.activity_history_boundary.lock();
    if state.activity_generation() != generation
        && !matches!(&signal, DistractionSignal::RecoveryComplete { .. })
    {
        let mut engine = state.engine.lock();
        reset_rejected_distraction(&mut engine.tracker, &signal);
        return;
    }
    match &signal {
        DistractionSignal::Warn { distracted_secs } => {
            let distracted_secs = *distracted_secs;
            let id = match state.db.with(|conn| {
                engine_data::create_interruption(
                    conn,
                    "warning",
                    flagged.commitment_id,
                    &engine_data::InterruptionContext {
                        app_name: &flagged.app_name,
                        process_name: &flagged.process_name,
                        browser_domain: flagged.browser_domain.as_deref(),
                        window_title: &flagged.window_title,
                    },
                    distracted_secs,
                    flagged.episode_started_at,
                )
            }) {
                Ok(id) => id,
                Err(error) => {
                    log::error!(target: "engine", "failed to store distraction warning: {error}");
                    let mut engine = state.engine.lock();
                    reset_rejected_distraction(&mut engine.tracker, &signal);
                    return;
                }
            };
            let accepted = {
                let mut engine = state.engine.lock();
                if state.activity_generation() == generation {
                    true
                } else {
                    reset_rejected_distraction(&mut engine.tracker, &signal);
                    false
                }
            };
            if !accepted {
                let _ = state.db.with(|conn| {
                    conn.execute(
                        "DELETE FROM interruptions WHERE id=?1 AND acknowledged_at IS NULL",
                        [id],
                    )?;
                    Ok(())
                });
                return;
            }
            emit_event(app, &AppEvent::DistractionWarning { distracted_secs });
            notify(
                app,
                "Off plan",
                &format!(
                    "{} for {} min. You committed to something else.",
                    flagged.app_name,
                    (distracted_secs / 60).max(1)
                ),
            );
        }
        DistractionSignal::Intervene { distracted_secs } => {
            let distracted_secs = *distracted_secs;
            let id = match state.db.with(|conn| {
                engine_data::create_interruption(
                    conn,
                    "intervention",
                    flagged.commitment_id,
                    &engine_data::InterruptionContext {
                        app_name: &flagged.app_name,
                        process_name: &flagged.process_name,
                        browser_domain: flagged.browser_domain.as_deref(),
                        window_title: &flagged.window_title,
                    },
                    distracted_secs,
                    flagged.episode_started_at,
                )
            }) {
                Ok(id) => id,
                Err(error) => {
                    log::error!(target: "engine", "failed to store intervention: {error}");
                    let mut engine = state.engine.lock();
                    reset_rejected_distraction(&mut engine.tracker, &signal);
                    return;
                }
            };
            let published = {
                let mut engine = state.engine.lock();
                if state.activity_generation() == generation {
                    engine.open_interruption = Some(id);
                    true
                } else {
                    reset_rejected_distraction(&mut engine.tracker, &signal);
                    false
                }
            };
            if !published {
                // A deletion/privacy boundary won the race after this row was
                // inserted. Never publish or retain a prompt from the stale
                // generation; otherwise it can outlive the user's deletion.
                let _ = state.db.with(|conn| {
                    conn.execute(
                        "DELETE FROM interruptions WHERE id=?1 AND acknowledged_at IS NULL",
                        [id],
                    )?;
                    Ok(())
                });
                return;
            }
            emit_event(
                app,
                &AppEvent::DistractionDetected {
                    distracted_secs,
                    app_name: flagged.app_name.clone(),
                    window_title: flagged.window_title,
                },
            );
            notify(
                app,
                "YOU'RE OFF PLAN",
                &format!("{} — {} minutes", flagged.app_name, distracted_secs / 60),
            );
            open_popup(app);
        }
        DistractionSignal::RecoveryComplete { recovery_secs } => {
            let recovery_secs = *recovery_secs;
            let Some(interruption_id) = flagged.recovering_interruption_id else {
                return;
            };
            let recorded = match state
                .db
                .with(|conn| {
                    engine_data::record_recovery(conn, interruption_id, recovery_secs)
                })
            {
                Ok(recorded) => recorded,
                Err(error) => {
                    log::error!(target: "engine", "failed to store distraction recovery: {error}");
                    return;
                }
            };
            {
                let mut engine = state.engine.lock();
                if engine.recovering_interruption_id == Some(interruption_id) {
                    engine.recovering_interruption_id = None;
                }
            }
            if !recorded {
                return;
            }
            if state.activity_generation() != generation {
                return;
            }
            emit_event(
                app,
                &AppEvent::DistractionResolved {
                    recovery_secs: Some(recovery_secs),
                },
            );
        }
    }
}

fn reset_rejected_distraction(
    tracker: &mut DistractionTracker,
    signal: &DistractionSignal,
) {
    if matches!(
        signal,
        DistractionSignal::Warn { .. } | DistractionSignal::Intervene { .. }
    ) {
        tracker.resolve();
    }
}

/// The flagged activity's full context, captured for the interruption row so
/// later answers ("this is actually work") teach about THIS activity, not
/// whatever is foreground when the user responds.
struct FlaggedActivity {
    commitment_id: Option<i64>,
    app_name: String,
    process_name: String,
    browser_domain: Option<String>,
    window_title: String,
    episode_started_at: Option<i64>,
    recovering_interruption_id: Option<i64>,
}

fn current_context(engine: &crate::state::EngineState) -> FlaggedActivity {
    let commitment_id = engine.active_commitment.as_ref().map(|c| c.id);
    let episode_started_at = engine.tracker.episode_started_at();
    match &engine.current_activity {
        Some(c) => FlaggedActivity {
            commitment_id,
            app_name: c.browser_domain.clone().unwrap_or_else(|| c.app_name.clone()),
            process_name: c.process_name.clone(),
            browser_domain: c.browser_domain.clone(),
            window_title: c.window_title.clone(),
            episode_started_at,
            recovering_interruption_id: engine.recovering_interruption_id,
        },
        None => FlaggedActivity {
            commitment_id,
            app_name: "Unknown".into(),
            process_name: String::new(),
            browser_domain: None,
            window_title: String::new(),
            episode_started_at,
            recovering_interruption_id: engine.recovering_interruption_id,
        },
    }
}

fn trigger_checkin(app: &tauri::AppHandle, now: i64, generation: u64) {
    let state = app.state::<AppState>();
    let _history_guard = state.activity_history_boundary.lock();
    if state.activity_generation() != generation {
        return;
    }
    let unanswered = state
        .db
        .with(engine_data::unanswered_checkin)
        .map_or(true, |checkin| checkin.is_some());
    let still_due = {
        let engine = state.engine.lock();
        let in_hours = in_work_hours(
            local_minutes_now(),
            engine.settings.work_start_min,
            engine.settings.work_end_min,
        );
        let suppressed = engine.monitoring_paused
            || engine.current_break.is_some()
            || engine.open_interruption.is_some()
            || engine.active_commitment.is_none()
            || unanswered
            || !matches!(
                engine.monitoring_state,
                MonitoringState::Active | MonitoringState::Demo
            );
        engine.checkin.due(now, in_hours, suppressed)
    };
    if !still_due {
        return;
    }
    // Include the still-open activity in the just-finished cadence window.
    // This is a normal aggregation boundary; the next sample starts a new
    // session without losing any time.
    flush_open_session(app);
    let (commitment_id, since) = {
        let engine = state.engine.lock();
        (
            engine.active_commitment.as_ref().map(|c| c.id),
            engine.checkin.last_at,
        )
    };
    // Stats and prompt creation share a transaction, including a final
    // unanswered check so racing ticks cannot create duplicate prompts.
    let created = state.db.with_tx(|tx| {
        if engine_data::unanswered_checkin(tx)?.is_some() {
            return Ok(None);
        }
        let mut totals = aos_core::scoring::DayTotals::default();
        for s in sessions::list_range(tx, since, now)? {
            if let Some(c) = Classification::parse(&s.classification) {
                totals.add(c, s.duration_seconds);
            }
        }
        let stats = serde_json::json!({
            "focused_secs": totals.focused_secs,
            "supporting_secs": totals.supporting_secs,
            "neutral_secs": totals.neutral_secs,
            "distracted_secs": totals.distracted_secs,
            "idle_secs": totals.idle_secs,
            "unknown_secs": totals.unknown_secs,
            "window_start": since,
        });
        Ok(Some(engine_data::create_checkin(
            tx,
            now,
            commitment_id,
            &stats,
        )?))
    });
    let Ok(Some(checkin_id)) = created else {
        return;
    };
    {
        let mut engine = state.engine.lock();
        engine.checkin.record(now);
    }
    emit_event(app, &AppEvent::CheckinDue { checkin_id });
    notify(app, "Accountability check", "Are you still working on the right thing?");
    open_popup(app);
}

/// Restore engine state from the DB after a restart (active focus session,
/// open break, today's snoozes are intentionally session-local).
pub fn restore(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    // Apply retention before hydrating durable pointers. Otherwise an old
    // focus, break, intervention, or check-in can be loaded into memory and
    // then deleted underneath the live engine later in this function.
    let retention_days = { state.engine.lock().settings.activity_retention_days };
    if retention_days > 0 {
        let _ = state
            .db
            .with_tx(|tx| sessions::prune_older_than(tx, retention_days));
    }
    let restored = state.db.with(|conn| {
        let focus = engine_data::active_focus(conn)?;
        let commitment = match &focus {
            Some(f) => Some(plans::get_commitment(conn, f.commitment_id)?),
            None => None,
        };
        let project_id = match &commitment {
            Some(c) => match c.task_id {
                Some(tid) => crate::db::tasks::get(conn, tid).ok().and_then(|t| t.project_id),
                None => None,
            },
            None => None,
        };
        Ok((focus, commitment, project_id))
    });
    if let Ok((focus, commitment, project_id)) = restored {
        let mut engine = state.engine.lock();
        engine.focus_session_id = focus.map(|f| f.id);
        engine.active_commitment = commitment.map(|c| crate::state::ActiveCommitment {
            id: c.id,
            title: c.title,
            done_definition: c.done_definition,
            project_id,
        });
    }
    // An intervention left unanswered before a restart is still owed an answer.
    if let Ok(Some(open)) = state.db.with(engine_data::open_interruption) {
        state.engine.lock().open_interruption = Some(open.id);
        open_popup(app);
    }
    // Periodic check-ins are durable accountability prompts too. Reopen the
    // popup after a restart instead of silently leaving an answer owed.
    if matches!(state.db.with(engine_data::unanswered_checkin), Ok(Some(_))) {
        open_popup(app);
    }
    // AI tasks that died with the previous process will never land; stop
    // showing those sessions as "awaiting AI" (they stay user-correctable).
    let _ = state.db.with(|conn| {
        conn.execute("UPDATE activity_sessions SET pending_ai=0 WHERE pending_ai=1", [])?;
        Ok(())
    });
    // A break that was running when the process died: resume it if time
    // remains, otherwise close its row at the planned end.
    if let Ok(Some(open_break)) = state.db.with(engine_data::open_break) {
        let now = db::now();
        if open_break.planned_end_at > now {
            let mut engine = state.engine.lock();
            engine.current_break = Some((
                open_break.id,
                aos_core::accountability::BreakState {
                    started_at: open_break.started_at,
                    ends_at: open_break.planned_end_at,
                },
            ));
        } else {
            let _ = state.db.with(|conn| {
                engine_data::close_break_at(conn, open_break.id, open_break.planned_end_at)
            });
        }
    }
    // Seed default blocked-domain rules once.
    if let Err(e) = state.db.with(rules::seed_defaults) {
        log::error!(target: "engine", "seeding default rules failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::{activity_cache_key, reset_rejected_distraction};
    use aos_core::accountability::{DistractionConfig, DistractionSignal, DistractionTracker};
    use aos_core::types::Classification;
    use crate::state::ActiveCommitment;

    #[test]
    fn activity_cache_key_tracks_commitment_semantics_without_storing_them() {
        let original = ActiveCommitment {
            id: 42,
            title: "Publish the brief".into(),
            done_definition: "The approved brief is published.".into(),
            project_id: None,
        };
        let cosmetic = ActiveCommitment {
            title: "  PUBLISH   THE brief ".into(),
            done_definition: "the approved brief is published.".into(),
            ..original.clone()
        };
        let revised = ActiveCommitment {
            done_definition: "The approved brief is published and emailed.".into(),
            ..original.clone()
        };

        let first = activity_cache_key(Some(&original), "editor.exe", None, "Brief");
        let same = activity_cache_key(Some(&cosmetic), "editor.exe", None, "Brief");
        let changed = activity_cache_key(Some(&revised), "editor.exe", None, "Brief");

        assert_eq!(first, same);
        assert_ne!(first, changed);
        assert!(!first.contains("Publish the brief"));
    }

    #[test]
    fn rejected_prompt_signal_resets_tracker_without_touching_recovery_events() {
        let mut tracker = DistractionTracker::new(DistractionConfig {
            warn_after_secs: 1,
            intervene_after_secs: 2,
            reset_after_secs: 60,
        });
        assert!(tracker.tick(0, Classification::Distracted, false).is_none());
        assert!(matches!(
            tracker.tick(1, Classification::Distracted, false),
            Some(DistractionSignal::Warn { .. })
        ));
        let intervention = tracker
            .tick(2, Classification::Distracted, false)
            .expect("the intervention threshold should be reached");
        assert!(matches!(intervention, DistractionSignal::Intervene { .. }));
        assert!(tracker.is_warned());

        reset_rejected_distraction(&mut tracker, &intervention);
        assert!(!tracker.is_warned());
        assert!(tracker.episode_started_at().is_none());

        tracker.tick(3, Classification::Distracted, false);
        let new_episode = tracker.episode_started_at();
        reset_rejected_distraction(
            &mut tracker,
            &DistractionSignal::RecoveryComplete { recovery_secs: 1 },
        );
        assert_eq!(tracker.episode_started_at(), new_episode);
    }
}
