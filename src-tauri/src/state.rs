//! Shared application state: the database plus the engine's runtime state.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::Serialize;

use aos_core::accountability::{BreakState, CheckinScheduler, DistractionConfig, DistractionTracker};
use aos_core::aggregator::{Aggregator, AggregatorConfig};
use aos_core::classify::ClassificationPipeline;
use aos_core::events::MonitoringState;
use aos_core::types::ClassifyOutcome;

use crate::db::settings::Settings;
use crate::db::Db;

/// Latest report from the browser extension (spec §9).
#[derive(Debug, Clone, Serialize)]
pub struct BrowserReport {
    pub domain: String,
    pub title: String,
    pub at: i64,
    pub window_focused: bool,
}

/// The commitment currently being worked on, denormalized for hot paths.
#[derive(Debug, Clone, Serialize)]
pub struct ActiveCommitment {
    pub id: i64,
    pub title: String,
    pub done_definition: String,
    pub project_id: Option<i64>,
}

/// What the user is doing right now (open, unstored session).
#[derive(Debug, Clone, Serialize)]
pub struct CurrentActivity {
    pub app_name: String,
    pub process_name: String,
    pub window_title: String,
    pub browser_domain: Option<String>,
    pub is_idle: bool,
    pub outcome: ClassifyOutcome,
    pub since: i64,
}

pub struct EngineState {
    pub settings: Settings,
    pub aggregator: Aggregator,
    pub pipeline: ClassificationPipeline,
    /// Set by commands that change rules/corrections/private apps; the
    /// monitor thread rebuilds the pipeline on the next tick.
    pub pipeline_dirty: bool,
    pub tracker: DistractionTracker,
    pub checkin: CheckinScheduler,

    pub monitoring_paused: bool,
    pub monitoring_state: MonitoringState,
    pub monitoring_message: Option<String>,

    pub active_commitment: Option<ActiveCommitment>,
    pub focus_session_id: Option<i64>,
    /// (breaks row id, break state)
    pub current_break: Option<(i64, BreakState)>,
    /// Break just ended; the prompt window should show "break over".
    pub break_over_pending: bool,
    pub open_interruption: Option<i64>,
    /// Exact intervention whose "return" response started recovery.
    pub recovering_interruption_id: Option<i64>,

    pub interview_snoozes: u32,
    pub interview_snoozed_until: Option<i64>,
    pub interview_prompted_date: Option<String>,
    pub review_prompted_date: Option<String>,
    pub review_delay_until: Option<i64>,

    pub current_activity: Option<CurrentActivity>,
    /// Cache keys with an in-flight AI classification, to avoid duplicates.
    pub pending_ai_keys: HashSet<String>,
    /// Most-recent persisted AI classifications, loaded alongside the
    /// pipeline so the monitor hot path never nests engine and DB locks.
    pub classification_cache: HashMap<String, ClassifyOutcome>,
    pub last_extension_report: Option<BrowserReport>,
}

impl EngineState {
    pub fn new(settings: Settings) -> Self {
        let started_at = crate::db::now();
        let mut aggregator = Aggregator::new(AggregatorConfig {
            idle_threshold_secs: settings.idle_threshold_secs,
            ..AggregatorConfig::default()
        });
        // The first idle sample must not be backdated to time before this app
        // instance was monitoring; that interval is an intentional data gap.
        aggregator.mark_gap(started_at);
        let tracker = DistractionTracker::new(DistractionConfig {
            warn_after_secs: settings.distraction_warn_secs,
            intervene_after_secs: settings.distraction_intervene_secs,
            ..DistractionConfig::default()
        });
        let checkin = CheckinScheduler::new(settings.checkin_cadence_min as i64 * 60, started_at);
        Self {
            settings,
            aggregator,
            pipeline: ClassificationPipeline::default(),
            pipeline_dirty: true,
            tracker,
            checkin,
            monitoring_paused: false,
            monitoring_state: MonitoringState::Paused,
            monitoring_message: None,
            active_commitment: None,
            focus_session_id: None,
            current_break: None,
            break_over_pending: false,
            open_interruption: None,
            recovering_interruption_id: None,
            interview_snoozes: 0,
            interview_snoozed_until: None,
            interview_prompted_date: None,
            review_prompted_date: None,
            review_delay_until: None,
            current_activity: None,
            pending_ai_keys: HashSet::new(),
            classification_cache: HashMap::new(),
            last_extension_report: None,
        }
    }

    /// Push settings changes into the live engine parts.
    pub fn apply_settings(&mut self, settings: Settings) {
        self.aggregator.set_idle_threshold(settings.idle_threshold_secs);
        self.tracker.set_config(DistractionConfig {
            warn_after_secs: settings.distraction_warn_secs,
            intervene_after_secs: settings.distraction_intervene_secs,
            ..*self.tracker.config()
        });
        self.checkin.cadence_secs = settings.checkin_cadence_min as i64 * 60;
        self.settings = settings;
        self.pipeline_dirty = true;
    }
}

pub struct AppState {
    pub db: Db,
    pub engine: Mutex<EngineState>,
    /// Serializes focus/break row creation and runtime publication with
    /// activity-history deletion. This prevents a deleted row from being
    /// published into the live engine after the privacy boundary completes.
    pub activity_history_boundary: Mutex<()>,
    /// Invalidates delayed activity/AI writes across deletion and privacy
    /// setting boundaries without introducing another lock-order dependency.
    pub activity_generation: AtomicU64,
    /// Cached AI API key (loaded from OS credential storage on first use).
    pub ai_key: Mutex<Option<String>>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn activity_generation(&self) -> u64 {
        self.activity_generation.load(Ordering::SeqCst)
    }

    pub fn invalidate_activity_tasks_with_engine(&self, engine: &mut EngineState) {
        self.activity_generation.fetch_add(1, Ordering::SeqCst);
        engine.pending_ai_keys.clear();
    }

    pub fn invalidate_activity_tasks(&self) {
        let mut engine = self.engine.lock();
        self.invalidate_activity_tasks_with_engine(&mut engine);
    }
}
