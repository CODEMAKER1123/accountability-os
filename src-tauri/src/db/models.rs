//! Row types crossing IPC to the frontend. Enum-ish fields stay strings
//! here; core logic parses them into `aos_core` types where it matters.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub archived: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub project_id: Option<i64>,
    pub parent_task_id: Option<i64>,
    pub status: String,
    pub priority: String,
    pub estimated_minutes: Option<i64>,
    pub due_date: Option<String>,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyPlan {
    pub id: i64,
    pub date: String,
    pub locked_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub likely_distraction: String,
    pub countermeasure: String,
    pub most_important_when: String,
    pub is_day_off: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitmentStep {
    pub title: String,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub id: i64,
    pub plan_id: i64,
    pub task_id: Option<i64>,
    pub title: String,
    pub done_definition: String,
    pub estimated_minutes: Option<i64>,
    pub priority: String,
    pub rank: i64,
    pub status: String,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub outcome_reason: Option<String>,
    pub outcome_note: Option<String>,
    pub steps: Vec<CommitmentStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivitySessionRow {
    pub id: i64,
    pub local_date: String,
    pub started_at: i64,
    pub ended_at: i64,
    pub duration_seconds: i64,
    pub application_name: String,
    pub process_name: String,
    pub window_title: String,
    pub browser_domain: Option<String>,
    pub browser_title: Option<String>,
    pub classification: String,
    pub classification_confidence: Option<f64>,
    pub classification_source: String,
    pub classification_reason: Option<String>,
    pub related_commitment_id: Option<i64>,
    pub is_idle: bool,
    pub pending_ai: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusSessionRow {
    pub id: i64,
    pub commitment_id: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckinRow {
    pub id: i64,
    pub due_at: i64,
    pub shown_at: Option<i64>,
    pub commitment_id: Option<i64>,
    pub window_stats: serde_json::Value,
    pub response: Option<String>,
    pub response_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptionRow {
    pub id: i64,
    pub kind: String,
    pub commitment_id: Option<i64>,
    pub app_name: String,
    pub process_name: String,
    pub browser_domain: Option<String>,
    pub window_title: String,
    pub distracted_secs: i64,
    pub episode_started_at: Option<i64>,
    pub started_at: i64,
    pub acknowledged_at: Option<i64>,
    pub response: Option<String>,
    pub response_note: Option<String>,
    pub returned_at: Option<i64>,
    pub recovery_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakRow {
    pub id: i64,
    pub started_at: i64,
    pub planned_end_at: i64,
    pub actual_end_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainRuleRow {
    pub id: i64,
    pub domain: String,
    pub classification: String,
    pub project_id: Option<i64>,
    pub commitment_id: Option<i64>,
    pub only_in_focus: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRuleRow {
    pub id: i64,
    pub process_name: String,
    pub classification: String,
    pub project_id: Option<i64>,
    pub commitment_id: Option<i64>,
    pub only_in_focus: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyScoreRow {
    pub date: String,
    pub total: Option<f64>,
    pub completion: Option<f64>,
    pub alignment: Option<f64>,
    pub focus_quality: Option<f64>,
    pub planning_accuracy: Option<f64>,
    pub focused_secs: i64,
    pub supporting_secs: i64,
    pub neutral_secs: i64,
    pub distracted_secs: i64,
    pub idle_secs: i64,
    pub unknown_secs: i64,
    pub context_switches: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightRow {
    pub id: i64,
    pub period: String,
    pub metric: String,
    pub text: String,
    pub source: String,
    pub created_at: i64,
}
