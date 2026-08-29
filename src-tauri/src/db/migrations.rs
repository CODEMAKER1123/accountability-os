//! Embedded schema migrations, applied via PRAGMA user_version.

pub const MIGRATIONS: &[&str] = &[
    // 0001 — initial schema (spec §29–30)
    r#"
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE projects (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  color TEXT,
  archived INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE TABLE tasks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
  parent_task_id INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
  status TEXT NOT NULL DEFAULT 'inbox',
  priority TEXT NOT NULL DEFAULT 'should',
  estimated_minutes INTEGER,
  due_date TEXT,
  tags TEXT NOT NULL DEFAULT '[]',
  created_at INTEGER NOT NULL,
  completed_at INTEGER
);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_project ON tasks(project_id);

CREATE TABLE daily_plans (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  date TEXT NOT NULL UNIQUE,
  locked_at INTEGER,
  ended_at INTEGER,
  likely_distraction TEXT NOT NULL DEFAULT '',
  countermeasure TEXT NOT NULL DEFAULT '',
  most_important_when TEXT NOT NULL DEFAULT 'flexible',
  interview_answers TEXT NOT NULL DEFAULT '{}',
  is_day_off INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE TABLE daily_commitments (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  plan_id INTEGER NOT NULL REFERENCES daily_plans(id) ON DELETE CASCADE,
  task_id INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
  title TEXT NOT NULL,
  done_definition TEXT NOT NULL DEFAULT '',
  estimated_minutes INTEGER,
  priority TEXT NOT NULL DEFAULT 'must',
  rank INTEGER NOT NULL DEFAULT 1,
  status TEXT NOT NULL DEFAULT 'pending',
  started_at INTEGER,
  completed_at INTEGER,
  outcome_reason TEXT,
  outcome_note TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_commitments_plan ON daily_commitments(plan_id);

CREATE TABLE activity_sessions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  local_date TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  ended_at INTEGER NOT NULL,
  duration_seconds INTEGER NOT NULL,
  application_name TEXT NOT NULL,
  process_name TEXT NOT NULL,
  window_title TEXT NOT NULL,
  browser_domain TEXT,
  browser_title TEXT,
  classification TEXT NOT NULL DEFAULT 'unknown',
  classification_confidence REAL,
  classification_source TEXT NOT NULL DEFAULT 'default',
  classification_reason TEXT,
  related_task_id INTEGER,
  related_commitment_id INTEGER,
  is_idle INTEGER NOT NULL DEFAULT 0,
  pending_ai INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_sessions_date ON activity_sessions(local_date);
CREATE INDEX idx_sessions_started ON activity_sessions(started_at);

CREATE TABLE activity_corrections (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id INTEGER REFERENCES activity_sessions(id) ON DELETE SET NULL,
  process_name TEXT NOT NULL,
  browser_domain TEXT,
  normalized_title TEXT NOT NULL,
  commitment_id INTEGER,
  project_id INTEGER,
  old_classification TEXT NOT NULL,
  new_classification TEXT NOT NULL,
  reason TEXT,
  created_at INTEGER NOT NULL
);

CREATE TABLE domain_rules (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  domain TEXT NOT NULL,
  classification TEXT NOT NULL,
  project_id INTEGER,
  commitment_id INTEGER,
  only_in_focus INTEGER NOT NULL DEFAULT 0,
  is_default INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE TABLE application_rules (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  process_name TEXT NOT NULL,
  classification TEXT NOT NULL,
  project_id INTEGER,
  commitment_id INTEGER,
  only_in_focus INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE TABLE classification_cache (
  cache_key TEXT PRIMARY KEY,
  classification TEXT NOT NULL,
  confidence REAL NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL
);

CREATE TABLE focus_sessions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  commitment_id INTEGER NOT NULL REFERENCES daily_commitments(id) ON DELETE CASCADE,
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  outcome TEXT,
  created_at INTEGER NOT NULL
);

CREATE TABLE checkins (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  due_at INTEGER NOT NULL,
  shown_at INTEGER,
  commitment_id INTEGER,
  window_stats TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL
);

CREATE TABLE checkin_responses (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  checkin_id INTEGER NOT NULL REFERENCES checkins(id) ON DELETE CASCADE,
  response TEXT NOT NULL,
  note TEXT,
  created_at INTEGER NOT NULL
);

CREATE TABLE interruptions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL,
  commitment_id INTEGER,
  app_name TEXT NOT NULL DEFAULT '',
  window_title TEXT NOT NULL DEFAULT '',
  distracted_secs INTEGER NOT NULL DEFAULT 0,
  started_at INTEGER NOT NULL,
  acknowledged_at INTEGER,
  response TEXT,
  response_note TEXT,
  returned_at INTEGER,
  recovery_secs INTEGER,
  created_at INTEGER NOT NULL
);

CREATE TABLE breaks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  started_at INTEGER NOT NULL,
  planned_end_at INTEGER NOT NULL,
  actual_end_at INTEGER,
  created_at INTEGER NOT NULL
);

CREATE TABLE daily_reviews (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  plan_id INTEGER NOT NULL REFERENCES daily_plans(id) ON DELETE CASCADE,
  reviewed_at INTEGER,
  ai_summary TEXT,
  created_at INTEGER NOT NULL
);

CREATE TABLE daily_scores (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  date TEXT NOT NULL UNIQUE,
  total REAL,
  completion REAL,
  alignment REAL,
  focus_quality REAL,
  planning_accuracy REAL,
  focused_secs INTEGER NOT NULL DEFAULT 0,
  supporting_secs INTEGER NOT NULL DEFAULT 0,
  neutral_secs INTEGER NOT NULL DEFAULT 0,
  distracted_secs INTEGER NOT NULL DEFAULT 0,
  idle_secs INTEGER NOT NULL DEFAULT 0,
  unknown_secs INTEGER NOT NULL DEFAULT 0,
  context_switches INTEGER NOT NULL DEFAULT 0,
  computed_at INTEGER NOT NULL
);

CREATE TABLE ai_insights (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  period TEXT NOT NULL,
  metric TEXT NOT NULL,
  text TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT 'deterministic',
  created_at INTEGER NOT NULL
);
"#,
    // 0002 — interruptions capture the exact activity context they flagged,
    // so "This is actually work" teaches the classifier about THAT activity,
    // not whatever window is foreground when the user answers.
    r#"
ALTER TABLE interruptions ADD COLUMN process_name TEXT NOT NULL DEFAULT '';
ALTER TABLE interruptions ADD COLUMN browser_domain TEXT;
"#,
    // 0003 — the true start of the flagged distraction episode (idle gaps
    // preserve an episode without extending distracted_secs, so the start
    // cannot be derived from the intervention time alone).
    r#"
ALTER TABLE interruptions ADD COLUMN episode_started_at INTEGER;
"#,
    // 0004 — rows sharing a group form ONE logical correction memory, stored
    // once per activity date it was learned from. Deleting any of those
    // dates deletes the whole group: the memory must not survive on a copy
    // anchored elsewhere.
    r#"
ALTER TABLE activity_corrections ADD COLUMN group_id INTEGER;
"#,
    // 0005 — group the intervention copies a pre-0004 build stored without
    // one, or grouped deletion would ignore them. Identical content IS the
    // same logical memory, so that's the grouping key; the negative id
    // keeps backfilled groups disjoint from the interruption ids new rows
    // use.
    r#"
UPDATE activity_corrections
SET group_id = -(
    SELECT MIN(a2.id) FROM activity_corrections a2
    WHERE a2.reason = 'Confirmed as work during intervention'
      AND a2.process_name = activity_corrections.process_name
      AND COALESCE(a2.browser_domain,'') = COALESCE(activity_corrections.browser_domain,'')
      AND a2.normalized_title = activity_corrections.normalized_title
      AND COALESCE(a2.commitment_id,-1) = COALESCE(activity_corrections.commitment_id,-1)
      AND COALESCE(a2.project_id,-1) = COALESCE(activity_corrections.project_id,-1)
      AND a2.old_classification = activity_corrections.old_classification
      AND a2.new_classification = activity_corrections.new_classification)
WHERE reason = 'Confirmed as work during intervention' AND group_id IS NULL;
"#,
    // 0006 — 0005's content-only key merged DISTINCT intervention events
    // that confirmed the same activity, so deleting a date covered by one
    // event also dropped the other event's still-valid memory. Copies of
    // one response were inserted in a single batch and share created_at;
    // re-split the backfilled (negative) groups on it so each event keeps
    // its own group.
    r#"
UPDATE activity_corrections
SET group_id = -(
    SELECT MIN(a2.id) FROM activity_corrections a2
    WHERE a2.group_id < 0
      AND a2.created_at = activity_corrections.created_at
      AND a2.process_name = activity_corrections.process_name
      AND COALESCE(a2.browser_domain,'') = COALESCE(activity_corrections.browser_domain,'')
      AND a2.normalized_title = activity_corrections.normalized_title
      AND COALESCE(a2.commitment_id,-1) = COALESCE(activity_corrections.commitment_id,-1)
      AND COALESCE(a2.project_id,-1) = COALESCE(activity_corrections.project_id,-1)
      AND a2.old_classification = activity_corrections.old_classification
      AND a2.new_classification = activity_corrections.new_classification)
WHERE group_id < 0;
"#,
    // 0007 — 0006 keyed on created_at EQUALITY, but the legacy build called
    // now() per row, so one batch's copies can straddle a second boundary
    // and end up split (one copy then outlives its sibling's deletion).
    // Recover the real batches instead: legacy copies were inserted in one
    // uninterrupted loop, so a batch is a run of id-adjacent legacy rows
    // with identical content and timestamps within 2s. Each legacy row is
    // regrouped to its nearest preceding "anchor" — a legacy row whose
    // preceding legacy row does NOT content-and-time match it.
    r#"
UPDATE activity_corrections
SET group_id = -(
    SELECT MAX(s.id) FROM activity_corrections s
    WHERE s.group_id < 0 AND s.id <= activity_corrections.id
      AND NOT EXISTS (
          SELECT 1 FROM activity_corrections p
          WHERE p.group_id < 0
            AND p.id = (SELECT MAX(q.id) FROM activity_corrections q
                        WHERE q.group_id < 0 AND q.id < s.id)
            AND p.process_name = s.process_name
            AND COALESCE(p.browser_domain,'') = COALESCE(s.browser_domain,'')
            AND p.normalized_title = s.normalized_title
            AND COALESCE(p.commitment_id,-1) = COALESCE(s.commitment_id,-1)
            AND COALESCE(p.project_id,-1) = COALESCE(s.project_id,-1)
            AND p.old_classification = s.old_classification
            AND p.new_classification = s.new_classification
            AND abs(s.created_at - p.created_at) <= 2))
WHERE group_id < 0;
"#,
    // 0008 — retire legacy intervention corrections outright. A database
    // that deleted a date under an intermediate build can hold a sibling
    // copy whose deletion was already requested, and such an orphan is
    // indistinguishable from a legitimate single-date memory — no
    // regrouping (0005–0007) can honor the earlier deletion after the
    // fact. These rows are a learning cache, not user data: purging every
    // backfilled (negative-group) row is privacy-safe, and one
    // re-confirmation at the next intervention re-teaches the classifier.
    // Rows written by current code carry positive interruption-id groups
    // and correct deletion semantics from the moment of insertion.
    r#"
DELETE FROM activity_corrections WHERE group_id < 0;
"#,
    // 0009 — enforce the runtime's single-active invariants in SQLite too.
    // Repair any rows created by older builds before adding the constraints.
    r#"
UPDATE focus_sessions
SET ended_at = CAST(strftime('%s','now') AS INTEGER),
    outcome = COALESCE(outcome, 'recovered')
WHERE ended_at IS NULL
  AND commitment_id IN (
    SELECT id FROM daily_commitments
    WHERE status IN ('completed', 'deferred', 'dropped', 'cancelled')
  );

UPDATE focus_sessions
SET ended_at = CAST(strftime('%s','now') AS INTEGER),
    outcome = COALESCE(outcome, 'recovered')
WHERE ended_at IS NULL
  AND id <> (
    SELECT id FROM focus_sessions
    WHERE ended_at IS NULL
    ORDER BY started_at DESC, id DESC
    LIMIT 1
  );

UPDATE daily_commitments
SET status = 'pending'
WHERE status = 'active'
  AND id NOT IN (
    SELECT commitment_id FROM focus_sessions
    WHERE ended_at IS NULL
  );

UPDATE daily_commitments
SET status = 'pending'
WHERE status = 'active'
  AND id <> (
    SELECT commitment_id FROM focus_sessions
    WHERE ended_at IS NULL
    ORDER BY started_at DESC, id DESC
    LIMIT 1
  );

UPDATE breaks
SET actual_end_at = planned_end_at
WHERE actual_end_at IS NULL
  AND id <> (
    SELECT id FROM breaks
    WHERE actual_end_at IS NULL
    ORDER BY started_at DESC, id DESC
    LIMIT 1
  );

DELETE FROM checkin_responses
WHERE id NOT IN (
  SELECT MIN(id) FROM checkin_responses GROUP BY checkin_id
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_focus_sessions_one_open
  ON focus_sessions((1)) WHERE ended_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS ux_commitments_one_active
  ON daily_commitments(status) WHERE status = 'active';
CREATE UNIQUE INDEX IF NOT EXISTS ux_breaks_one_open
  ON breaks((1)) WHERE actual_end_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS ux_checkin_responses_one_per_checkin
  ON checkin_responses(checkin_id);
"#,
    // 0010 — optional, editable action steps generated during the morning
    // interview. JSON keeps the checklist ordered and lets completion state
    // evolve without turning a short commitment checklist into task-tree
    // bookkeeping.
    r#"
ALTER TABLE daily_commitments ADD COLUMN steps TEXT NOT NULL DEFAULT '[]';
"#,
];

pub fn apply(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    reconcile_audit_v2_lineage(conn)?;
    apply_list(conn, MIGRATIONS)
}

/// The audit branch and the later review branch both independently shipped a
/// migration numbered 0002. Audit databases therefore have the invariant
/// indexes but not the interruption-context columns, while review databases
/// have the columns. Repair that one historical fork before advancing either
/// lineage through the shared migration list.
fn reconcile_audit_v2_lineage(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 2 {
        return Ok(());
    }
    let interruptions_exist: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='interruptions')",
        [],
        |row| row.get(0),
    )?;
    if !interruptions_exist {
        return Ok(());
    }
    let columns = {
        let mut stmt = conn.prepare("PRAGMA table_info(interruptions)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<Result<std::collections::HashSet<_>, _>>()?
    };
    if columns.contains("process_name") && columns.contains("browser_domain") {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    if !columns.contains("process_name") {
        tx.execute_batch(
            "ALTER TABLE interruptions ADD COLUMN process_name TEXT NOT NULL DEFAULT '';",
        )?;
    }
    if !columns.contains("browser_domain") {
        tx.execute_batch("ALTER TABLE interruptions ADD COLUMN browser_domain TEXT;")?;
    }
    tx.commit()
}

fn apply_list(conn: &rusqlite::Connection, migrations: &[&str]) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in migrations.iter().enumerate() {
        let version = (i + 1) as i64;
        if version > current {
            // Schema changes and their version marker must land together. A
            // failed statement rolls the whole migration back, so a restart
            // never retries against a half-created schema.
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_migration_rolls_back_schema_and_version() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let migrations = &[
            "CREATE TABLE stable(id INTEGER PRIMARY KEY);",
            "CREATE TABLE partial(id INTEGER); THIS IS NOT SQL;",
        ];

        assert!(apply_list(&conn, migrations).is_err());
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let partial_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='partial'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(partial_exists, 0);
    }

    #[test]
    fn audit_v2_database_reconciles_with_the_review_migration_lineage() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply_list(&conn, &MIGRATIONS[..1]).unwrap();
        // Recreate the audit branch's original 0002: invariant indexes with
        // no interruption-context columns, then mark the historical version.
        conn.execute_batch(MIGRATIONS[8]).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();

        apply(&conn).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        let columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(interruptions)").unwrap();
            let rows = stmt.query_map([], |row| row.get(1)).unwrap();
            rows.collect::<Result<_, _>>().unwrap()
        };
        assert!(columns.iter().any(|column| column == "process_name"));
        assert!(columns.iter().any(|column| column == "browser_domain"));
        let indexes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index'
                 AND name IN ('ux_focus_sessions_one_open', 'ux_commitments_one_active',
                              'ux_breaks_one_open', 'ux_checkin_responses_one_per_checkin')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexes, 4);
    }

    #[test]
    fn invariant_migration_keeps_newest_non_terminal_focus_open() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply_list(&conn, &MIGRATIONS[..8]).unwrap();
        conn.execute(
            "INSERT INTO daily_plans(date, created_at) VALUES('2026-08-29', 1)",
            [],
        )
        .unwrap();
        let plan_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO daily_commitments(
                plan_id, title, status, created_at
             ) VALUES(?1, 'Eligible focus', 'active', 1)",
            [plan_id],
        )
        .unwrap();
        let eligible_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO daily_commitments(
                plan_id, title, status, created_at
             ) VALUES(?1, 'Already completed', 'completed', 1)",
            [plan_id],
        )
        .unwrap();
        let terminal_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO focus_sessions(commitment_id, started_at, created_at)
             VALUES(?1, 100, 100)",
            [eligible_id],
        )
        .unwrap();
        let eligible_focus_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO focus_sessions(commitment_id, started_at, created_at)
             VALUES(?1, 200, 200)",
            [terminal_id],
        )
        .unwrap();

        apply_list(&conn, MIGRATIONS).unwrap();

        let open_focus_id: i64 = conn
            .query_row(
                "SELECT id FROM focus_sessions WHERE ended_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(open_focus_id, eligible_focus_id);
        let eligible_status: String = conn
            .query_row(
                "SELECT status FROM daily_commitments WHERE id=?1",
                [eligible_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(eligible_status, "active");
        let terminal_was_closed: bool = conn
            .query_row(
                "SELECT ended_at IS NOT NULL FROM focus_sessions WHERE commitment_id=?1",
                [terminal_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(terminal_was_closed);
    }
}
