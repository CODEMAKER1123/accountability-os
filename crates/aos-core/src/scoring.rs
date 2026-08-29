//! Score calculations (spec §19–20). Every score exposes its components —
//! the purpose is behavioral feedback, so the math is never hidden.

use serde::{Deserialize, Serialize};

use crate::types::{Classification, Priority};

/// Aggregated seconds per classification for a period.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct DayTotals {
    pub focused_secs: i64,
    pub supporting_secs: i64,
    pub neutral_secs: i64,
    pub distracted_secs: i64,
    pub idle_secs: i64,
    pub unknown_secs: i64,
}

impl DayTotals {
    pub fn add(&mut self, classification: Classification, secs: i64) {
        let secs = secs.max(0);
        match classification {
            Classification::Focused => self.focused_secs += secs,
            Classification::Supporting => self.supporting_secs += secs,
            Classification::Neutral => self.neutral_secs += secs,
            Classification::Distracted => self.distracted_secs += secs,
            Classification::Idle => self.idle_secs += secs,
            Classification::Unknown => self.unknown_secs += secs,
        }
    }

    /// Classified working time: the denominator for alignment and focus
    /// quality (spec §19). Idle is excluded per the spec; Unknown is excluded
    /// because unclassified time must not penalize the user (spec §12) — it
    /// neither helps nor hurts until classified.
    pub fn non_idle_secs(&self) -> i64 {
        self.focused_secs + self.supporting_secs + self.neutral_secs + self.distracted_secs
    }
}

/// Commitment Alignment (spec §19): share of working time that contributed
/// to declared commitments. `(focused + supporting*0.7) / non_idle`, 0–100.
pub fn commitment_alignment(t: &DayTotals) -> Option<f64> {
    let denom = t.non_idle_secs() as f64;
    if denom <= 0.0 {
        return None;
    }
    let num = t.focused_secs as f64 + t.supporting_secs as f64 * 0.7;
    Some((num / denom * 100.0).clamp(0.0, 100.0))
}

/// Focus Score (spec §19): quality of attention while working. Uses the §10
/// weights (neutral counts a little — necessary admin isn't distraction),
/// minus a context-switching penalty above 6 switches/hour.
pub fn focus_score(t: &DayTotals, context_switches: u32) -> Option<f64> {
    let denom = t.non_idle_secs() as f64;
    if denom <= 0.0 {
        return None;
    }
    let weighted = t.focused_secs as f64 * 1.0
        + t.supporting_secs as f64 * 0.7
        + t.neutral_secs as f64 * 0.25;
    let base = weighted / denom * 100.0;
    let hours = denom / 3600.0;
    let rate = if hours > 0.0 { context_switches as f64 / hours } else { 0.0 };
    let penalty = ((rate - 6.0) * 2.0).clamp(0.0, 30.0);
    Some((base - penalty).clamp(0.0, 100.0))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CommitmentOutcome {
    pub priority: Priority,
    pub completed: bool,
}

/// Execution Score (spec §19): priority-weighted completion percentage.
pub fn execution_score(outcomes: &[CommitmentOutcome]) -> Option<f64> {
    if outcomes.is_empty() {
        return None;
    }
    let total: f64 = outcomes.iter().map(|o| o.priority.weight()).sum();
    let done: f64 = outcomes
        .iter()
        .filter(|o| o.completed)
        .map(|o| o.priority.weight())
        .sum();
    Some((done / total * 100.0).clamp(0.0, 100.0))
}

/// Planning Accuracy (spec §19): symmetric error between estimated and
/// actual focused time. 100 = perfect estimate; overshooting by 2x and
/// working half the estimate score the same.
pub fn planning_accuracy(estimated_secs: i64, actual_focused_secs: i64) -> Option<f64> {
    if estimated_secs <= 0 {
        return None;
    }
    let est = estimated_secs as f64;
    let act = actual_focused_secs.max(0) as f64;
    let denom = est.max(act);
    if denom <= 0.0 {
        return None;
    }
    Some(((1.0 - (est - act).abs() / denom) * 100.0).clamp(0.0, 100.0))
}

/// Daily Execution Score (spec §20): 40% completion, 30% alignment,
/// 20% focus quality, 10% planning accuracy. Missing components (e.g. no
/// estimates given) redistribute their weight instead of scoring zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyScore {
    pub total: f64,
    pub completion: Option<f64>,
    pub alignment: Option<f64>,
    pub focus_quality: Option<f64>,
    pub planning_accuracy: Option<f64>,
}

pub fn daily_score(
    completion: Option<f64>,
    alignment: Option<f64>,
    focus_quality: Option<f64>,
    planning: Option<f64>,
) -> Option<DailyScore> {
    let parts: [(Option<f64>, f64); 4] = [
        (completion, 0.4),
        (alignment, 0.3),
        (focus_quality, 0.2),
        (planning, 0.1),
    ];
    let available_weight: f64 = parts.iter().filter(|(v, _)| v.is_some()).map(|(_, w)| w).sum();
    if available_weight <= 0.0 {
        return None;
    }
    let total: f64 = parts
        .iter()
        .filter_map(|(v, w)| v.map(|v| v * w))
        .sum::<f64>()
        / available_weight;
    Some(DailyScore {
        total: total.clamp(0.0, 100.0),
        completion,
        alignment,
        focus_quality,
        planning_accuracy: planning,
    })
}

/// Distraction stats (spec §19).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DistractionStats {
    pub total_secs: i64,
    pub session_count: u32,
    pub longest_secs: i64,
    /// (app or domain, seconds), descending.
    pub top_sources: Vec<(String, i64)>,
    pub avg_recovery_secs: Option<i64>,
}

pub fn distraction_stats<'a>(
    sessions: impl Iterator<Item = (&'a str, Classification, i64)>,
    recovery_secs: &[i64],
) -> DistractionStats {
    let mut stats = DistractionStats::default();
    let mut by_source: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for (source, class, secs) in sessions {
        if class == Classification::Distracted {
            stats.total_secs += secs;
            stats.session_count += 1;
            stats.longest_secs = stats.longest_secs.max(secs);
            *by_source.entry(source.to_string()).or_default() += secs;
        }
    }
    let mut top: Vec<(String, i64)> = by_source.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(5);
    stats.top_sources = top;
    if !recovery_secs.is_empty() {
        stats.avg_recovery_secs =
            Some(recovery_secs.iter().sum::<i64>() / recovery_secs.len() as i64);
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    fn totals(f: i64, s: i64, n: i64, d: i64, i: i64) -> DayTotals {
        DayTotals {
            focused_secs: f * 60,
            supporting_secs: s * 60,
            neutral_secs: n * 60,
            distracted_secs: d * 60,
            idle_secs: i * 60,
            unknown_secs: 0,
        }
    }

    #[test]
    fn alignment_follows_spec_formula() {
        // 60m focused, 30m supporting, 10m neutral, 20m distracted, 15m idle.
        let t = totals(60, 30, 10, 20, 15);
        // (60 + 21) / 120 = 67.5%; idle excluded from denominator.
        let a = commitment_alignment(&t).unwrap();
        assert!((a - 67.5).abs() < 1e-9, "got {a}");
    }

    #[test]
    fn alignment_is_none_with_no_work_time() {
        assert!(commitment_alignment(&totals(0, 0, 0, 0, 60)).is_none());
    }

    #[test]
    fn unknown_time_neither_helps_nor_hurts() {
        // 60m focused + 60m unknown (AI off / low confidence): the unknown
        // hour must not drag alignment or focus down (spec §12).
        let mut t = totals(60, 0, 0, 0, 0);
        t.unknown_secs = 3600;
        assert!((commitment_alignment(&t).unwrap() - 100.0).abs() < 1e-9);
        assert!((focus_score(&t, 0).unwrap() - 100.0).abs() < 1e-9);
        // An all-unknown day scores None, not zero.
        let mut only_unknown = totals(0, 0, 0, 0, 0);
        only_unknown.unknown_secs = 3600;
        assert!(commitment_alignment(&only_unknown).is_none());
    }

    #[test]
    fn perfect_day_scores_100_alignment() {
        let a = commitment_alignment(&totals(120, 0, 0, 0, 0)).unwrap();
        assert!((a - 100.0).abs() < 1e-9);
    }

    #[test]
    fn focus_score_penalizes_context_switching() {
        let t = totals(120, 0, 0, 0, 0); // two pure hours
        let calm = focus_score(&t, 4).unwrap(); // 2/hr — no penalty
        let frantic = focus_score(&t, 40).unwrap(); // 20/hr — penalized
        assert!((calm - 100.0).abs() < 1e-9);
        assert!(frantic < calm);
        assert!((calm - frantic - 28.0).abs() < 1e-9, "(20-6)*2 = 28 point penalty");
    }

    #[test]
    fn execution_score_weights_priorities() {
        // must done, should done, could missed: (3+2)/(3+2+1) = 83.33
        let outcomes = [
            CommitmentOutcome { priority: Priority::Must, completed: true },
            CommitmentOutcome { priority: Priority::Should, completed: true },
            CommitmentOutcome { priority: Priority::Could, completed: false },
        ];
        let s = execution_score(&outcomes).unwrap();
        assert!((s - 83.33333).abs() < 0.001, "got {s}");
        // Missing the must hurts more than missing the could.
        let miss_must = [
            CommitmentOutcome { priority: Priority::Must, completed: false },
            CommitmentOutcome { priority: Priority::Should, completed: true },
            CommitmentOutcome { priority: Priority::Could, completed: true },
        ];
        assert!(execution_score(&miss_must).unwrap() < s);
    }

    #[test]
    fn planning_accuracy_is_symmetric() {
        // Estimated 90m, worked 45m → 50%. Estimated 45m, worked 90m → 50%.
        let under = planning_accuracy(90 * 60, 45 * 60).unwrap();
        let over = planning_accuracy(45 * 60, 90 * 60).unwrap();
        assert!((under - 50.0).abs() < 1e-9);
        assert!((over - 50.0).abs() < 1e-9);
        assert!((planning_accuracy(60, 60).unwrap() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn daily_score_uses_spec_weights() {
        let s = daily_score(Some(100.0), Some(80.0), Some(50.0), Some(60.0)).unwrap();
        // 100*.4 + 80*.3 + 50*.2 + 60*.1 = 40+24+10+6 = 80
        assert!((s.total - 80.0).abs() < 1e-9, "got {}", s.total);
    }

    #[test]
    fn daily_score_redistributes_missing_components() {
        // No planning estimates given: remaining 0.9 weight renormalized.
        let s = daily_score(Some(100.0), Some(100.0), Some(100.0), None).unwrap();
        assert!((s.total - 100.0).abs() < 1e-9, "missing component must not cap the score");
        assert!(daily_score(None, None, None, None).is_none());
    }

    #[test]
    fn distraction_stats_ranks_sources() {
        let sessions = [
            ("x.com", Classification::Distracted, 420),
            ("chrome.exe", Classification::Focused, 3000),
            ("reddit.com", Classification::Distracted, 900),
            ("x.com", Classification::Distracted, 300),
        ];
        let stats = distraction_stats(
            sessions.iter().map(|(s, c, d)| (*s, *c, *d)),
            &[134, 66],
        );
        assert_eq!(stats.total_secs, 1620);
        assert_eq!(stats.session_count, 3);
        assert_eq!(stats.longest_secs, 900);
        assert_eq!(stats.top_sources[0], ("reddit.com".into(), 900));
        assert_eq!(stats.top_sources[1], ("x.com".into(), 720));
        assert_eq!(stats.avg_recovery_secs, Some(100));
    }
}
