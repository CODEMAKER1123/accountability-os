//! Long-term pattern detection (spec §23): pure functions over historical
//! facts. The app layer feeds these from SQLite; AI narrates on top of them
//! but never invents numbers.

use serde::{Deserialize, Serialize};

use crate::types::Classification;

/// One historical session reduced to what analysis needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFact {
    pub started_at: i64,
    pub duration_secs: i64,
    pub classification: Classification,
    /// App name, or browser domain for browser sessions.
    pub source_label: String,
    /// Local hour of day the session started (0–23); computed by the caller
    /// so this crate stays timezone-free.
    pub local_hour: u8,
    /// Seconds already elapsed within `local_hour` when the session started
    /// (0–3599); lets the hourly profile apportion long sessions across the
    /// hours they actually span. Callers with only coarse data may pass 0.
    #[serde(default)]
    pub secs_into_hour: u32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct HourBucket {
    pub focused_secs: i64,
    pub distracted_secs: i64,
    pub total_secs: i64,
}

/// Focused/distracted seconds per local hour of day. A session spanning
/// hour boundaries is apportioned across every hour it overlaps — a
/// 09:50–11:50 focus block credits 10 min to 9:00, an hour to 10:00 and
/// 50 min to 11:00, not two hours to 9:00.
pub fn hourly_profile(facts: &[SessionFact]) -> [HourBucket; 24] {
    let mut hours = [HourBucket::default(); 24];
    for f in facts {
        let mut hour = (f.local_hour as usize).min(23);
        let mut remaining = f.duration_secs.max(0);
        let mut room_in_hour = (3600 - (f.secs_into_hour as i64).min(3599)).max(1);
        while remaining > 0 {
            let chunk = remaining.min(room_in_hour);
            let b = &mut hours[hour];
            b.total_secs += chunk;
            match f.classification {
                Classification::Focused => b.focused_secs += chunk,
                Classification::Distracted => b.distracted_secs += chunk,
                _ => {}
            }
            remaining -= chunk;
            hour = (hour + 1) % 24;
            room_in_hour = 3600;
        }
    }
    hours
}

/// Hour with the highest focused share, requiring a minimum sample so one
/// good Tuesday doesn't become "your most productive hour".
pub fn most_productive_hour(profile: &[HourBucket; 24], min_total_secs: i64) -> Option<u8> {
    profile
        .iter()
        .enumerate()
        .filter(|(_, b)| b.total_secs >= min_total_secs)
        .max_by(|(_, a), (_, b)| {
            let ra = a.focused_secs as f64 / a.total_secs as f64;
            let rb = b.focused_secs as f64 / b.total_secs as f64;
            ra.partial_cmp(&rb).unwrap()
        })
        .map(|(h, _)| h as u8)
}

/// Hour with the most total distracted time.
pub fn peak_distraction_hour(profile: &[HourBucket; 24]) -> Option<u8> {
    profile
        .iter()
        .enumerate()
        .filter(|(_, b)| b.distracted_secs > 0)
        .max_by_key(|(_, b)| b.distracted_secs)
        .map(|(h, _)| h as u8)
}

/// Top time sinks for one classification, descending seconds.
pub fn top_sources(
    facts: &[SessionFact],
    classification: Classification,
    n: usize,
) -> Vec<(String, i64)> {
    let mut by_source: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for f in facts {
        if f.classification == classification {
            *by_source.entry(f.source_label.as_str()).or_default() += f.duration_secs;
        }
    }
    let mut v: Vec<(String, i64)> = by_source
        .into_iter()
        .map(|(k, s)| (k.to_string(), s))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

/// Number of transitions between different sources among CLASSIFIED work
/// sessions. Idle is not work; Unknown must neither help nor hurt (spec §12),
/// so neither participates in the switch count that feeds the focus-score
/// penalty. Facts must be in chronological order.
pub fn context_switches(facts: &[SessionFact]) -> u32 {
    let mut switches = 0;
    let mut prev: Option<&str> = None;
    for f in facts {
        if matches!(f.classification, Classification::Idle | Classification::Unknown) {
            continue;
        }
        if let Some(p) = prev {
            if p != f.source_label {
                switches += 1;
            }
        }
        prev = Some(&f.source_label);
    }
    switches
}

/// Contiguous focused/supporting spans of at least `min_secs`. A gap of up
/// to `max_gap_secs` of anything else doesn't break the block.
pub fn deep_work_blocks(facts: &[SessionFact], min_secs: i64, max_gap_secs: i64) -> Vec<(i64, i64)> {
    let mut blocks = vec![];
    let mut cur: Option<(i64, i64)> = None; // (start, end)
    for f in facts {
        let aligned = matches!(
            f.classification,
            Classification::Focused | Classification::Supporting
        );
        let end = f.started_at + f.duration_secs;
        match (&mut cur, aligned) {
            (None, true) => cur = Some((f.started_at, end)),
            (Some((_, cend)), true) if f.started_at - *cend <= max_gap_secs => *cend = end,
            (Some((cstart, cend)), true) => {
                if *cend - *cstart >= min_secs {
                    blocks.push((*cstart, *cend));
                }
                cur = Some((f.started_at, end));
            }
            (Some((cstart, cend)), false) if f.started_at - *cend > max_gap_secs || f.duration_secs > max_gap_secs => {
                if *cend - *cstart >= min_secs {
                    blocks.push((*cstart, *cend));
                }
                cur = None;
            }
            _ => {}
        }
    }
    if let Some((cstart, cend)) = cur {
        if cend - cstart >= min_secs {
            blocks.push((cstart, cend));
        }
    }
    blocks
}

/// Average actual/estimated ratio across commitments that had estimates.
/// > 1.0 = the user underestimates (plans 60m, needs 80m).
pub fn estimation_bias(pairs: &[(i64, i64)]) -> Option<f64> {
    let valid: Vec<f64> = pairs
        .iter()
        .filter(|(est, _)| *est > 0)
        .map(|(est, act)| *act as f64 / *est as f64)
        .collect();
    if valid.is_empty() {
        return None;
    }
    Some(valid.iter().sum::<f64>() / valid.len() as f64)
}

/// Completion rate split by commitment start hour: (before_cutoff, after).
pub fn completion_by_start(
    commitments: &[(u8, bool)],
    cutoff_hour: u8,
) -> (Option<f64>, Option<f64>) {
    let rate = |items: Vec<&(u8, bool)>| {
        if items.is_empty() {
            None
        } else {
            Some(items.iter().filter(|(_, done)| *done).count() as f64 / items.len() as f64 * 100.0)
        }
    };
    let before: Vec<_> = commitments.iter().filter(|(h, _)| *h < cutoff_hour).collect();
    let after: Vec<_> = commitments.iter().filter(|(h, _)| *h >= cutoff_hour).collect();
    (rate(before), rate(after))
}

/// A deterministic insight with the numbers that produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub metric: String,
    pub text: String,
}

/// Deterministic insight sentences (spec §23 examples). Used directly when
/// AI is off and as grounding context for the AI coach.
pub fn generate_insights(
    facts: &[SessionFact],
    commitment_starts: &[(u8, bool)],
    estimate_pairs: &[(i64, i64)],
) -> Vec<Insight> {
    let mut insights = vec![];
    let profile = hourly_profile(facts);

    if let Some(h) = most_productive_hour(&profile, 1800) {
        insights.push(Insight {
            metric: "most_productive_hour".into(),
            text: format!("Your highest focus ratio is around {}:00.", h),
        });
    }
    if let Some(h) = peak_distraction_hour(&profile) {
        let mins = profile[h as usize].distracted_secs / 60;
        if mins >= 15 {
            insights.push(Insight {
                metric: "peak_distraction_hour".into(),
                text: format!(
                    "Your most common distraction window starts around {}:00 ({} min of distraction recorded there).",
                    h, mins
                ),
            });
        }
    }
    let top = top_sources(facts, Classification::Distracted, 1);
    if let Some((source, secs)) = top.first() {
        if *secs >= 1800 {
            insights.push(Insight {
                metric: "top_distraction".into(),
                text: format!("{} is your largest distraction: {} min in this period.", source, secs / 60),
            });
        }
    }
    if let Some(bias) = estimation_bias(estimate_pairs) {
        if bias > 1.25 {
            insights.push(Insight {
                metric: "underplanning".into(),
                text: format!(
                    "You consistently need about {:.0}% more focused time than you estimate.",
                    (bias - 1.0) * 100.0
                ),
            });
        } else if bias < 0.75 && bias > 0.0 {
            insights.push(Insight {
                metric: "overplanning".into(),
                text: format!(
                    "You typically finish in about {:.0}% of the time you reserve. Your estimates run high.",
                    bias * 100.0
                ),
            });
        }
    }
    let (before, after) = completion_by_start(commitment_starts, 12);
    if let (Some(b), Some(a)) = (before, after) {
        if b - a >= 15.0 {
            insights.push(Insight {
                metric: "morning_completion_edge".into(),
                text: format!(
                    "You complete {:.0}% of commitments started before noon but only {:.0}% of those started later.",
                    b, a
                ),
            });
        }
    }
    insights
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(start: i64, dur: i64, class: Classification, label: &str, hour: u8) -> SessionFact {
        SessionFact {
            started_at: start,
            duration_secs: dur,
            classification: class,
            source_label: label.into(),
            local_hour: hour,
            secs_into_hour: 0,
        }
    }

    #[test]
    fn hourly_profile_apportions_across_hour_boundaries() {
        // Focus block 09:50–11:50: 10m to 9:00, 60m to 10:00, 50m to 11:00.
        let mut f = fact(0, 7200, Classification::Focused, "docs", 9);
        f.secs_into_hour = 50 * 60;
        let p = hourly_profile(&[f]);
        assert_eq!(p[9].focused_secs, 600);
        assert_eq!(p[10].focused_secs, 3600);
        assert_eq!(p[11].focused_secs, 3000);
    }

    #[test]
    fn context_switches_ignore_unknown_sessions() {
        let facts = vec![
            fact(0, 600, Classification::Focused, "docs", 9),
            fact(600, 300, Classification::Unknown, "mystery", 9),
            fact(900, 600, Classification::Focused, "docs", 9),
        ];
        assert_eq!(context_switches(&facts), 0, "unknown must neither help nor hurt");
    }

    #[test]
    fn hourly_profile_buckets_by_hour() {
        let facts = vec![
            fact(0, 1800, Classification::Focused, "docs", 9),
            fact(1800, 600, Classification::Distracted, "x.com", 9),
            fact(2400, 3600, Classification::Focused, "docs", 14),
        ];
        let p = hourly_profile(&facts);
        assert_eq!(p[9].focused_secs, 1800);
        assert_eq!(p[9].distracted_secs, 600);
        assert_eq!(p[14].focused_secs, 3600);
    }

    #[test]
    fn hourly_profile_splits_sessions_across_hour_boundaries() {
        let mut spanning = fact(0, 30 * 60, Classification::Focused, "docs", 9);
        spanning.secs_into_hour = 45 * 60;
        let profile = hourly_profile(&[spanning]);
        assert_eq!(profile[9].focused_secs, 15 * 60);
        assert_eq!(profile[10].focused_secs, 15 * 60);
        assert_eq!(profile.iter().map(|bucket| bucket.total_secs).sum::<i64>(), 30 * 60);
    }

    #[test]
    fn most_productive_hour_requires_sample() {
        let facts = vec![
            fact(0, 60, Classification::Focused, "docs", 6), // tiny perfect sample
            fact(100, 3600, Classification::Focused, "docs", 9),
            fact(4000, 1200, Classification::Distracted, "x.com", 9),
        ];
        let p = hourly_profile(&facts);
        assert_eq!(most_productive_hour(&p, 1800), Some(9), "6:00 lacks sample");
    }

    #[test]
    fn context_switches_ignore_idle_and_same_source() {
        let facts = vec![
            fact(0, 600, Classification::Focused, "docs", 9),
            fact(600, 120, Classification::Idle, "Idle", 9),
            fact(720, 600, Classification::Focused, "docs", 9), // same source after idle
            fact(1320, 300, Classification::Supporting, "outlook", 9),
            fact(1620, 300, Classification::Focused, "docs", 9),
        ];
        assert_eq!(context_switches(&facts), 2);
    }

    #[test]
    fn deep_work_blocks_bridge_small_gaps() {
        let facts = vec![
            fact(0, 1200, Classification::Focused, "docs", 9),
            fact(1200, 60, Classification::Neutral, "explorer", 9), // 60s blip
            fact(1260, 1500, Classification::Focused, "docs", 9),
            fact(2760, 900, Classification::Distracted, "x.com", 9),
            fact(3660, 600, Classification::Focused, "docs", 10), // too short alone
        ];
        let blocks = deep_work_blocks(&facts, 1500, 120);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0], (0, 2760), "blip bridged into one deep block");
    }

    #[test]
    fn estimation_bias_detects_underplanning() {
        // planned 60m spent 90m, planned 30m spent 45m → bias 1.5
        let bias = estimation_bias(&[(3600, 5400), (1800, 2700)]).unwrap();
        assert!((bias - 1.5).abs() < 1e-9);
        assert!(estimation_bias(&[(0, 100)]).is_none());
    }

    #[test]
    fn completion_split_by_start_hour() {
        let commitments = vec![(9, true), (10, true), (11, false), (14, false), (15, false), (16, true)];
        let (before, after) = completion_by_start(&commitments, 12);
        assert!((before.unwrap() - 66.66666).abs() < 0.001);
        assert!((after.unwrap() - 33.33333).abs() < 0.001);
    }

    #[test]
    fn insights_generated_from_real_numbers_only() {
        let facts = vec![
            fact(0, 3600, Classification::Focused, "docs", 9),
            fact(3600, 2400, Classification::Distracted, "x.com", 14),
        ];
        let insights = generate_insights(&facts, &[], &[]);
        assert!(insights.iter().any(|i| i.metric == "top_distraction"));
        assert!(insights.iter().any(|i| i.metric == "peak_distraction_hour"));
        // No estimates → no planning insight fabricated.
        assert!(!insights.iter().any(|i| i.metric == "underplanning"));
    }
}
