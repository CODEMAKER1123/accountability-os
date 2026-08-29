//! Turns raw foreground polls into aggregated activity sessions (spec §8).
//!
//! Polls arrive every few seconds; continuous activity on the same
//! app/window becomes ONE session row, not hundreds. Idle and lock are
//! detected here and back-dated to when input actually stopped.

use serde::{Deserialize, Serialize};

use crate::types::{ActivitySample, SessionDraft};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorConfig {
    /// Seconds without input before the user counts as idle (spec §10 Idle).
    pub idle_threshold_secs: u64,
    /// Sessions shorter than this are dropped as switch noise.
    pub min_session_secs: i64,
    /// A gap between samples larger than this (sleep, crash) closes the
    /// current session at the last known timestamp instead of bridging it.
    pub max_gap_secs: i64,
}

impl Default for AggregatorConfig {
    fn default() -> Self {
        Self {
            idle_threshold_secs: 180,
            min_session_secs: 5,
            max_gap_secs: 30,
        }
    }
}

/// Normalizes window titles so cosmetic changes don't split sessions and
/// cache keys stay stable: collapse whitespace, strip unsaved-marker
/// prefixes and notification counters like "(3) ".
pub fn normalize_title(title: &str) -> String {
    let mut t = title.trim();
    // Strip leading unsaved markers and notification counts: "● ", "* ", "(3) "
    loop {
        let before = t;
        t = t.trim_start_matches(['●', '*', '•']).trim_start();
        if t.starts_with('(') {
            if let Some(end) = t.find(')') {
                let inner = &t[1..end];
                if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) {
                    t = t[end + 1..].trim_start();
                }
            }
        }
        if t == before {
            break;
        }
    }
    let collapsed: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActivityIdentity {
    process_name: String,
    browser_domain: Option<String>,
    normalized_title: String,
    is_idle: bool,
}

impl ActivityIdentity {
    fn of(sample: &ActivitySample, idle: bool) -> Self {
        if idle {
            return Self {
                process_name: String::new(),
                browser_domain: None,
                normalized_title: String::new(),
                is_idle: true,
            };
        }
        Self {
            process_name: sample.process_name.to_lowercase(),
            browser_domain: sample.browser_domain.as_deref().map(str::to_lowercase),
            normalized_title: normalize_title(&sample.window_title),
            is_idle: false,
        }
    }
}

/// Stateful aggregator: feed it samples in order; it emits a finished
/// `SessionDraft` whenever the activity identity changes.
#[derive(Debug)]
pub struct Aggregator {
    config: AggregatorConfig,
    current: Option<(ActivityIdentity, SessionDraft)>,
    /// Earliest safe start for the next recorded sample after a context or
    /// privacy gap. It survives empty flushes and is consumed by `ingest`.
    next_start_floor: Option<i64>,
}

impl Aggregator {
    pub fn new(config: AggregatorConfig) -> Self {
        Self {
            config,
            current: None,
            next_start_floor: None,
        }
    }

    pub fn config(&self) -> &AggregatorConfig {
        &self.config
    }

    pub fn set_idle_threshold(&mut self, secs: u64) {
        self.config.idle_threshold_secs = secs;
    }

    /// Record a gap or context boundary that future idle backdating must not
    /// cross. Repeated boundaries only move the floor forward.
    pub fn mark_gap(&mut self, timestamp: i64) {
        self.next_start_floor = Some(
            self.next_start_floor
                .map_or(timestamp, |current| current.max(timestamp)),
        );
    }

    /// Feed one poll. Returns a completed session when one just ended.
    pub fn ingest(&mut self, sample: &ActivitySample) -> Option<SessionDraft> {
        let idle = sample.locked || sample.idle_seconds >= self.config.idle_threshold_secs;
        let identity = ActivityIdentity::of(sample, idle);
        let floor = self.next_start_floor.take();
        let latest_open_timestamp = self.current.as_ref().map(|(_, draft)| draft.ended_at);
        let timestamp_floor = floor
            .into_iter()
            .chain(latest_open_timestamp)
            .max()
            .unwrap_or(sample.timestamp);
        let observed_ts = sample.timestamp.max(timestamp_floor);

        // When idle is first detected, input actually stopped `idle_seconds`
        // ago — the tail of the previous session was really idle time.
        let mut effective_ts = if idle && !sample.locked {
            (observed_ts - sample.idle_seconds as i64).max(0)
        } else {
            observed_ts
        };
        if let Some(floor) = floor {
            // A context boundary may have occurred between polls. Idle
            // backdating is still useful within the continuity window, but it
            // must not cross the boundary or bridge a suspended monitor.
            let within_gap = sample.timestamp >= floor
                && sample.timestamp.saturating_sub(floor) <= self.config.max_gap_secs.max(0);
            effective_ts = if sample.timestamp < floor {
                floor
            } else if within_gap {
                effective_ts.max(floor)
            } else {
                observed_ts
            };
        }
        let stale_against_open = latest_open_timestamp
            .is_some_and(|latest| sample.timestamp <= latest);
        if sample.timestamp < timestamp_floor || stale_against_open {
            // A retimestamped poll cannot retroactively turn activity already
            // observed at the monotonic floor into idle time.
            effective_ts = effective_ts.max(timestamp_floor);
        }

        // An idle transition explains its own gap: idleness began at
        // `effective_ts`, so continuity is judged from there, not the poll.
        let continuity_ts = if idle {
            effective_ts.min(observed_ts)
        } else {
            observed_ts
        };
        let (same_identity, gap_exceeded) = match &self.current {
            None => {
                self.current = Some((
                    identity,
                    draft_from(sample, effective_ts, observed_ts, idle),
                ));
                return None;
            }
            Some((cur_id, draft)) => (
                *cur_id == identity,
                continuity_ts - draft.ended_at > self.config.max_gap_secs,
            ),
        };

        if same_identity && !gap_exceeded {
            let (_, draft) = self.current.as_mut().expect("current checked above");
            draft.ended_at = observed_ts;
            // Browser metadata can arrive a beat late; merge it in.
            if draft.browser_title.is_none() && sample.browser_title.is_some() {
                draft.browser_title = sample.browser_title.clone();
            }
            return None;
        }

        let mut finished = self.take_current();
        let new_start = if gap_exceeded {
            // System slept or the monitor was paused: leave the old session
            // ending where we last saw it and start fresh at the new poll.
            observed_ts
        } else {
            let f = finished.as_mut().expect("current existed");
            // The old state really ended when the new one began: at the poll
            // for a plain switch, back-dated for an idle transition.
            f.ended_at = effective_ts.clamp(f.started_at, observed_ts);
            f.ended_at
        };
        self.current = Some((
            identity,
            draft_from(sample, new_start, observed_ts, idle),
        ));
        self.emit(finished)
    }

    /// Close and return whatever session is open (shutdown, pause).
    pub fn flush(&mut self) -> Option<SessionDraft> {
        let finished = self.take_current();
        self.emit(finished)
    }

    /// Close the open session at a known context boundary rather than at the
    /// timestamp of the most recent poll. Scheduled transitions can fall
    /// between polls, and old-context activity must stop at that exact instant.
    pub fn flush_at(&mut self, ended_at: i64) -> Option<SessionDraft> {
        self.mark_gap(ended_at);
        let mut finished = self.take_current();
        if let Some(draft) = &mut finished {
            // Truncation is always safe. Extension is safe only across the
            // same continuity window used by `ingest`; never bridge sleep or
            // a suspended monitor merely because a scheduled boundary passed.
            let can_move_to_boundary = ended_at <= draft.ended_at
                || ended_at.saturating_sub(draft.ended_at) <= self.config.max_gap_secs.max(0);
            if can_move_to_boundary {
                draft.ended_at = ended_at.max(draft.started_at);
            }
        }
        self.emit(finished)
    }

    /// The currently open (unfinished) session, for live UI display.
    pub fn current_draft(&self) -> Option<&SessionDraft> {
        self.current.as_ref().map(|(_, d)| d)
    }

    fn take_current(&mut self) -> Option<SessionDraft> {
        self.current.take().map(|(_, d)| d)
    }

    fn emit(&self, finished: Option<SessionDraft>) -> Option<SessionDraft> {
        finished.filter(|f| f.duration_seconds() >= self.config.min_session_secs)
    }
}

fn draft_from(
    sample: &ActivitySample,
    started_at: i64,
    observed_at: i64,
    idle: bool,
) -> SessionDraft {
    if idle {
        SessionDraft {
            started_at,
            ended_at: observed_at,
            app_name: "Idle".into(),
            process_name: String::new(),
            window_title: String::new(),
            browser_domain: None,
            browser_title: None,
            is_idle: true,
        }
    } else {
        SessionDraft {
            started_at,
            ended_at: observed_at,
            app_name: sample.app_name.clone(),
            process_name: sample.process_name.clone(),
            window_title: sample.window_title.clone(),
            browser_domain: sample.browser_domain.clone(),
            browser_title: sample.browser_title.clone(),
            is_idle: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ts: i64, process: &str, title: &str, idle: u64) -> ActivitySample {
        ActivitySample {
            timestamp: ts,
            app_name: process.trim_end_matches(".exe").to_string(),
            process_name: process.to_string(),
            window_title: title.to_string(),
            idle_seconds: idle,
            locked: false,
            browser_domain: None,
            browser_title: None,
        }
    }

    #[test]
    fn continuous_activity_becomes_one_session() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        for i in 0..200 {
            let out = agg.ingest(&sample(1000 + i * 5, "chrome.exe", "Playbook - Google Docs", 0));
            assert!(out.is_none(), "no session should close mid-activity");
        }
        let s = agg.flush().expect("flush returns the open session");
        assert_eq!(s.started_at, 1000);
        assert_eq!(s.ended_at, 1000 + 199 * 5);
        assert_eq!(s.process_name, "chrome.exe");
        assert!(!s.is_idle);
    }

    #[test]
    fn app_switch_closes_previous_session() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        for i in 0..12 {
            agg.ingest(&sample(1000 + i * 5, "chrome.exe", "Docs", 0));
        }
        let closed = agg
            .ingest(&sample(1060, "outlook.exe", "Inbox", 0))
            .expect("switch closes chrome session");
        assert_eq!(closed.process_name, "chrome.exe");
        assert_eq!(closed.ended_at, 1060);
        let cur = agg.current_draft().unwrap();
        assert_eq!(cur.process_name, "outlook.exe");
        assert_eq!(cur.started_at, 1060);
    }

    #[test]
    fn title_change_within_app_is_a_new_session() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        for i in 0..4 {
            agg.ingest(&sample(1000 + i * 5, "chrome.exe", "Doc A - Google Docs", 0));
        }
        let closed = agg.ingest(&sample(1020, "chrome.exe", "Doc B - Google Docs", 0));
        assert!(closed.is_some(), "different document = different session");
    }

    #[test]
    fn cosmetic_title_change_does_not_split_session() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        agg.ingest(&sample(1000, "slack.exe", "general - Slack", 0));
        let out = agg.ingest(&sample(1005, "slack.exe", "(3) general - Slack", 0));
        assert!(out.is_none(), "unread-count prefix should not split the session");
    }

    #[test]
    fn idle_is_backdated_to_when_input_stopped() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        // Working at t=1000..1180, then polls keep coming but idle grows.
        for i in 0..37 {
            agg.ingest(&sample(1000 + i * 5, "chrome.exe", "Docs", 0));
        }
        // At t=1365 the system reports 185s idle -> idle began at t=1180.
        let closed = agg
            .ingest(&sample(1365, "chrome.exe", "Docs", 185))
            .expect("idle transition closes work session");
        assert_eq!(closed.ended_at, 1180, "work session truncated to last input");
        let cur = agg.current_draft().unwrap();
        assert!(cur.is_idle);
        assert_eq!(cur.started_at, 1180, "idle session starts when input stopped");
    }

    #[test]
    fn returning_from_idle_starts_new_work_session() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        agg.ingest(&sample(1000, "chrome.exe", "Docs", 200));
        assert!(agg.current_draft().unwrap().is_idle);
        let closed = agg.ingest(&sample(1300, "chrome.exe", "Docs", 2));
        let closed = closed.expect("idle session closes on activity");
        assert!(closed.is_idle);
        assert!(!agg.current_draft().unwrap().is_idle);
    }

    #[test]
    fn lock_counts_as_idle_immediately() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        agg.ingest(&sample(1000, "chrome.exe", "Docs", 0));
        let mut locked = sample(1010, "chrome.exe", "Docs", 0);
        locked.locked = true;
        let closed = agg.ingest(&locked).expect("lock closes work session");
        assert!(!closed.is_idle);
        assert!(agg.current_draft().unwrap().is_idle);
    }

    #[test]
    fn short_sessions_are_dropped_as_noise() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        agg.ingest(&sample(1000, "chrome.exe", "Docs", 0));
        agg.ingest(&sample(1002, "explorer.exe", "Desktop", 0)); // 2s chrome — dropped
        let out = agg.ingest(&sample(1030, "outlook.exe", "Inbox", 0));
        let closed = out.expect("28s explorer session survives");
        assert_eq!(closed.process_name, "explorer.exe");
    }

    #[test]
    fn large_gap_closes_session_at_last_seen_timestamp() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        agg.ingest(&sample(1000, "chrome.exe", "Docs", 0));
        agg.ingest(&sample(1030, "chrome.exe", "Docs", 0));
        // Laptop slept for an hour.
        let closed = agg
            .ingest(&sample(4630, "chrome.exe", "Docs", 0))
            .expect("gap closes stale session");
        assert_eq!(closed.ended_at, 1030, "must not bridge a sleep gap");
        assert_eq!(agg.current_draft().unwrap().started_at, 4630);
    }

    #[test]
    fn scheduled_boundary_truncates_open_session_exactly() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        agg.ingest(&sample(1000, "chrome.exe", "Docs", 0));
        agg.ingest(&sample(1012, "chrome.exe", "Docs", 0));

        let closed = agg.flush_at(1010).expect("boundary closes the session");

        assert_eq!(closed.started_at, 1000);
        assert_eq!(closed.ended_at, 1010);
        assert!(agg.current_draft().is_none());
        assert!(agg.flush().is_none(), "empty flush must retain the boundary");

        agg.ingest(&sample(1020, "chrome.exe", "Docs", 200));
        let post_boundary = agg.current_draft().expect("new idle draft");
        assert!(post_boundary.is_idle);
        assert_eq!(
            post_boundary.started_at, 1010,
            "idle must not overlap boundary"
        );
    }

    #[test]
    fn scheduled_boundary_does_not_bridge_a_sleep_gap() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        agg.ingest(&sample(1000, "chrome.exe", "Docs", 0));
        agg.ingest(&sample(1005, "chrome.exe", "Docs", 0));

        let closed = agg.flush_at(1100).expect("boundary closes the session");

        assert_eq!(closed.ended_at, 1005, "unobserved gap must remain a gap");

        agg.ingest(&sample(5000, "chrome.exe", "Docs", 5000));
        let resumed = agg.current_draft().expect("resumed idle draft");
        assert_eq!(resumed.started_at, 5000, "resume must start at its sample");
    }

    #[test]
    fn stale_clock_sample_cannot_cross_a_recorded_boundary() {
        let mut agg = Aggregator::new(AggregatorConfig::default());
        agg.ingest(&sample(1000, "chrome.exe", "Docs", 0));
        agg.ingest(&sample(1005, "chrome.exe", "Docs", 0));
        let closed = agg.flush_at(1010).expect("boundary closes the session");
        assert_eq!(closed.ended_at, 1010);

        agg.ingest(&sample(1008, "chrome.exe", "Docs", 0));
        let stale = agg.current_draft().expect("stale sample is retimestamped");
        assert_eq!(stale.started_at, 1010);
        assert_eq!(stale.ended_at, 1010);

        agg.ingest(&sample(1009, "chrome.exe", "Docs", 0));
        assert_eq!(agg.current_draft().unwrap().ended_at, 1010);

        agg.ingest(&sample(1020, "chrome.exe", "Docs", 0));
        let active = agg
            .ingest(&sample(1020, "chrome.exe", "Docs", 200))
            .expect("equal-timestamp idle transition closes active time");
        assert_eq!(active.ended_at, 1020);
        let idle = agg.current_draft().expect("idle draft starts after active time");
        assert!(idle.is_idle);
        assert_eq!(idle.started_at, 1020);
        assert_eq!(idle.ended_at, 1020);
    }

    #[test]
    fn normalize_title_strips_counters_and_markers() {
        assert_eq!(normalize_title("(3) Inbox - Outlook"), "Inbox - Outlook");
        assert_eq!(normalize_title("● draft.md - VS Code"), "draft.md - VS Code");
        assert_eq!(normalize_title("  spaced   out  "), "spaced out");
        assert_eq!(normalize_title("(beta) thing"), "(beta) thing", "non-numeric parens kept");
    }
}
