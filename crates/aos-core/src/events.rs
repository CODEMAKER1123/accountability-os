//! Internal event model (spec §31). The engine emits these; the UI renders
//! them. Serialized as a tagged union on a single event channel.

use serde::{Deserialize, Serialize};

use crate::types::Classification;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AppEvent {
    ActivityChanged {
        app_name: String,
        window_title: String,
        classification: Classification,
    },
    UserIdle,
    UserActive,
    FocusStarted {
        commitment_id: i64,
    },
    FocusEnded {
        commitment_id: i64,
    },
    CommitmentChanged {
        commitment_id: Option<i64>,
    },
    DistractionWarning {
        distracted_secs: i64,
    },
    DistractionDetected {
        distracted_secs: i64,
        app_name: String,
        window_title: String,
    },
    DistractionResolved {
        recovery_secs: Option<i64>,
    },
    CheckinDue {
        checkin_id: i64,
    },
    CheckinAnswered {
        checkin_id: i64,
    },
    PriorityChangeRequested {
        commitment_id: Option<i64>,
    },
    BlockedFlowRequested {
        commitment_id: Option<i64>,
    },
    BreakStarted {
        ends_at: i64,
    },
    BreakEnded,
    TaskCompleted {
        task_id: i64,
    },
    DayLocked {
        plan_id: i64,
    },
    DayEnded {
        plan_id: i64,
    },
    InterviewDue,
    ReviewDue,
    MonitoringStatus {
        state: MonitoringState,
    },
    /// Activity sessions changed; timeline views should refetch.
    SessionsUpdated,
    /// Scores changed; scorecards should refetch.
    ScoresUpdated,
}

/// Monitoring status shown in the UI at all times (spec §41).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringState {
    Active,
    Paused,
    PermissionRequired,
    /// Demo simulation driving the monitor instead of the OS.
    Demo,
}
