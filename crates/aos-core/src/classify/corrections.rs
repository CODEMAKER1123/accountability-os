//! Layer 2: historical user corrections (spec §11, §42). When the user has
//! already told us what an activity is, believe them before anything else.

use serde::{Deserialize, Serialize};

use crate::aggregator::normalize_title;
use crate::types::{ActivityContext, Classification, ClassificationSource, ClassifyOutcome};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correction {
    pub id: i64,
    pub process_name: String,
    pub browser_domain: Option<String>,
    pub normalized_title: String,
    /// Context in which the correction was made.
    pub commitment_id: Option<i64>,
    pub project_id: Option<i64>,
    pub classification: Classification,
}

#[derive(Debug, Clone, Default)]
pub struct CorrectionMatcher {
    pub corrections: Vec<Correction>,
}

/// How precisely a stored correction matches the current activity.
/// Confidence degrades as the match gets less specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchLevel {
    DomainInCommitment, // same domain, same commitment, different page
    TitleAnywhere,      // same domain/app + same title, any context
    TitleInProject,     // same domain/app + same title, same project
    TitleInCommitment,  // same domain/app + same title, same commitment
}

impl MatchLevel {
    fn confidence(self) -> f64 {
        match self {
            MatchLevel::TitleInCommitment => 0.95,
            MatchLevel::TitleInProject => 0.9,
            MatchLevel::TitleAnywhere => 0.85,
            MatchLevel::DomainInCommitment => 0.72,
        }
    }
}

impl CorrectionMatcher {
    /// Exact-title corrections; strongest user signal, beats rules.
    pub fn title_match(&self, ctx: &ActivityContext) -> Option<ClassifyOutcome> {
        self.best(ctx, true)
    }

    /// Weaker domain-level memory, consulted after rules.
    pub fn domain_match(&self, ctx: &ActivityContext) -> Option<ClassifyOutcome> {
        self.best(ctx, false)
    }

    fn best(&self, ctx: &ActivityContext, title_only: bool) -> Option<ClassifyOutcome> {
        let ctx_title = normalize_title(&ctx.window_title);
        let ctx_domain = ctx.browser_domain.as_deref().map(str::to_lowercase);

        let mut best: Option<(MatchLevel, &Correction)> = None;
        for c in &self.corrections {
            let level = match_level(c, ctx, &ctx_title, ctx_domain.as_deref());
            let Some(level) = level else { continue };
            let is_title = level >= MatchLevel::TitleAnywhere;
            if title_only != is_title {
                continue;
            }
            if best.as_ref().is_none_or(|(l, _)| level > *l) {
                best = Some((level, c));
            }
        }
        best.map(|(level, c)| ClassifyOutcome {
            classification: c.classification,
            confidence: level.confidence(),
            source: ClassificationSource::Correction,
            reason: "Matches a previous manual correction".into(),
        })
    }
}

fn match_level(
    c: &Correction,
    ctx: &ActivityContext,
    ctx_title: &str,
    ctx_domain: Option<&str>,
) -> Option<MatchLevel> {
    let same_surface = match (&c.browser_domain, ctx_domain) {
        // Browser activity: the domain is the identity.
        (Some(cd), Some(xd)) => cd.eq_ignore_ascii_case(xd),
        (None, None) => c.process_name.eq_ignore_ascii_case(&ctx.process_name),
        _ => false,
    };
    if !same_surface {
        return None;
    }

    let same_title = !ctx_title.is_empty() && c.normalized_title == ctx_title;
    let same_commitment = c.commitment_id.is_some() && c.commitment_id == ctx.commitment_id;
    let same_project = c.project_id.is_some() && c.project_id == ctx.project_id;

    if same_title && same_commitment {
        Some(MatchLevel::TitleInCommitment)
    } else if same_title && same_project {
        Some(MatchLevel::TitleInProject)
    } else if same_title {
        Some(MatchLevel::TitleAnywhere)
    } else if same_commitment && c.browser_domain.is_some() {
        Some(MatchLevel::DomainInCommitment)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correction(
        id: i64,
        domain: Option<&str>,
        title: &str,
        commitment: Option<i64>,
        class: Classification,
    ) -> Correction {
        Correction {
            id,
            process_name: "chrome.exe".into(),
            browser_domain: domain.map(String::from),
            normalized_title: normalize_title(title),
            commitment_id: commitment,
            project_id: None,
            classification: class,
        }
    }

    fn ctx(domain: Option<&str>, title: &str, commitment: Option<i64>) -> ActivityContext {
        ActivityContext {
            app_name: "Chrome".into(),
            process_name: "chrome.exe".into(),
            window_title: title.into(),
            browser_domain: domain.map(String::from),
            browser_title: None,
            commitment_id: commitment,
            project_id: None,
            in_focus_session: true,
            is_idle: false,
        }
    }

    #[test]
    fn exact_title_correction_matches() {
        let m = CorrectionMatcher {
            corrections: vec![correction(
                1,
                Some("docs.google.com"),
                "Commercial Sales Playbook - Google Docs",
                Some(7),
                Classification::Focused,
            )],
        };
        let out = m
            .title_match(&ctx(
                Some("docs.google.com"),
                "Commercial Sales Playbook - Google Docs",
                Some(7),
            ))
            .expect("exact repeat of corrected activity");
        assert_eq!(out.classification, Classification::Focused);
        assert!(out.confidence > 0.9);
    }

    #[test]
    fn different_domain_does_not_match() {
        let m = CorrectionMatcher {
            corrections: vec![correction(1, Some("docs.google.com"), "Playbook", Some(7), Classification::Focused)],
        };
        assert!(m.title_match(&ctx(Some("sheets.google.com"), "Playbook", Some(7))).is_none());
    }

    #[test]
    fn domain_level_memory_is_weaker_and_scoped_to_commitment() {
        let m = CorrectionMatcher {
            corrections: vec![correction(
                1,
                Some("youtube.com"),
                "Tauri tutorial - YouTube",
                Some(7),
                Classification::Focused,
            )],
        };
        // Different video, same commitment: weak domain match only.
        let ctx2 = ctx(Some("youtube.com"), "Some other video - YouTube", Some(7));
        assert!(m.title_match(&ctx2).is_none());
        let out = m.domain_match(&ctx2).expect("domain memory within commitment");
        assert!(out.confidence < 0.8);
        // Different commitment: no match at all.
        let ctx3 = ctx(Some("youtube.com"), "Some other video - YouTube", Some(9));
        assert!(m.domain_match(&ctx3).is_none());
    }

    #[test]
    fn most_specific_correction_wins() {
        let m = CorrectionMatcher {
            corrections: vec![
                correction(1, Some("youtube.com"), "Tauri tutorial - YouTube", None, Classification::Distracted),
                correction(2, Some("youtube.com"), "Tauri tutorial - YouTube", Some(7), Classification::Focused),
            ],
        };
        let out = m
            .title_match(&ctx(Some("youtube.com"), "Tauri tutorial - YouTube", Some(7)))
            .unwrap();
        assert_eq!(out.classification, Classification::Focused);
    }
}
