//! Layer 1: deterministic rules (spec §11).

use serde::{Deserialize, Serialize};

use crate::types::{ActivityContext, Classification, ClassificationSource, ClassifyOutcome};

/// Domains that default to Distracted during a focus session (spec §11).
/// Seeded into the database on first run; the user can edit or delete them.
pub const DEFAULT_BLOCKED_DOMAINS: &[&str] = &[
    "x.com",
    "twitter.com",
    "reddit.com",
    "facebook.com",
    "instagram.com",
    "tiktok.com",
    "espn.com",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainRule {
    pub id: i64,
    /// Matched as a label-boundary suffix: "x.com" matches "www.x.com" but
    /// not "notx.com".
    pub domain: String,
    pub classification: Classification,
    /// Scope: rule applies only while this project/commitment is active.
    pub project_id: Option<i64>,
    pub commitment_id: Option<i64>,
    /// Apply only while a focus session is running.
    pub only_in_focus: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRule {
    pub id: i64,
    /// Case-insensitive process name match ("outlook.exe").
    pub process_name: String,
    pub classification: Classification,
    pub project_id: Option<i64>,
    pub commitment_id: Option<i64>,
    pub only_in_focus: bool,
}

/// True when `domain` equals `rule_domain` or is a subdomain of it.
pub fn domain_matches(rule_domain: &str, domain: &str) -> bool {
    let rule = rule_domain.trim().trim_start_matches("www.").to_lowercase();
    let dom = domain.trim().trim_start_matches("www.").to_lowercase();
    if rule.is_empty() || dom.is_empty() {
        return false;
    }
    dom == rule || dom.ends_with(&format!(".{rule}"))
}

#[derive(Debug, Clone, Default)]
pub struct RulesEngine {
    pub domain_rules: Vec<DomainRule>,
    pub app_rules: Vec<AppRule>,
}

/// Specificity: commitment-scoped beats project-scoped beats global.
fn specificity(commitment_id: Option<i64>, project_id: Option<i64>) -> u8 {
    match (commitment_id, project_id) {
        (Some(_), _) => 2,
        (None, Some(_)) => 1,
        (None, None) => 0,
    }
}

impl RulesEngine {
    pub fn evaluate(&self, ctx: &ActivityContext) -> Option<ClassifyOutcome> {
        let mut best: Option<(u8, ClassifyOutcome)> = None;

        if let Some(domain) = ctx.browser_domain.as_deref() {
            for rule in &self.domain_rules {
                if !rule_in_scope(rule.commitment_id, rule.project_id, rule.only_in_focus, ctx) {
                    continue;
                }
                if domain_matches(&rule.domain, domain) {
                    let spec = specificity(rule.commitment_id, rule.project_id);
                    let outcome = ClassifyOutcome {
                        classification: rule.classification,
                        confidence: 1.0,
                        source: ClassificationSource::Rule,
                        reason: format!("Domain rule: {}", rule.domain),
                    };
                    if best.as_ref().is_none_or(|(s, _)| spec > *s) {
                        best = Some((spec, outcome));
                    }
                }
            }
        }

        for rule in &self.app_rules {
            if !rule_in_scope(rule.commitment_id, rule.project_id, rule.only_in_focus, ctx) {
                continue;
            }
            if rule.process_name.eq_ignore_ascii_case(&ctx.process_name) {
                let spec = specificity(rule.commitment_id, rule.project_id);
                let outcome = ClassifyOutcome {
                    classification: rule.classification,
                    confidence: 1.0,
                    source: ClassificationSource::Rule,
                    reason: format!("Application rule: {}", rule.process_name),
                };
                // Domain rules win ties: they are more precise than app rules.
                if best.as_ref().is_none_or(|(s, _)| spec > *s) {
                    best = Some((spec, outcome));
                }
            }
        }

        best.map(|(_, o)| o)
    }
}

fn rule_in_scope(
    commitment_id: Option<i64>,
    project_id: Option<i64>,
    only_in_focus: bool,
    ctx: &ActivityContext,
) -> bool {
    if only_in_focus && !ctx.in_focus_session {
        return false;
    }
    if let Some(cid) = commitment_id {
        if ctx.commitment_id != Some(cid) {
            return false;
        }
    }
    if let Some(pid) = project_id {
        if ctx.project_id != Some(pid) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(domain: Option<&str>, process: &str, in_focus: bool) -> ActivityContext {
        ActivityContext {
            app_name: process.trim_end_matches(".exe").to_string(),
            process_name: process.to_string(),
            window_title: "whatever".into(),
            browser_domain: domain.map(String::from),
            browser_title: None,
            commitment_id: Some(1),
            project_id: Some(10),
            in_focus_session: in_focus,
            is_idle: false,
        }
    }

    fn blocked_engine() -> RulesEngine {
        RulesEngine {
            domain_rules: DEFAULT_BLOCKED_DOMAINS
                .iter()
                .enumerate()
                .map(|(i, d)| DomainRule {
                    id: i as i64,
                    domain: (*d).into(),
                    classification: Classification::Distracted,
                    project_id: None,
                    commitment_id: None,
                    only_in_focus: true,
                })
                .collect(),
            app_rules: vec![],
        }
    }

    #[test]
    fn blocked_domain_is_distracted_during_focus() {
        let engine = blocked_engine();
        let out = engine
            .evaluate(&ctx(Some("x.com"), "chrome.exe", true))
            .expect("x.com should match");
        assert_eq!(out.classification, Classification::Distracted);
    }

    #[test]
    fn blocked_domain_not_applied_outside_focus() {
        let engine = blocked_engine();
        assert!(engine.evaluate(&ctx(Some("x.com"), "chrome.exe", false)).is_none());
    }

    #[test]
    fn subdomain_matches_but_lookalike_does_not() {
        assert!(domain_matches("x.com", "www.x.com"));
        assert!(domain_matches("reddit.com", "old.reddit.com"));
        assert!(!domain_matches("x.com", "notx.com"));
        assert!(!domain_matches("x.com", "x.com.evil.io"));
    }

    #[test]
    fn scoped_rule_beats_global_rule() {
        let mut engine = blocked_engine();
        // User exception: x.com is Supporting while working on project 10
        // (say, social media marketing).
        engine.domain_rules.push(DomainRule {
            id: 99,
            domain: "x.com".into(),
            classification: Classification::Supporting,
            project_id: Some(10),
            commitment_id: None,
            only_in_focus: false,
        });
        let out = engine.evaluate(&ctx(Some("x.com"), "chrome.exe", true)).unwrap();
        assert_eq!(out.classification, Classification::Supporting);
    }

    #[test]
    fn app_rule_matches_process_case_insensitively() {
        let engine = RulesEngine {
            domain_rules: vec![],
            app_rules: vec![AppRule {
                id: 1,
                process_name: "OUTLOOK.EXE".into(),
                classification: Classification::Neutral,
                project_id: None,
                commitment_id: None,
                only_in_focus: false,
            }],
        };
        let out = engine.evaluate(&ctx(None, "outlook.exe", false)).unwrap();
        assert_eq!(out.classification, Classification::Neutral);
    }
}
