//! Accountability logic (spec §13–18, §28): distraction thresholds, recovery
//! tracking, check-in scheduling, breaks, strict mode, commitment switching.

use serde::{Deserialize, Serialize};

use crate::types::Classification;

/// Do not normally allow more than 3 daily commitments (spec §6).
pub const MAX_COMMITMENTS: usize = 3;

/// The pushback line when the user over-commits (spec §6 Q2).
pub fn too_many_commitments_message(selected: usize) -> Option<String> {
    if selected <= MAX_COMMITMENTS {
        None
    } else {
        Some(format!(
            "You have selected {selected} priorities. {selected} priorities means you have no priorities. Pick the {MAX_COMMITMENTS} that matter most."
        ))
    }
}

/// Switching priorities requires a reason (spec §7): intentional, not hard.
pub fn validate_switch_reason(reason: &str) -> Result<(), &'static str> {
    if reason.trim().len() < 3 {
        Err("A short reason is required to switch commitments.")
    } else {
        Ok(())
    }
}

/// True when local time (minutes since midnight) is inside the configured
/// workday. Handles overnight windows (end < start).
pub fn in_work_hours(now_min: u32, start_min: u32, end_min: u32) -> bool {
    if start_min == end_min {
        return true; // degenerate config: treat as always
    }
    if start_min < end_min {
        (start_min..end_min).contains(&now_min)
    } else {
        now_min >= start_min || now_min < end_min
    }
}

// ---------------------------------------------------------------------------
// Distraction detection (spec §13–14)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DistractionConfig {
    /// Yellow warning threshold (default 3 minutes).
    pub warn_after_secs: i64,
    /// Full intervention threshold (default 7 minutes).
    pub intervene_after_secs: i64,
    /// This much continuous non-distracted activity closes the episode; a
    /// 20-second flip back to work does not (spec §13).
    pub reset_after_secs: i64,
}

impl Default for DistractionConfig {
    fn default() -> Self {
        Self {
            warn_after_secs: 180,
            intervene_after_secs: 420,
            reset_after_secs: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DistractionSignal {
    /// Crossed the warning threshold: show a subtle yellow state.
    Warn { distracted_secs: i64 },
    /// Crossed the intervention threshold: open the accountability prompt.
    Intervene { distracted_secs: i64 },
    /// User said "return to task" and has now actually returned.
    RecoveryComplete { recovery_secs: i64 },
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum EpisodePhase {
    #[default]
    Clean,
    Accumulating,
    Warned,
    Intervened,
    /// Waiting for the user to reach aligned activity after acknowledging.
    Recovering,
}

/// Tracks one rolling distraction episode. Feed it every engine tick.
#[derive(Debug)]
pub struct DistractionTracker {
    config: DistractionConfig,
    phase: EpisodePhase,
    distracted_secs: i64,
    clean_secs: i64,
    last_tick: Option<i64>,
    recovery_started_at: Option<i64>,
    /// When the current episode's FIRST distracted activity began. Idle gaps
    /// preserve an episode without adding to `distracted_secs`, so this is
    /// the only truthful lower bound for "what did this episode flag".
    episode_started_at: Option<i64>,
}

impl DistractionTracker {
    pub fn new(config: DistractionConfig) -> Self {
        Self {
            config,
            phase: EpisodePhase::Clean,
            distracted_secs: 0,
            clean_secs: 0,
            last_tick: None,
            recovery_started_at: None,
            episode_started_at: None,
        }
    }

    pub fn config(&self) -> &DistractionConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: DistractionConfig) {
        self.config = config;
    }

    /// Continuous distracted seconds in the current episode (for UI).
    pub fn current_distracted_secs(&self) -> i64 {
        self.distracted_secs
    }

    /// When the current episode's first distracted activity began.
    pub fn episode_started_at(&self) -> Option<i64> {
        self.episode_started_at
    }

    pub fn is_warned(&self) -> bool {
        matches!(self.phase, EpisodePhase::Warned | EpisodePhase::Intervened)
    }

    /// `suppressed` = on break, monitoring paused, or prompt already open.
    pub fn tick(
        &mut self,
        now: i64,
        classification: Classification,
        suppressed: bool,
    ) -> Option<DistractionSignal> {
        let dt = match self.last_tick {
            Some(prev) => (now - prev).clamp(0, 60),
            None => 0,
        };
        self.last_tick = Some(now);

        if suppressed {
            self.reset();
            return None;
        }

        if self.phase == EpisodePhase::Recovering {
            match classification {
                Classification::Focused | Classification::Supporting => {
                    let started = self.recovery_started_at.unwrap_or(now);
                    self.reset();
                    return Some(DistractionSignal::RecoveryComplete {
                        recovery_secs: (now - started).max(0),
                    });
                }
                _ => return None,
            }
        }

        match classification {
            Classification::Distracted => {
                self.clean_secs = 0;
                self.distracted_secs += dt;
                if self.phase == EpisodePhase::Clean {
                    self.phase = EpisodePhase::Accumulating;
                    self.episode_started_at = Some(now - dt);
                }
                if self.phase == EpisodePhase::Accumulating
                    && self.distracted_secs >= self.config.warn_after_secs
                {
                    self.phase = EpisodePhase::Warned;
                    return Some(DistractionSignal::Warn {
                        distracted_secs: self.distracted_secs,
                    });
                }
                if self.phase == EpisodePhase::Warned
                    && self.distracted_secs >= self.config.intervene_after_secs
                {
                    self.phase = EpisodePhase::Intervened;
                    return Some(DistractionSignal::Intervene {
                        distracted_secs: self.distracted_secs,
                    });
                }
                None
            }
            Classification::Idle => {
                // Idle is not distraction (spec §10) but doesn't clear an
                // episode either — walking away mid-scroll isn't recovery.
                None
            }
            _ => {
                if self.phase == EpisodePhase::Clean {
                    return None;
                }
                self.clean_secs += dt;
                if self.clean_secs >= self.config.reset_after_secs {
                    self.reset();
                }
                None
            }
        }
    }

    /// User answered the intervention with "Return to task": start the
    /// recovery timer (spec §14).
    pub fn begin_recovery(&mut self, now: i64) {
        self.phase = EpisodePhase::Recovering;
        self.recovery_started_at = Some(now);
        self.distracted_secs = 0;
        self.clean_secs = 0;
    }

    /// User answered with anything that legitimizes the activity (actually
    /// work / planned break / priority change / blocked): close the episode.
    pub fn resolve(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        self.phase = EpisodePhase::Clean;
        self.distracted_secs = 0;
        self.clean_secs = 0;
        self.recovery_started_at = None;
        self.episode_started_at = None;
    }
}

// ---------------------------------------------------------------------------
// Check-ins (spec §18)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckinScheduler {
    pub cadence_secs: i64,
    /// Last check-in (or day anchor when none yet).
    pub last_at: i64,
}

impl CheckinScheduler {
    pub fn new(cadence_secs: i64, anchor: i64) -> Self {
        Self {
            cadence_secs,
            last_at: anchor,
        }
    }

    pub fn due(&self, now: i64, in_work_hours: bool, suppressed: bool) -> bool {
        in_work_hours && !suppressed && now - self.last_at >= self.cadence_secs
    }

    pub fn record(&mut self, now: i64) {
        self.last_at = now;
    }

    pub fn next_at(&self) -> i64 {
        self.last_at + self.cadence_secs
    }
}

// ---------------------------------------------------------------------------
// Breaks (spec §17)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakState {
    pub started_at: i64,
    pub ends_at: i64,
}

impl BreakState {
    pub fn start(now: i64, duration_secs: i64) -> Self {
        Self {
            started_at: now,
            ends_at: now + duration_secs.max(60),
        }
    }

    pub fn active(&self, now: i64) -> bool {
        now < self.ends_at
    }

    pub fn remaining_secs(&self, now: i64) -> i64 {
        (self.ends_at - now).max(0)
    }
}

// ---------------------------------------------------------------------------
// Strict mode (spec §28)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StrictPolicy {
    pub enabled: bool,
    /// Interview snoozes allowed per day when strict (unlimited otherwise).
    pub max_interview_snoozes: u32,
}

impl Default for StrictPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_interview_snoozes: 2,
        }
    }
}

impl StrictPolicy {
    pub fn can_snooze_interview(&self, snoozes_used: u32) -> bool {
        !self.enabled || snoozes_used < self.max_interview_snoozes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_hours_normal_and_overnight() {
        assert!(in_work_hours(9 * 60, 8 * 60, 17 * 60));
        assert!(!in_work_hours(18 * 60, 8 * 60, 17 * 60));
        assert!(!in_work_hours(7 * 60 + 59, 8 * 60, 17 * 60));
        // Night shift: 22:00 → 06:00
        assert!(in_work_hours(23 * 60, 22 * 60, 6 * 60));
        assert!(in_work_hours(3 * 60, 22 * 60, 6 * 60));
        assert!(!in_work_hours(12 * 60, 22 * 60, 6 * 60));
    }

    #[test]
    fn commitment_limit_message() {
        assert!(too_many_commitments_message(3).is_none());
        let msg = too_many_commitments_message(6).unwrap();
        assert!(msg.contains("6 priorities means you have no priorities"));
    }

    fn run_ticks(
        tracker: &mut DistractionTracker,
        start: i64,
        secs: i64,
        class: Classification,
    ) -> Vec<DistractionSignal> {
        let mut signals = vec![];
        let mut t = start;
        while t < start + secs {
            t += 5;
            if let Some(s) = tracker.tick(t, class, false) {
                signals.push(s);
            }
        }
        signals
    }

    #[test]
    fn short_distraction_produces_no_signal() {
        let mut tr = DistractionTracker::new(DistractionConfig::default());
        tr.tick(0, Classification::Focused, false);
        let signals = run_ticks(&mut tr, 0, 60, Classification::Distracted);
        assert!(signals.is_empty(), "60s of X is below the 3-minute warning");
    }

    #[test]
    fn warning_then_intervention_at_thresholds() {
        let mut tr = DistractionTracker::new(DistractionConfig::default());
        tr.tick(0, Classification::Focused, false);
        let signals = run_ticks(&mut tr, 0, 480, Classification::Distracted);
        assert_eq!(signals.len(), 2);
        assert!(matches!(signals[0], DistractionSignal::Warn { distracted_secs } if distracted_secs >= 180));
        assert!(matches!(signals[1], DistractionSignal::Intervene { distracted_secs } if distracted_secs >= 420));
    }

    #[test]
    fn brief_return_to_work_does_not_reset_episode() {
        let mut tr = DistractionTracker::new(DistractionConfig::default());
        tr.tick(0, Classification::Focused, false);
        run_ticks(&mut tr, 0, 170, Classification::Distracted); // just under warn
        run_ticks(&mut tr, 170, 20, Classification::Focused); // 20s blip
        let signals = run_ticks(&mut tr, 190, 30, Classification::Distracted);
        assert!(
            signals.iter().any(|s| matches!(s, DistractionSignal::Warn { .. })),
            "episode should persist through a 20s blip"
        );
    }

    #[test]
    fn sustained_work_resets_episode() {
        let mut tr = DistractionTracker::new(DistractionConfig::default());
        tr.tick(0, Classification::Focused, false);
        run_ticks(&mut tr, 0, 170, Classification::Distracted);
        run_ticks(&mut tr, 170, 90, Classification::Focused); // > 60s clean
        let signals = run_ticks(&mut tr, 260, 170, Classification::Distracted);
        assert!(signals.is_empty(), "fresh episode must restart the count");
    }

    #[test]
    fn episode_start_survives_idle_gaps() {
        // 4 min browsing, 10 min idle, 3 more min: the intervention fires,
        // and the episode start must still point at the FIRST distracted
        // minute — idle preserves the episode without moving its start.
        let mut tr = DistractionTracker::new(DistractionConfig::default());
        tr.tick(0, Classification::Focused, false);
        run_ticks(&mut tr, 0, 240, Classification::Distracted);
        let start = tr.episode_started_at().expect("episode began");
        assert!(start <= 5, "start ≈ first distracted tick, got {start}");
        run_ticks(&mut tr, 240, 600, Classification::Idle);
        let signals = run_ticks(&mut tr, 840, 200, Classification::Distracted);
        assert!(
            signals.iter().any(|s| matches!(s, DistractionSignal::Intervene { .. })),
            "4m + 3m of distraction crosses the 7m threshold"
        );
        assert_eq!(tr.episode_started_at(), Some(start), "idle gap must not move the start");
    }

    #[test]
    fn suppressed_ticks_reset_tracking() {
        let mut tr = DistractionTracker::new(DistractionConfig::default());
        tr.tick(0, Classification::Focused, false);
        run_ticks(&mut tr, 0, 170, Classification::Distracted);
        tr.tick(175, Classification::Distracted, true); // break started
        assert_eq!(tr.current_distracted_secs(), 0);
    }

    #[test]
    fn recovery_measured_from_acknowledgement_to_aligned_activity() {
        let mut tr = DistractionTracker::new(DistractionConfig::default());
        tr.tick(0, Classification::Focused, false);
        run_ticks(&mut tr, 0, 480, Classification::Distracted); // intervention fired
        tr.begin_recovery(500); // user clicked "Return to task"
        // Still dawdling for 2 minutes...
        assert!(tr.tick(560, Classification::Distracted, false).is_none());
        let signal = tr.tick(634, Classification::Focused, false).unwrap();
        assert_eq!(signal, DistractionSignal::RecoveryComplete { recovery_secs: 134 });
    }

    #[test]
    fn checkin_due_only_in_work_hours_and_after_cadence() {
        let mut s = CheckinScheduler::new(90 * 60, 0);
        assert!(!s.due(60 * 60, true, false), "only 60m elapsed");
        assert!(s.due(90 * 60, true, false));
        assert!(!s.due(90 * 60, false, false), "outside work hours");
        assert!(!s.due(90 * 60, true, true), "suppressed during break");
        s.record(90 * 60);
        assert!(!s.due(120 * 60, true, false));
        assert!(s.due(180 * 60, true, false));
    }

    #[test]
    fn break_state_lifecycle() {
        let b = BreakState::start(1000, 600);
        assert!(b.active(1300));
        assert_eq!(b.remaining_secs(1300), 300);
        assert!(!b.active(1600));
        assert_eq!(b.remaining_secs(1700), 0);
    }

    #[test]
    fn strict_mode_limits_snoozes() {
        let lax = StrictPolicy { enabled: false, max_interview_snoozes: 2 };
        assert!(lax.can_snooze_interview(99));
        let strict = StrictPolicy { enabled: true, max_interview_snoozes: 2 };
        assert!(strict.can_snooze_interview(1));
        assert!(!strict.can_snooze_interview(2));
    }

    #[test]
    fn switch_reason_required() {
        assert!(validate_switch_reason("").is_err());
        assert!(validate_switch_reason("  ").is_err());
        assert!(validate_switch_reason("Client escalation").is_ok());
    }
}
