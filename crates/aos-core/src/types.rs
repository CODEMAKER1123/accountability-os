use serde::{Deserialize, Serialize};

/// Process-name sentinel stored for private applications after redaction
/// (spec §52). The pipeline treats it as private so a scrubbed sample can
/// never fall through to the rules/cache/AI path.
pub const PRIVATE_PROCESS_SENTINEL: &str = "__private__";

/// Stable reason stored for activity that occurred during an intentional
/// break. Consumers use it to keep planned rest out of productivity math.
pub const PLANNED_BREAK_REASON: &str = "Planned break";

/// One classification per activity session (spec §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Focused,
    Supporting,
    Neutral,
    Distracted,
    Idle,
    /// Low-confidence result awaiting user confirmation (spec §12).
    Unknown,
}

impl Classification {
    /// Alignment weight used by scoring (spec §10). Idle/Unknown carry no weight
    /// and are excluded from the working-time denominator separately.
    pub fn weight(self) -> f64 {
        match self {
            Classification::Focused => 1.0,
            Classification::Supporting => 0.7,
            Classification::Neutral => 0.25,
            Classification::Distracted => 0.0,
            Classification::Idle | Classification::Unknown => 0.0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Classification::Focused => "focused",
            Classification::Supporting => "supporting",
            Classification::Neutral => "neutral",
            Classification::Distracted => "distracted",
            Classification::Idle => "idle",
            Classification::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "focused" => Some(Classification::Focused),
            "supporting" => Some(Classification::Supporting),
            "neutral" => Some(Classification::Neutral),
            "distracted" => Some(Classification::Distracted),
            "idle" => Some(Classification::Idle),
            "unknown" => Some(Classification::Unknown),
            _ => None,
        }
    }
}

/// Where a classification came from, most→least authoritative for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationSource {
    Manual,
    Correction,
    Rule,
    Cache,
    Ai,
    Default,
}

impl ClassificationSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ClassificationSource::Manual => "manual",
            ClassificationSource::Correction => "correction",
            ClassificationSource::Rule => "rule",
            ClassificationSource::Cache => "cache",
            ClassificationSource::Ai => "ai",
            ClassificationSource::Default => "default",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Inbox,
    Planned,
    Committed,
    Active,
    Completed,
    Deferred,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Must,
    Should,
    Could,
}

impl Priority {
    /// Weight used by the execution score (spec §19).
    pub fn weight(self) -> f64 {
        match self {
            Priority::Must => 3.0,
            Priority::Should => 2.0,
            Priority::Could => 1.0,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "must" => Some(Priority::Must),
            "should" => Some(Priority::Should),
            "could" => Some(Priority::Could),
            _ => None,
        }
    }
}

/// A single foreground poll from the monitoring probe (spec §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivitySample {
    /// Unix seconds.
    pub timestamp: i64,
    /// Human-friendly application name ("Chrome", "Outlook").
    pub app_name: String,
    /// Process executable name ("chrome.exe").
    pub process_name: String,
    pub window_title: String,
    /// Seconds since last user input, system-wide.
    pub idle_seconds: u64,
    /// True while the workstation is locked.
    pub locked: bool,
    /// Provided by the browser extension when the browser is foreground.
    pub browser_domain: Option<String>,
    pub browser_title: Option<String>,
}

/// An aggregated activity session, pre-classification (spec §30).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDraft {
    pub started_at: i64,
    pub ended_at: i64,
    pub app_name: String,
    pub process_name: String,
    pub window_title: String,
    pub browser_domain: Option<String>,
    pub browser_title: Option<String>,
    pub is_idle: bool,
}

impl SessionDraft {
    pub fn duration_seconds(&self) -> i64 {
        (self.ended_at - self.started_at).max(0)
    }
}

/// Everything the classification pipeline needs to know about the current
/// activity + commitment context. Kept minimal by design (spec §3).
#[derive(Debug, Clone, Default)]
pub struct ActivityContext {
    pub app_name: String,
    pub process_name: String,
    pub window_title: String,
    pub browser_domain: Option<String>,
    pub browser_title: Option<String>,
    pub commitment_id: Option<i64>,
    pub project_id: Option<i64>,
    /// True while a focus session is running — blocked domains default to
    /// Distracted only in focus mode (spec §11 layer 1).
    pub in_focus_session: bool,
    pub is_idle: bool,
}

/// Result of the classification pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassifyOutcome {
    pub classification: Classification,
    pub confidence: f64,
    pub source: ClassificationSource,
    pub reason: String,
}
