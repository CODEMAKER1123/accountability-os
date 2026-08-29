//! Accountability OS core domain logic.
//!
//! This crate is deliberately free of Tauri, SQLite and OS dependencies so the
//! business rules — session aggregation, classification, scoring, distraction
//! thresholds, check-in scheduling, pattern detection — are unit-testable on
//! any platform.

pub mod accountability;
pub mod aggregator;
pub mod classify;
pub mod events;
pub mod patterns;
pub mod scoring;
pub mod types;
