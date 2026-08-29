//! The full classification pipeline: rules first, cache second, AI third
//! (spec §36) — with exact user corrections outranking everything except a
//! manual override, since they are the user's own words about their own work.

use crate::types::{ActivityContext, Classification, ClassificationSource, ClassifyOutcome};

use super::corrections::CorrectionMatcher;
use super::rules::RulesEngine;

/// Below this confidence an AI classification is stored as Unknown and the
/// user is asked later instead of being penalized (spec §12).
pub const AI_CONFIDENCE_THRESHOLD: f64 = 0.65;

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineResult {
    Decided(ClassifyOutcome),
    /// Deterministic layers could not decide; ask the AI (or fall back to
    /// Unknown when AI is unavailable).
    NeedsAi,
}

#[derive(Debug, Default)]
pub struct ClassificationPipeline {
    pub rules: RulesEngine,
    pub corrections: CorrectionMatcher,
    /// Process names the user marked private (spec §52).
    pub private_processes: Vec<String>,
}

impl ClassificationPipeline {
    pub fn evaluate(&self, ctx: &ActivityContext) -> PipelineResult {
        if ctx.is_idle {
            return decided(Classification::Idle, 1.0, ClassificationSource::Default, "No user input");
        }

        if self.is_private(&ctx.process_name) {
            return decided(
                Classification::Neutral,
                1.0,
                ClassificationSource::Rule,
                "Private application — details not recorded",
            );
        }

        // Exact-title corrections: the user already told us what this is.
        if let Some(out) = self.corrections.title_match(ctx) {
            return PipelineResult::Decided(out);
        }

        if let Some(out) = self.rules.evaluate(ctx) {
            return PipelineResult::Decided(out);
        }

        // Weaker domain-level correction memory.
        if let Some(out) = self.corrections.domain_match(ctx) {
            return PipelineResult::Decided(out);
        }

        // Without an active commitment there is nothing to align against:
        // everything non-idle defaults to Neutral rather than punishing the
        // user for time they never committed (spec §2).
        if ctx.commitment_id.is_none() {
            return decided(
                Classification::Neutral,
                0.7,
                ClassificationSource::Default,
                "No active commitment to align against",
            );
        }

        PipelineResult::NeedsAi
    }

    pub fn is_private(&self, process_name: &str) -> bool {
        process_name == crate::types::PRIVATE_PROCESS_SENTINEL
            || self
                .private_processes
                .iter()
                .any(|p| p.eq_ignore_ascii_case(process_name))
    }

    /// Post-process an AI answer: low confidence becomes Unknown (spec §12).
    pub fn resolve_ai(classification: Classification, confidence: f64, reason: String) -> ClassifyOutcome {
        if confidence < AI_CONFIDENCE_THRESHOLD {
            ClassifyOutcome {
                classification: Classification::Unknown,
                confidence,
                source: ClassificationSource::Ai,
                reason: format!("Low confidence ({confidence:.2}): {reason}"),
            }
        } else {
            ClassifyOutcome {
                classification,
                confidence,
                source: ClassificationSource::Ai,
                reason,
            }
        }
    }
}

fn decided(
    classification: Classification,
    confidence: f64,
    source: ClassificationSource,
    reason: &str,
) -> PipelineResult {
    PipelineResult::Decided(ClassifyOutcome {
        classification,
        confidence,
        source,
        reason: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::corrections::Correction;
    use crate::classify::rules::DomainRule;

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

    fn pipeline_with_block() -> ClassificationPipeline {
        ClassificationPipeline {
            rules: RulesEngine {
                domain_rules: vec![DomainRule {
                    id: 1,
                    domain: "x.com".into(),
                    classification: Classification::Distracted,
                    project_id: None,
                    commitment_id: None,
                    only_in_focus: true,
                }],
                app_rules: vec![],
            },
            corrections: CorrectionMatcher::default(),
            private_processes: vec!["1password.exe".into()],
        }
    }

    #[test]
    fn idle_wins_over_everything() {
        let p = pipeline_with_block();
        let mut c = ctx(Some("x.com"), "Home / X", Some(1));
        c.is_idle = true;
        match p.evaluate(&c) {
            PipelineResult::Decided(o) => assert_eq!(o.classification, Classification::Idle),
            _ => panic!("idle must be decided deterministically"),
        }
    }

    #[test]
    fn private_app_is_neutral_without_ai() {
        let p = pipeline_with_block();
        let mut c = ctx(None, "1Password", Some(1));
        c.process_name = "1password.exe".into();
        match p.evaluate(&c) {
            PipelineResult::Decided(o) => {
                assert_eq!(o.classification, Classification::Neutral);
                assert_eq!(o.source, ClassificationSource::Rule);
            }
            _ => panic!("private apps never reach AI"),
        }
    }

    #[test]
    fn redacted_private_sentinel_stays_private() {
        // After redaction the sample carries the sentinel, not the original
        // process name — it must still short-circuit before rules/cache/AI.
        let p = pipeline_with_block();
        let mut c = ctx(None, "Private Application", Some(1));
        c.process_name = crate::types::PRIVATE_PROCESS_SENTINEL.into();
        c.window_title = String::new();
        match p.evaluate(&c) {
            PipelineResult::Decided(o) => assert_eq!(o.classification, Classification::Neutral),
            PipelineResult::NeedsAi => panic!("redacted private sample must never reach AI"),
        }
    }

    #[test]
    fn exact_correction_outranks_blocked_domain_rule() {
        let mut p = pipeline_with_block();
        p.corrections.corrections.push(Correction {
            id: 1,
            process_name: "chrome.exe".into(),
            browser_domain: Some("x.com".into()),
            normalized_title: "Company announcement / X".into(),
            commitment_id: Some(1),
            project_id: None,
            classification: Classification::Supporting,
        });
        match p.evaluate(&ctx(Some("x.com"), "Company announcement / X", Some(1))) {
            PipelineResult::Decided(o) => assert_eq!(o.classification, Classification::Supporting),
            _ => panic!("correction should decide"),
        }
    }

    #[test]
    fn ambiguous_activity_with_commitment_needs_ai() {
        let p = pipeline_with_block();
        let r = p.evaluate(&ctx(Some("news.ycombinator.com"), "Hacker News", Some(1)));
        assert_eq!(r, PipelineResult::NeedsAi);
    }

    #[test]
    fn no_commitment_defaults_to_neutral() {
        let p = pipeline_with_block();
        match p.evaluate(&ctx(Some("news.ycombinator.com"), "Hacker News", None)) {
            PipelineResult::Decided(o) => assert_eq!(o.classification, Classification::Neutral),
            _ => panic!("no-commitment context must not call AI"),
        }
    }

    #[test]
    fn low_confidence_ai_becomes_unknown() {
        let out = ClassificationPipeline::resolve_ai(Classification::Distracted, 0.5, "unsure".into());
        assert_eq!(out.classification, Classification::Unknown);
        let out = ClassificationPipeline::resolve_ai(Classification::Distracted, 0.9, "sure".into());
        assert_eq!(out.classification, Classification::Distracted);
    }
}
