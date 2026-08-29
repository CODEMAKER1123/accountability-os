//! Hybrid classification engine (spec §11): deterministic rules, historical
//! user corrections, then AI for the ambiguous remainder. The AI call itself
//! lives in the app layer; this module decides *whether* AI is needed and
//! post-processes its answer.

mod cache;
mod corrections;
mod pipeline;
mod rules;

pub use cache::{cache_key, normalize_domain};
pub use corrections::{Correction, CorrectionMatcher};
pub use pipeline::{ClassificationPipeline, PipelineResult, AI_CONFIDENCE_THRESHOLD};
pub use rules::{domain_matches, AppRule, DomainRule, RulesEngine, DEFAULT_BLOCKED_DOMAINS};
