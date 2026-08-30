//! One-time recovery for databases created by development builds launched
//! from the packaged Codex app. Windows redirects those builds' roaming app
//! data into Codex's package-local cache, while an installed build uses the
//! normal roaming directory. This module safely joins those two histories.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, DatabaseName, OpenFlags};

use crate::error::{AppError, AppResult};

use super::{migrations, scores};

const DATABASE_NAME: &str = "accountability.sqlite3";
const APP_DATA_DIR_NAME: &str = "com.accountability-os.desktop";
const RECOVERY_MARKER_KEY: &str = "codex_virtualized_database_recovery_v1";

#[derive(Debug, Clone)]
pub struct RecoveryReport {
    pub source: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub imported_activity_sessions: usize,
}

/// Recover the richest compatible database found in Codex's Windows package
/// cache. A populated installed database always wins; this only repairs the
/// specific split-brain state where the installed app has settings/default
/// rules and monitoring samples, but no user-created planning data.
pub fn recover_codex_virtualized_database(target: &Path) -> AppResult<Option<RecoveryReport>> {
    #[cfg(not(windows))]
    {
        let _ = target;
        Ok(None)
    }

    #[cfg(windows)]
    {
        let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") else {
            return Ok(None);
        };
        let mut candidates = discover_codex_databases(Path::new(&local_app_data))?;
        candidates.retain(|candidate| !same_file_path(candidate, target));

        let mut scored = Vec::new();
        for candidate in candidates {
            match open_read_only(&candidate).and_then(|conn| recoverable_data_score(&conn)) {
                Ok(score) if score > 0 => scored.push((score, candidate)),
                Ok(_) => {}
                Err(error) => {
                    log::warn!(
                        target: "recovery",
                        "ignoring unreadable Codex database {}: {error}",
                        candidate.display()
                    );
                }
            }
        }
        scored.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));

        for (_, candidate) in scored {
            match recover_legacy_database(target, &candidate) {
                Ok(Some(mut report)) => {
                    report.source = candidate;
                    return Ok(Some(report));
                }
                Ok(None) => return Ok(None),
                Err(error) => {
                    log::warn!(
                        target: "recovery",
                        "could not recover Codex database {}: {error}",
                        candidate.display()
                    );
                }
            }
        }
        Ok(None)
    }
}

fn discover_codex_databases(local_app_data: &Path) -> AppResult<Vec<PathBuf>> {
    let packages = local_app_data.join("Packages");
    if !packages.is_dir() {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(packages)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("openai.codex_")
        {
            continue;
        }
        let candidate = entry
            .path()
            .join("LocalCache")
            .join("Roaming")
            .join(APP_DATA_DIR_NAME)
            .join(DATABASE_NAME);
        if candidate.is_file() {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn same_file_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// Restore `legacy` only when `target` is a fresh installed database. The
/// target is backed up first, its later monitoring samples are appended to a
/// validated legacy snapshot, and only then is the target replaced through
/// SQLite's online backup API.
fn recover_legacy_database(target: &Path, legacy: &Path) -> AppResult<Option<RecoveryReport>> {
    if !legacy.is_file() || same_file_path(target, legacy) {
        return Ok(None);
    }

    let legacy_conn = open_read_only(legacy)?;
    if recoverable_data_score(&legacy_conn)? == 0 {
        return Ok(None);
    }

    let target_has_database = match target.metadata() {
        Ok(metadata) => metadata.len() > 0,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let target_conn = if target_has_database {
        // Open through the exact path the installed app uses and fold any
        // committed WAL pages into the main file before taking the snapshot.
        // This avoids carrying stale sidecars across the restore boundary.
        let conn = Connection::open(target)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        if recovery_was_completed(&conn)? {
            return Ok(None);
        }
        if !is_fresh_installed_database(&conn)? {
            // A populated installed database is authoritative, including when
            // data was restored manually. Retire the virtualized source now so
            // deleting current planning rows cannot make it eligible later.
            record_recovery_completed(&conn, legacy)?;
            return Ok(None);
        }
        checkpoint_wal(&conn)?;
        Some(conn)
    } else {
        None
    };

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let stamp = recovery_stamp();
    let backup_path = if let Some(target_conn) = target_conn.as_ref() {
        let backup_dir = target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("recovery-backups");
        std::fs::create_dir_all(&backup_dir)?;
        let path = backup_dir.join(format!(
            "accountability-before-legacy-recovery-{stamp}.sqlite3"
        ));
        target_conn.backup(DatabaseName::Main, &path, None)?;
        Some(path)
    } else {
        None
    };

    let temp_path = target
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".accountability-legacy-recovery-{stamp}.tmp.sqlite3"
        ));
    let temp_guard = TemporaryDatabase::new(temp_path);
    legacy_conn.backup(DatabaseName::Main, temp_guard.path(), None)?;
    drop(legacy_conn);

    let consolidated = Connection::open(temp_guard.path())?;
    consolidated.pragma_update(None, "foreign_keys", "ON")?;
    migrations::apply(&consolidated)?;

    let imported_activity_sessions = if let Some(snapshot_path) = backup_path.as_ref() {
        append_fresh_target_data(&consolidated, snapshot_path)?
    } else {
        0
    };
    record_recovery_completed(&consolidated, legacy)?;
    validate_database(&consolidated)?;

    drop(target_conn);
    let restore_result = restore_connection_into_path(&consolidated, target);
    if let Err(restore_error) = restore_result {
        if let Some(snapshot_path) = backup_path.as_ref() {
            let snapshot = open_read_only(snapshot_path)?;
            if let Err(rollback_error) = restore_connection_into_path(&snapshot, target) {
                return Err(AppError::Internal(format!(
                    "legacy recovery failed: {restore_error}; restoring the pre-recovery backup also failed: {rollback_error}"
                )));
            }
        }
        return Err(restore_error);
    }
    drop(consolidated);

    Ok(Some(RecoveryReport {
        source: legacy.to_path_buf(),
        backup_path,
        imported_activity_sessions,
    }))
}

fn open_read_only(path: &Path) -> AppResult<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

fn restore_connection_into_path(source: &Connection, target: &Path) -> AppResult<()> {
    let mut destination = Connection::open(target)?;
    destination.busy_timeout(std::time::Duration::from_secs(5))?;
    {
        let backup = rusqlite::backup::Backup::new(source, &mut destination)?;
        backup.run_to_completion(100, std::time::Duration::from_millis(5), None)?;
    }
    destination.pragma_update(None, "foreign_keys", "ON")?;
    checkpoint_wal(&destination)?;
    validate_database(&destination)
}

fn recoverable_data_score(conn: &Connection) -> AppResult<i64> {
    Ok(planning_data_score(conn)? + table_count_if_present(conn, "activity_sessions")?)
}

fn planning_data_score(conn: &Connection) -> AppResult<i64> {
    let mut score = 0;
    for table in [
        "projects",
        "tasks",
        "daily_plans",
        "daily_commitments",
        "activity_corrections",
        "application_rules",
        "focus_sessions",
        "checkins",
        "checkin_responses",
        "interruptions",
        "breaks",
        "daily_reviews",
        "daily_scores",
        "ai_insights",
    ] {
        score += table_count_if_present(conn, table)?;
    }
    if table_exists(conn, "domain_rules")? {
        score += conn.query_row(
            "SELECT COUNT(*) FROM domain_rules WHERE is_default = 0",
            [],
            |row| row.get::<_, i64>(0),
        )?;
    }
    Ok(score)
}

fn is_fresh_installed_database(conn: &Connection) -> AppResult<bool> {
    if planning_data_score(conn)? != 0 {
        return Ok(false);
    }
    if table_exists(conn, "activity_sessions")? {
        let linked_sessions: i64 = conn.query_row(
            "SELECT COUNT(*) FROM activity_sessions
             WHERE related_task_id IS NOT NULL OR related_commitment_id IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        if linked_sessions != 0 {
            return Ok(false);
        }
    }
    if table_exists(conn, "domain_rules")? {
        let linked_rules: i64 = conn.query_row(
            "SELECT COUNT(*) FROM domain_rules
             WHERE is_default = 0 OR project_id IS NOT NULL OR commitment_id IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        if linked_rules != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn recovery_was_completed(conn: &Connection) -> AppResult<bool> {
    if !table_exists(conn, "settings")? {
        return Ok(false);
    }
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM settings WHERE key = ?1)",
        [RECOVERY_MARKER_KEY],
        |row| row.get(0),
    )?)
}

fn record_recovery_completed(conn: &Connection, source: &Path) -> AppResult<()> {
    let value = serde_json::json!({
        "completed_at": chrono::Utc::now().to_rfc3339(),
        "source": source.to_string_lossy(),
    });
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (RECOVERY_MARKER_KEY, serde_json::to_string(&value)?),
    )?;
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
        [table],
        |row| row.get(0),
    )?)
}

fn table_count_if_present(conn: &Connection, table: &str) -> AppResult<i64> {
    if !table_exists(conn, table)? {
        return Ok(0);
    }
    let sql = format!("SELECT COUNT(*) FROM {table}");
    Ok(conn.query_row(&sql, [], |row| row.get(0))?)
}

fn append_fresh_target_data(conn: &Connection, snapshot_path: &Path) -> AppResult<usize> {
    conn.execute(
        "ATTACH DATABASE ?1 AS current_snapshot",
        [snapshot_path.to_string_lossy().as_ref()],
    )?;

    let result = (|| -> AppResult<usize> {
        let tx = conn.unchecked_transaction()?;
        let affected_score_dates = activity_dates_to_import(&tx)?;
        let imported = tx.execute(
            "INSERT INTO main.activity_sessions (
                local_date, started_at, ended_at, duration_seconds,
                application_name, process_name, window_title,
                browser_domain, browser_title, classification,
                classification_confidence, classification_source,
                classification_reason, related_task_id,
                related_commitment_id, is_idle, pending_ai, created_at
             )
             SELECT
                current.local_date, current.started_at, current.ended_at,
                current.duration_seconds, current.application_name,
                current.process_name, current.window_title,
                current.browser_domain, current.browser_title,
                current.classification, current.classification_confidence,
                current.classification_source, current.classification_reason,
                NULL, NULL, current.is_idle, current.pending_ai,
                current.created_at
             FROM current_snapshot.activity_sessions current
             WHERE NOT EXISTS (
                SELECT 1 FROM main.activity_sessions legacy
                WHERE legacy.started_at = current.started_at
                  AND legacy.ended_at = current.ended_at
                  AND legacy.application_name = current.application_name
                  AND legacy.process_name = current.process_name
                  AND legacy.window_title = current.window_title
                  AND COALESCE(legacy.browser_domain, '') =
                      COALESCE(current.browser_domain, '')
             )",
            [],
        )?;

        tx.execute_batch(
            "INSERT INTO main.domain_rules (
                domain, classification, project_id, commitment_id,
                only_in_focus, is_default, created_at
             )
             SELECT current.domain, current.classification, NULL, NULL,
                    current.only_in_focus, current.is_default,
                    current.created_at
             FROM current_snapshot.domain_rules current
             WHERE current.is_default = 1
               AND NOT EXISTS (
                   SELECT 1 FROM main.domain_rules legacy
                   WHERE legacy.domain = current.domain
                     AND legacy.classification = current.classification
                     AND legacy.only_in_focus = current.only_in_focus
                     AND legacy.is_default = 1
               );

             INSERT INTO main.classification_cache (
                 cache_key, classification, confidence, reason, created_at
              )
              SELECT cache_key, classification, confidence, reason, created_at
             FROM current_snapshot.classification_cache
             WHERE 1
             ON CONFLICT(cache_key) DO UPDATE SET
                 classification = excluded.classification,
                 confidence = excluded.confidence,
                 reason = excluded.reason,
                 created_at = excluded.created_at
             WHERE excluded.created_at > classification_cache.created_at;",
        )?;
        merge_settings(&tx)?;
        for date in affected_score_dates {
            scores::refresh_stored_score(&tx, &date)?;
        }
        tx.commit()?;
        Ok(imported)
    })();

    let detach_result = conn.execute_batch("DETACH DATABASE current_snapshot;");
    match (result, detach_result) {
        (Ok(imported), Ok(())) => Ok(imported),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
    }
}

fn activity_dates_to_import(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT current.local_date
         FROM current_snapshot.activity_sessions current
         WHERE NOT EXISTS (
            SELECT 1 FROM main.activity_sessions legacy
            WHERE legacy.started_at = current.started_at
              AND legacy.ended_at = current.ended_at
              AND legacy.application_name = current.application_name
              AND legacy.process_name = current.process_name
              AND legacy.window_title = current.window_title
              AND COALESCE(legacy.browser_domain, '') =
                  COALESCE(current.browser_domain, '')
         )",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn merge_settings(conn: &Connection) -> AppResult<()> {
    let legacy: Option<String> = optional_setting(conn, "main")?;
    let current: Option<String> = optional_setting(conn, "current_snapshot")?;
    let (Some(legacy), Some(current)) = (legacy, current) else {
        return Ok(());
    };

    let legacy_json: serde_json::Value = serde_json::from_str(&legacy)?;
    let current_json: serde_json::Value = serde_json::from_str(&current)?;
    let merged = merged_settings(legacy_json, current_json)?;

    conn.execute(
        "UPDATE main.settings SET value = ?1 WHERE key = 'app_settings'",
        [serde_json::to_string(&merged)?],
    )?;
    Ok(())
}

fn merged_settings(
    mut legacy: serde_json::Value,
    current: serde_json::Value,
) -> AppResult<serde_json::Value> {
    let defaults = serde_json::to_value(super::settings::Settings::default())?;
    let (Some(legacy_object), Some(current_object), Some(default_object)) = (
        legacy.as_object_mut(),
        current.as_object(),
        defaults.as_object(),
    ) else {
        return Ok(legacy);
    };

    // A value that differs from the installed version's default was changed
    // after installation, so it is newer intent. Default-valued fields keep
    // the richer legacy preference instead of resetting work hours, strict
    // mode, or the widget just because the installed app launched once.
    for (key, value) in current_object {
        if is_privacy_or_installation_local_setting(key) {
            continue;
        }
        if !legacy_object.contains_key(key) || default_object.get(key) != Some(value) {
            legacy_object.insert(key.clone(), value.clone());
        }
    }

    // Privacy is monotonic: recovery must never broaden collection or AI use,
    // discard an exclusion, or lengthen retention. Consent is explicitly tied
    // to the currently installed app. The extension bridge token/port are also
    // installation-local and must stay current.
    copy_current_value(legacy_object, current_object, "monitoring_consent");
    for key in [
        "browser_monitoring_enabled",
        "ai_classification_enabled",
        "ai_coaching_enabled",
    ] {
        merge_boolean_and(legacy_object, current_object, key);
    }
    for key in ["excluded_apps", "excluded_domains", "private_apps"] {
        merge_string_array_union(legacy_object, current_object, key);
    }
    merge_integer_min(legacy_object, current_object, "activity_retention_days");
    copy_current_value(legacy_object, current_object, "extension_port");
    copy_current_value(legacy_object, current_object, "extension_token");

    Ok(legacy)
}

fn is_privacy_or_installation_local_setting(key: &str) -> bool {
    matches!(
        key,
        "monitoring_consent"
            | "browser_monitoring_enabled"
            | "ai_classification_enabled"
            | "ai_coaching_enabled"
            | "excluded_apps"
            | "excluded_domains"
            | "private_apps"
            | "activity_retention_days"
            | "extension_port"
            | "extension_token"
    )
}

fn copy_current_value(
    legacy: &mut serde_json::Map<String, serde_json::Value>,
    current: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) {
    if let Some(value) = current.get(key) {
        legacy.insert(key.into(), value.clone());
    }
}

fn merge_boolean_and(
    legacy: &mut serde_json::Map<String, serde_json::Value>,
    current: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) {
    let Some(current_value) = current.get(key).and_then(serde_json::Value::as_bool) else {
        return;
    };
    let legacy_value = legacy
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    legacy.insert(
        key.into(),
        serde_json::Value::Bool(legacy_value && current_value),
    );
}

fn merge_integer_min(
    legacy: &mut serde_json::Map<String, serde_json::Value>,
    current: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) {
    let Some(current_value) = current.get(key).and_then(serde_json::Value::as_u64) else {
        return;
    };
    let merged = legacy
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .map_or(current_value, |legacy_value| {
            legacy_value.min(current_value)
        });
    legacy.insert(key.into(), serde_json::Value::from(merged));
}

fn merge_string_array_union(
    legacy: &mut serde_json::Map<String, serde_json::Value>,
    current: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) {
    let mut merged = Vec::new();
    for value in [legacy.get(key), current.get(key)]
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter_map(serde_json::Value::as_str)
    {
        if !merged.iter().any(|existing| existing == value) {
            merged.push(value.to_string());
        }
    }
    legacy.insert(
        key.into(),
        serde_json::Value::Array(merged.into_iter().map(serde_json::Value::String).collect()),
    );
}

fn optional_setting(conn: &Connection, schema: &str) -> AppResult<Option<String>> {
    let sql = format!("SELECT value FROM {schema}.settings WHERE key = 'app_settings'");
    match conn.query_row(&sql, [], |row| row.get(0)) {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn validate_database(conn: &Connection) -> AppResult<()> {
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(AppError::Internal(format!(
            "database integrity check failed: {integrity}"
        )));
    }
    let foreign_key_errors: i64 =
        conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_errors != 0 {
        return Err(AppError::Internal(format!(
            "database has {foreign_key_errors} foreign-key violation(s)"
        )));
    }
    Ok(())
}

fn checkpoint_wal(conn: &Connection) -> AppResult<()> {
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
    if busy != 0 {
        return Err(AppError::Internal(
            "database WAL is busy; recovery was not applied".into(),
        ));
    }
    Ok(())
}

fn recovery_stamp() -> String {
    format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%fZ"),
        std::process::id()
    )
}

struct TemporaryDatabase {
    path: PathBuf,
}

impl TemporaryDatabase {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        for path in [
            self.path.clone(),
            sidecar_path(&self.path, "-wal"),
            sidecar_path(&self.path, "-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(suffix);
    path.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::settings::{self, Settings};
    use rusqlite::params;

    fn initialized(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        migrations::apply(&conn).unwrap();
        conn
    }

    fn insert_activity(conn: &Connection, started_at: i64, title: &str) {
        conn.execute(
            "INSERT INTO activity_sessions(
                local_date, started_at, ended_at, duration_seconds,
                application_name, process_name, window_title,
                classification, classification_source, created_at
             ) VALUES('2026-08-30', ?1, ?1 + 5, 5, 'Test', 'test.exe',
                      ?2, 'neutral', 'default', ?1)",
            params![started_at, title],
        )
        .unwrap();
    }

    fn insert_cache(
        conn: &Connection,
        key: &str,
        classification: &str,
        reason: &str,
        created_at: i64,
    ) {
        conn.execute(
            "INSERT INTO classification_cache(
                cache_key, classification, confidence, reason, created_at
             ) VALUES(?1, ?2, 0.9, ?3, ?4)",
            params![key, classification, reason, created_at],
        )
        .unwrap();
    }

    #[test]
    fn restores_legacy_data_and_keeps_new_monitoring_samples() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.sqlite3");
        let target_path = dir.path().join(DATABASE_NAME);

        let legacy = initialized(&legacy_path);
        legacy
            .execute(
                "INSERT INTO tasks(title, created_at) VALUES('Recovered task', 1)",
                [],
            )
            .unwrap();
        insert_activity(&legacy, 100, "shared sample");
        insert_cache(
            &legacy,
            "current-is-newer",
            "productive",
            "legacy stale",
            100,
        );
        insert_cache(
            &legacy,
            "legacy-is-newer",
            "productive",
            "legacy newest",
            300,
        );
        let legacy_settings = Settings {
            work_end_min: 18 * 60,
            checkin_cadence_min: 60,
            strict_mode: true,
            monitoring_consent: true,
            ai_classification_enabled: true,
            ai_coaching_enabled: true,
            excluded_apps: vec!["legacy-private.exe".into()],
            private_apps: vec!["legacy-secret.exe".into()],
            extension_token: "legacy-extension-token".into(),
            onboarding_completed: true,
            ..Settings::default()
        };
        settings::save(&legacy, &legacy_settings).unwrap();
        legacy
            .execute(
                "INSERT INTO daily_scores(
                    date, neutral_secs, context_switches, computed_at
                 ) VALUES('2026-08-30', 5, 0, 1)",
                [],
            )
            .unwrap();
        drop(legacy);

        let target = initialized(&target_path);
        target.pragma_update(None, "journal_mode", "WAL").unwrap();
        insert_activity(&target, 100, "shared sample");
        insert_activity(&target, 200, "new sample");
        insert_cache(
            &target,
            "current-is-newer",
            "distracting",
            "current newest",
            200,
        );
        insert_cache(
            &target,
            "legacy-is-newer",
            "distracting",
            "current stale",
            200,
        );
        let current_settings = Settings {
            work_end_min: 17 * 60,
            checkin_cadence_min: 45,
            monitoring_consent: false,
            browser_monitoring_enabled: false,
            activity_retention_days: 30,
            ai_classification_enabled: false,
            ai_coaching_enabled: false,
            excluded_apps: vec!["current-private.exe".into()],
            private_apps: vec!["current-secret.exe".into()],
            extension_token: "current-extension-token".into(),
            onboarding_completed: true,
            ..Settings::default()
        };
        settings::save(&target, &current_settings).unwrap();
        drop(target);

        let report = recover_legacy_database(&target_path, &legacy_path)
            .unwrap()
            .expect("recovery should run");

        assert_eq!(report.imported_activity_sessions, 1);
        let backup_path = report.backup_path.expect("existing target is backed up");
        assert!(backup_path.is_file());

        let restored = Connection::open(&target_path).unwrap();
        let tasks: i64 = restored
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        let sessions: i64 = restored
            .query_row("SELECT COUNT(*) FROM activity_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(tasks, 1);
        assert_eq!(sessions, 2, "the duplicate sample must not be copied twice");

        let restored_settings = settings::load(&restored).unwrap();
        assert_eq!(restored_settings.work_end_min, 18 * 60);
        assert_eq!(restored_settings.checkin_cadence_min, 45);
        assert!(restored_settings.strict_mode);
        assert!(!restored_settings.monitoring_consent);
        assert!(!restored_settings.browser_monitoring_enabled);
        assert!(!restored_settings.ai_classification_enabled);
        assert!(!restored_settings.ai_coaching_enabled);
        assert_eq!(restored_settings.activity_retention_days, 30);
        assert_eq!(
            restored_settings.excluded_apps,
            vec!["legacy-private.exe", "current-private.exe"]
        );
        assert_eq!(
            restored_settings.private_apps,
            vec!["legacy-secret.exe", "current-secret.exe"]
        );
        assert_eq!(
            restored_settings.extension_token, "current-extension-token",
            "the browser bridge must keep the installed app's token"
        );
        let refreshed_neutral_secs: i64 = restored
            .query_row(
                "SELECT neutral_secs FROM daily_scores WHERE date='2026-08-30'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(refreshed_neutral_secs, 10);
        let current_cache: (String, String, i64) = restored
            .query_row(
                "SELECT classification, reason, created_at
                 FROM classification_cache WHERE cache_key='current-is-newer'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            current_cache,
            ("distracting".into(), "current newest".into(), 200),
            "a newer installed-app classification must replace the legacy cache row"
        );
        let legacy_cache: (String, String, i64) = restored
            .query_row(
                "SELECT classification, reason, created_at
                 FROM classification_cache WHERE cache_key='legacy-is-newer'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            legacy_cache,
            ("productive".into(), "legacy newest".into(), 300),
            "an older installed-app classification must not replace the legacy cache row"
        );

        let backup = Connection::open(backup_path).unwrap();
        let backup_sessions: i64 = backup
            .query_row("SELECT COUNT(*) FROM activity_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(backup_sessions, 2);
    }

    #[test]
    fn activity_only_legacy_history_is_recovered_into_a_fresh_target() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.sqlite3");
        let target_path = dir.path().join(DATABASE_NAME);

        let legacy = initialized(&legacy_path);
        insert_activity(&legacy, 100, "legacy activity");
        drop(legacy);

        let target = initialized(&target_path);
        insert_activity(&target, 200, "current activity");
        drop(target);

        let report = recover_legacy_database(&target_path, &legacy_path)
            .unwrap()
            .expect("activity history alone should be recoverable");
        assert_eq!(report.imported_activity_sessions, 1);

        let restored = Connection::open(target_path).unwrap();
        let sessions: i64 = restored
            .query_row("SELECT COUNT(*) FROM activity_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(sessions, 2);
    }

    #[test]
    fn populated_target_wins_and_retires_the_legacy_source() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.sqlite3");
        let target_path = dir.path().join(DATABASE_NAME);
        for (path, title) in [
            (&legacy_path, "Legacy task"),
            (&target_path, "Current task"),
        ] {
            let conn = initialized(path);
            conn.execute(
                "INSERT INTO tasks(title, created_at) VALUES(?1, 1)",
                [title],
            )
            .unwrap();
        }

        assert!(recover_legacy_database(&target_path, &legacy_path)
            .unwrap()
            .is_none());
        let target = Connection::open(&target_path).unwrap();
        let title: String = target
            .query_row("SELECT title FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(title, "Current task");
        assert!(recovery_was_completed(&target).unwrap());
        target.execute("DELETE FROM tasks", []).unwrap();
        drop(target);

        assert!(recover_legacy_database(&target_path, &legacy_path)
            .unwrap()
            .is_none());
        let reopened = Connection::open(target_path).unwrap();
        let tasks: i64 = reopened
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(tasks, 0, "the retired legacy source must stay retired");
    }

    #[test]
    fn missing_target_is_restored_without_inventing_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.sqlite3");
        let target_path = dir.path().join(DATABASE_NAME);
        let legacy = initialized(&legacy_path);
        legacy
            .execute(
                "INSERT INTO tasks(title, created_at) VALUES('Legacy task', 1)",
                [],
            )
            .unwrap();
        drop(legacy);

        let report = recover_legacy_database(&target_path, &legacy_path)
            .unwrap()
            .expect("recovery should create the target");
        assert!(report.backup_path.is_none());

        let restored = Connection::open(target_path).unwrap();
        let title: String = restored
            .query_row("SELECT title FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(title, "Legacy task");
        validate_database(&restored).unwrap();
    }

    #[test]
    fn completed_recovery_does_not_resurrect_deleted_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.sqlite3");
        let target_path = dir.path().join(DATABASE_NAME);
        let legacy = initialized(&legacy_path);
        legacy
            .execute(
                "INSERT INTO tasks(title, created_at) VALUES('Delete me later', 1)",
                [],
            )
            .unwrap();
        drop(legacy);

        recover_legacy_database(&target_path, &legacy_path)
            .unwrap()
            .expect("initial recovery should run");
        let restored = Connection::open(&target_path).unwrap();
        assert!(recovery_was_completed(&restored).unwrap());
        restored.execute("DELETE FROM tasks", []).unwrap();
        drop(restored);

        assert!(recover_legacy_database(&target_path, &legacy_path)
            .unwrap()
            .is_none());
        let reopened = Connection::open(target_path).unwrap();
        let tasks: i64 = reopened
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(tasks, 0, "deleted tasks must stay deleted after restart");
    }

    #[test]
    fn discovers_only_codex_package_databases() {
        let dir = tempfile::tempdir().unwrap();
        let expected = dir
            .path()
            .join("Packages")
            .join("OpenAI.Codex_example")
            .join("LocalCache")
            .join("Roaming")
            .join(APP_DATA_DIR_NAME)
            .join(DATABASE_NAME);
        std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
        std::fs::write(&expected, b"sqlite placeholder").unwrap();

        let ignored = dir
            .path()
            .join("Packages")
            .join("Unrelated.App")
            .join("LocalCache")
            .join("Roaming")
            .join(APP_DATA_DIR_NAME)
            .join(DATABASE_NAME);
        std::fs::create_dir_all(ignored.parent().unwrap()).unwrap();
        std::fs::write(ignored, b"sqlite placeholder").unwrap();

        assert_eq!(
            discover_codex_databases(dir.path()).unwrap(),
            vec![expected]
        );
    }
}
