//! App settings (spec §51), stored as one JSON blob in the settings table.
//! The AI API key is NOT here — it lives in OS credential storage (spec §35).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Work rhythm
    pub work_start_min: u32,
    pub work_end_min: u32,
    pub interview_time_min: u32,
    pub review_time_min: u32,
    pub checkin_cadence_min: u32,

    // Accountability
    pub distraction_warn_secs: i64,
    pub distraction_intervene_secs: i64,
    pub strict_mode: bool,
    pub idle_threshold_secs: u64,

    // App behavior
    pub launch_at_startup: bool,
    pub start_minimized: bool,
    pub widget_enabled: bool,
    pub widget_always_on_top: bool,

    // Monitoring & privacy
    pub monitoring_consent: bool,
    pub browser_monitoring_enabled: bool,
    pub activity_retention_days: u32,
    pub excluded_apps: Vec<String>,
    pub excluded_domains: Vec<String>,
    pub private_apps: Vec<String>,
    pub demo_mode: bool,

    // AI
    pub ai_classification_enabled: bool,
    pub ai_coaching_enabled: bool,
    pub ai_base_url: String,
    pub ai_classify_model: String,
    pub ai_coach_model: String,

    // Browser extension bridge
    pub extension_port: u16,
    pub extension_token: String,

    pub onboarding_completed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            work_start_min: 8 * 60,
            work_end_min: 17 * 60,
            interview_time_min: 8 * 60,
            review_time_min: 17 * 60,
            checkin_cadence_min: 90,
            distraction_warn_secs: 180,
            distraction_intervene_secs: 420,
            strict_mode: false,
            idle_threshold_secs: 180,
            launch_at_startup: false,
            start_minimized: false,
            widget_enabled: false,
            widget_always_on_top: true,
            monitoring_consent: false,
            browser_monitoring_enabled: true,
            activity_retention_days: 365,
            excluded_apps: vec![],
            excluded_domains: vec![],
            private_apps: vec![],
            demo_mode: false,
            ai_classification_enabled: false,
            ai_coaching_enabled: false,
            ai_base_url: "https://api.openai.com/v1".into(),
            ai_classify_model: "gpt-4o-mini".into(),
            ai_coach_model: "gpt-5.6-luna".into(),
            extension_port: 43117,
            extension_token: String::new(),
            onboarding_completed: false,
        }
    }
}

const KEY: &str = "app_settings";

pub fn load(conn: &Connection) -> AppResult<Settings> {
    let json: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [KEY], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    let mut settings = match json {
        // serde(default) keeps old blobs working as fields are added.
        Some(j) => serde_json::from_str(&j)?,
        None => Settings::default(),
    };
    let mut changed = false;
    if matches!(
        settings.ai_coach_model.trim().to_ascii_lowercase().as_str(),
        "gpt-4o" | "gpt-4.1-mini"
    ) {
        settings.ai_coach_model = "gpt-5.6-luna".into();
        changed = true;
    }
    if settings.extension_token.is_empty() {
        settings.extension_token = generate_token();
        changed = true;
    }
    if changed {
        save(conn, &settings)?;
    }
    Ok(settings)
}

pub fn save(conn: &Connection, settings: &Settings) -> AppResult<()> {
    let json = serde_json::to_string(settings)?;
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![KEY, json],
    )?;
    Ok(())
}

fn generate_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| {
            let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            chars[rng.gen_range(0..chars.len())] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn defaults_use_luna_for_coaching_and_mini_for_classification() {
        let settings = Settings::default();
        assert_eq!(settings.ai_classify_model, "gpt-4o-mini");
        assert_eq!(settings.ai_coach_model, "gpt-5.6-luna");
    }

    #[test]
    fn stored_former_defaults_are_upgraded_to_luna() {
        for former_default in ["gpt-4o", "gpt-4.1-mini"] {
            let conn = settings_connection();
            let settings = Settings {
                extension_token: "existing-token".into(),
                ai_coach_model: former_default.into(),
                ..Settings::default()
            };
            save(&conn, &settings).unwrap();

            let loaded = load(&conn).unwrap();

            assert_eq!(loaded.ai_coach_model, "gpt-5.6-luna");
            let reloaded = load(&conn).unwrap();
            assert_eq!(reloaded.ai_coach_model, "gpt-5.6-luna");
        }
    }

    #[test]
    fn stored_custom_coaching_model_is_preserved() {
        let conn = settings_connection();
        let settings = Settings {
            extension_token: "existing-token".into(),
            ai_coach_model: "provider/custom-coach".into(),
            ..Settings::default()
        };
        save(&conn, &settings).unwrap();

        let loaded = load(&conn).unwrap();

        assert_eq!(loaded.ai_coach_model, "provider/custom-coach");
    }
}
