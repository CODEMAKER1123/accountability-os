pub mod engine_data;
pub mod migrations;
pub mod models;
pub mod plans;
pub mod recovery;
pub mod rules;
pub mod scores;
pub mod sessions;
pub mod settings;
pub mod tasks;

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::Connection;

use crate::error::AppResult;

/// Single serialized connection. Fine at this scale: writes are batched by
/// the aggregator and reads are small.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::apply(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::apply(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn with<R>(&self, f: impl FnOnce(&Connection) -> AppResult<R>) -> AppResult<R> {
        let conn = self.conn.lock();
        f(&conn)
    }

    pub fn with_tx<R>(&self, f: impl FnOnce(&rusqlite::Transaction) -> AppResult<R>) -> AppResult<R> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction().map_err(crate::error::AppError::from)?;
        let out = f(&tx)?;
        tx.commit().map_err(crate::error::AppError::from)?;
        Ok(out)
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Local calendar date for a unix timestamp, as YYYY-MM-DD.
pub fn local_date_of(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".into())
}

pub fn today_local() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Minutes since local midnight, for work-hours checks.
pub fn local_minutes_now() -> u32 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    now.hour() * 60 + now.minute()
}

/// Local hour (0-23) for a unix timestamp.
pub fn local_hour_of(ts: i64) -> u8 {
    use chrono::{TimeZone, Timelike};
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.hour() as u8)
        .unwrap_or(0)
}

/// Seconds elapsed within the local hour for a unix timestamp (0–3599).
pub fn local_secs_into_hour(ts: i64) -> u32 {
    use chrono::{TimeZone, Timelike};
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.minute() * 60 + dt.second())
        .unwrap_or(0)
}

/// Local midnight of a calendar date as a unix timestamp, DST-safe: on
/// transition days where 00:00 is ambiguous the earlier instant is used,
/// and where midnight does not exist (spring-forward zones that skip it)
/// the first existing time of the day is used.
fn local_midnight(d: chrono::NaiveDate) -> Option<i64> {
    use chrono::TimeZone;
    for (h, m) in [(0, 0), (1, 0), (2, 0)] {
        if let Some(dt) = chrono::Local
            .from_local_datetime(&d.and_hms_opt(h, m, 0)?)
            .earliest()
        {
            return Some(dt.timestamp());
        }
    }
    None
}

/// Unix timestamp range [start, end) of a local calendar date. The end bound
/// is the NEXT calendar date's local midnight — not start + 24h, which is
/// wrong by an hour on daylight-saving transition days.
pub fn local_day_bounds(date: &str) -> Option<(i64, i64)> {
    let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let start = local_midnight(d)?;
    let end = local_midnight(d + chrono::Duration::days(1))?;
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aos_core::types::{Classification, ClassificationSource, ClassifyOutcome, SessionDraft};

    fn outcome(c: Classification) -> ClassifyOutcome {
        ClassifyOutcome {
            classification: c,
            confidence: 1.0,
            source: ClassificationSource::Rule,
            reason: "test".into(),
        }
    }

    #[test]
    fn migrations_apply_and_tasks_persist() {
        let db = Db::open_in_memory().unwrap();
        let task = db
            .with(|conn| {
                tasks::create(
                    conn,
                    &tasks::TaskInput {
                        title: "Write the playbook".into(),
                        description: "".into(),
                        project_id: None,
                        parent_task_id: None,
                        status: "inbox".into(),
                        priority: "must".into(),
                        estimated_minutes: Some(90),
                        due_date: None,
                        tags: vec!["sales".into()],
                    },
                )
            })
            .unwrap();
        let listed = db.with(|conn| tasks::list(conn, None, None)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, task.id);
        assert_eq!(listed[0].tags, vec!["sales"]);

        let completed = db.with(|conn| tasks::set_status(conn, task.id, "completed")).unwrap();
        assert!(completed.completed_at.is_some());
    }

    #[test]
    fn lock_day_enforces_commitment_rules() {
        let db = Db::open_in_memory().unwrap();
        // Empty commitments rejected.
        let err = db.with_tx(|tx| {
            plans::lock_day(
                tx,
                &plans::LockDayInput {
                    date: "2026-08-29".into(),
                    commitments: vec![],
                    likely_distraction: String::new(),
                    countermeasure: String::new(),
                    most_important_when: "now".into(),
                    interview_answers: serde_json::Value::Null,
                },
            )
        });
        assert!(err.is_err());

        // Vague DONE definition rejected (spec §6 Q3).
        let vague = db.with_tx(|tx| {
            plans::lock_day(
                tx,
                &plans::LockDayInput {
                    date: "2026-08-29".into(),
                    commitments: vec![plans::CommitmentInput {
                        task_id: None,
                        title: "Sales".into(),
                        done_definition: "do it".into(),
                        estimated_minutes: Some(60),
                        priority: "must".into(),
                        steps: vec![],
                    }],
                    likely_distraction: String::new(),
                    countermeasure: String::new(),
                    most_important_when: "now".into(),
                    interview_answers: serde_json::Value::Null,
                },
            )
        });
        assert!(vague.is_err());

        // A proper contract locks, and re-locking the same day is refused.
        let ok = db.with_tx(|tx| {
            plans::lock_day(
                tx,
                &plans::LockDayInput {
                    date: "2026-08-29".into(),
                    commitments: vec![plans::CommitmentInput {
                        task_id: None,
                        title: "Finish playbook".into(),
                        done_definition: "10-page playbook finished and sent to the team".into(),
                        estimated_minutes: Some(90),
                        priority: "must".into(),
                        steps: vec!["Finish the draft".into(), "Send it to the team".into()],
                    }],
                    likely_distraction: "Email".into(),
                    countermeasure: "Capture and return".into(),
                    most_important_when: "now".into(),
                    interview_answers: serde_json::Value::Null,
                },
            )
        });
        let (plan, commitments) = ok.unwrap();
        assert!(plan.locked_at.is_some());
        assert_eq!(commitments.len(), 1);
        let relock = db.with_tx(|tx| {
            plans::lock_day(
                tx,
                &plans::LockDayInput {
                    date: "2026-08-29".into(),
                    commitments: vec![plans::CommitmentInput {
                        task_id: None,
                        title: "Other".into(),
                        done_definition: "something else entirely done".into(),
                        estimated_minutes: None,
                        priority: "must".into(),
                        steps: vec![],
                    }],
                    likely_distraction: String::new(),
                    countermeasure: String::new(),
                    most_important_when: "now".into(),
                    interview_answers: serde_json::Value::Null,
                },
            )
        });
        assert!(relock.is_err(), "locked days stay locked");
    }

    #[test]
    fn sessions_feed_day_score() {
        let db = Db::open_in_memory().unwrap();
        let date = today_local();
        let (day_start, _) = local_day_bounds(&date).unwrap();
        let t0 = day_start + 9 * 3600;
        let mk = |off_start: i64, off_end: i64, app: &str, idle: bool| SessionDraft {
            started_at: t0 + off_start * 60,
            ended_at: t0 + off_end * 60,
            app_name: app.into(),
            process_name: format!("{}.exe", app.to_lowercase()),
            window_title: format!("{app} window"),
            browser_domain: None,
            browser_title: None,
            is_idle: idle,
        };
        db.with(|conn| {
            sessions::insert(conn, &mk(0, 60, "Docs", false), &outcome(Classification::Focused), None, false)?;
            sessions::insert(conn, &mk(60, 90, "Outlook", false), &outcome(Classification::Supporting), None, false)?;
            sessions::insert(conn, &mk(90, 110, "X", false), &outcome(Classification::Distracted), None, false)?;
            sessions::insert(conn, &mk(110, 120, "Idle", true), &outcome(Classification::Idle), None, false)?;
            Ok(())
        })
        .unwrap();

        let score = db.with(|conn| scores::compute_day_score(conn, &date)).unwrap();
        assert_eq!(score.focused_secs, 3600);
        assert_eq!(score.supporting_secs, 1800);
        assert_eq!(score.distracted_secs, 1200);
        assert_eq!(score.idle_secs, 600);
        // Alignment: (3600 + 0.7*1800) / 6600 = 73.6%
        let alignment = score.alignment.unwrap();
        assert!((alignment - 73.63636).abs() < 0.01, "got {alignment}");
        assert_eq!(score.context_switches, 2);
    }

    #[test]
    fn sessions_split_at_local_midnight() {
        let db = Db::open_in_memory().unwrap();
        let today = today_local();
        let yesterday = {
            let d = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d").unwrap()
                - chrono::Duration::days(1);
            d.format("%Y-%m-%d").to_string()
        };
        let (today_start, _) = local_day_bounds(&today).unwrap();
        // 23:50 yesterday → 00:10 today, one continuous session.
        let draft = SessionDraft {
            started_at: today_start - 600,
            ended_at: today_start + 600,
            app_name: "Chrome".into(),
            process_name: "chrome.exe".into(),
            window_title: "Late night doc".into(),
            browser_domain: None,
            browser_title: None,
            is_idle: false,
        };
        db.with(|conn| sessions::insert(conn, &draft, &outcome(Classification::Focused), None, false))
            .unwrap();

        let yesterday_rows = db.with(|conn| sessions::list_for_date(conn, &yesterday)).unwrap();
        let today_rows = db.with(|conn| sessions::list_for_date(conn, &today)).unwrap();
        assert_eq!(yesterday_rows.len(), 1);
        assert_eq!(today_rows.len(), 1);
        assert_eq!(yesterday_rows[0].duration_seconds, 600);
        assert_eq!(today_rows[0].duration_seconds, 600);
        assert_eq!(yesterday_rows[0].ended_at, today_start);
        assert_eq!(today_rows[0].started_at, today_start);
    }

    #[test]
    fn corrections_refresh_stored_scores() {
        let db = Db::open_in_memory().unwrap();
        let date = today_local();
        let (day_start, _) = local_day_bounds(&date).unwrap();
        let draft = SessionDraft {
            started_at: day_start + 9 * 3600,
            ended_at: day_start + 9 * 3600 + 3600,
            app_name: "X".into(),
            process_name: "chrome.exe".into(),
            window_title: "Home / X".into(),
            browser_domain: Some("x.com".into()),
            browser_title: None,
            is_idle: false,
        };
        let id = db
            .with(|conn| sessions::insert(conn, &draft, &outcome(Classification::Distracted), None, false))
            .unwrap()[0];
        // Finalize the day's score with the hour marked distracted.
        db.with(|conn| {
            let score = scores::compute_day_score(conn, &date)?;
            scores::store_day_score(conn, &score)
        })
        .unwrap();
        let before = db.with(|conn| scores::list_scores_range(conn, &date, &date)).unwrap();
        assert_eq!(before[0].distracted_secs, 3600);

        // Correcting the session must refresh the stored row.
        db.with(|conn| {
            sessions::apply_correction(
                conn,
                &sessions::CorrectionRecord {
                    session_id: id,
                    new_classification: "focused".into(),
                    reason: None,
                    commitment_id: None,
                    project_id: None,
                },
            )?;
            scores::refresh_stored_score(conn, &date)
        })
        .unwrap();
        let after = db.with(|conn| scores::list_scores_range(conn, &date, &date)).unwrap();
        assert_eq!(after[0].distracted_secs, 0);
        assert_eq!(after[0].focused_secs, 3600);
    }

    #[test]
    fn manual_correction_survives_late_ai_response() {
        let db = Db::open_in_memory().unwrap();
        let date = today_local();
        let (day_start, _) = local_day_bounds(&date).unwrap();
        let draft = SessionDraft {
            started_at: day_start + 9 * 3600,
            ended_at: day_start + 9 * 3600 + 600,
            app_name: "Chrome".into(),
            process_name: "chrome.exe".into(),
            window_title: "Ambiguous page".into(),
            browser_domain: Some("example.com".into()),
            browser_title: None,
            is_idle: false,
        };
        let id = db
            .with(|conn| {
                sessions::insert(
                    conn,
                    &draft,
                    &outcome(Classification::Unknown),
                    Some(1),
                    true, // pending AI
                )
            })
            .unwrap()[0];
        // User corrects it while the AI request is still in flight.
        db.with(|conn| {
            sessions::apply_correction(
                conn,
                &sessions::CorrectionRecord {
                    session_id: id,
                    new_classification: "focused".into(),
                    reason: None,
                    commitment_id: Some(1),
                    project_id: None,
                },
            )
        })
        .unwrap();
        // The late AI response must NOT overwrite the manual correction.
        let applied = db
            .with(|conn| sessions::update_classification(conn, id, &outcome(Classification::Distracted)))
            .unwrap();
        assert!(!applied, "late AI patch must be a no-op on corrected rows");
        let row = db.with(|conn| sessions::get(conn, id)).unwrap();
        assert_eq!(row.classification, "focused");
        assert_eq!(row.classification_source, "manual");
        assert!(!row.pending_ai);
    }

    #[test]
    fn grouped_corrections_die_with_any_covered_date() {
        let db = Db::open_in_memory().unwrap();
        let today = today_local();
        let yesterday = {
            let d = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d").unwrap()
                - chrono::Duration::days(1);
            d.format("%Y-%m-%d").to_string()
        };
        let (today_start, _) = local_day_bounds(&today).unwrap();
        // A flagged episode spanning midnight: one draft, two dated rows.
        let draft = SessionDraft {
            started_at: today_start - 600,
            ended_at: today_start + 600,
            app_name: "X".into(),
            process_name: "chrome.exe".into(),
            window_title: "Home / X".into(),
            browser_domain: Some("x.com".into()),
            browser_title: None,
            is_idle: false,
        };
        let ids = db
            .with(|conn| sessions::insert(conn, &draft, &outcome(Classification::Distracted), None, false))
            .unwrap();
        assert_eq!(ids.len(), 2);
        // The intervention memory: one grouped copy anchored per date.
        db.with(|conn| {
            for id in &ids {
                conn.execute(
                    "INSERT INTO activity_corrections(session_id, group_id, process_name,
                        browser_domain, normalized_title, commitment_id, project_id,
                        old_classification, new_classification, reason, created_at)
                     VALUES(?1, 1, 'chrome.exe', 'x.com', 'Home / X', NULL, NULL,
                        'distracted', 'supporting', 'Confirmed as work during intervention', ?2)",
                    rusqlite::params![id, now()],
                )?;
            }
            Ok(())
        })
        .unwrap();
        // An unrelated ungrouped correction on the surviving date must stay.
        let keep_id = db
            .with(|conn| {
                sessions::insert(
                    conn,
                    &SessionDraft {
                        started_at: today_start - 4000,
                        ended_at: today_start - 3600,
                        app_name: "Docs".into(),
                        process_name: "docs.exe".into(),
                        window_title: "Playbook".into(),
                        browser_domain: None,
                        browser_title: None,
                        is_idle: false,
                    },
                    &outcome(Classification::Unknown),
                    None,
                    false,
                )
            })
            .unwrap()[0];
        db.with(|conn| {
            sessions::apply_correction(
                conn,
                &sessions::CorrectionRecord {
                    session_id: keep_id,
                    new_classification: "focused".into(),
                    reason: None,
                    commitment_id: None,
                    project_id: None,
                },
            )
        })
        .unwrap();

        // Deleting only TODAY must remove both copies of the grouped memory
        // — the copy anchored to yesterday must not survive it.
        db.with(|conn| sessions::delete_range(conn, &today, &today)).unwrap();
        let remaining: Vec<(Option<i64>, Option<i64>)> = db
            .with(|conn| {
                let mut stmt =
                    conn.prepare("SELECT session_id, group_id FROM activity_corrections")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap();
        assert_eq!(remaining, vec![(Some(keep_id), None)]);
        // Yesterday's activity rows (episode segment + docs) are untouched.
        assert_eq!(db.with(|conn| sessions::list_for_date(conn, &yesterday)).unwrap().len(), 2);
    }

    #[test]
    fn legacy_intervention_corrections_are_purged_on_upgrade() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // A version-3 database as intermediate builds left it: per-date
        // intervention copies with per-row timestamps, plus a singleton
        // that could equally be the orphan of a partially deleted episode
        // — states no regrouping can honor a past deletion for, so the
        // upgrade retires them all.
        for sql in &migrations::MIGRATIONS[..3] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 3).unwrap();
        let today = today_local();
        let (today_start, _) = local_day_bounds(&today).unwrap();
        let draft = SessionDraft {
            started_at: today_start - 600,
            ended_at: today_start + 600,
            app_name: "X".into(),
            process_name: "chrome.exe".into(),
            window_title: "Home / X".into(),
            browser_domain: Some("x.com".into()),
            browser_title: None,
            is_idle: false,
        };
        let ids = sessions::insert(&conn, &draft, &outcome(Classification::Distracted), None, false).unwrap();
        assert_eq!(ids.len(), 2);
        let legacy_copy = |session_id: i64, created_at: i64| {
            conn.execute(
                "INSERT INTO activity_corrections(session_id, process_name, browser_domain,
                    normalized_title, commitment_id, project_id, old_classification,
                    new_classification, reason, created_at)
                 VALUES(?1, 'chrome.exe', 'x.com', 'Home / X', NULL, NULL,
                    'distracted', 'supporting', 'Confirmed as work during intervention', ?2)",
                rusqlite::params![session_id, created_at],
            )
            .unwrap();
        };
        // Event A: a midnight-spanning episode whose copies straddle a
        // second boundary (the legacy build called now() per row).
        let t_a = now();
        for (i, id) in ids.iter().enumerate() {
            legacy_copy(*id, t_a + i as i64);
        }
        // Event B: a lone copy — a one-date episode, or the orphan of one
        // partially deleted under an intermediate build. Undecidable.
        let b_session = sessions::insert(
            &conn,
            &SessionDraft {
                started_at: today_start - 7200,
                ended_at: today_start - 6600,
                ..draft.clone()
            },
            &outcome(Classification::Distracted),
            None,
            false,
        )
        .unwrap()[0];
        legacy_copy(b_session, t_a - 3600);
        // A user's Activity-view correction is real data and must survive.
        sessions::apply_correction(
            &conn,
            &sessions::CorrectionRecord {
                session_id: b_session,
                new_classification: "focused".into(),
                reason: None,
                commitment_id: None,
                project_id: None,
            },
        )
        .unwrap();

        migrations::apply(&conn).unwrap();

        // Every legacy intervention memory is retired; the manual
        // correction is untouched.
        let rows: Vec<(Option<i64>, Option<String>)> = {
            let mut stmt = conn
                .prepare("SELECT group_id, reason FROM activity_corrections")
                .unwrap();
            let r = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            r.collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, None);
        assert_ne!(rows[0].1.as_deref(), Some("Confirmed as work during intervention"));
    }

    #[test]
    fn prune_expires_correction_metadata_with_sessions() {
        let db = Db::open_in_memory().unwrap();
        let old_end = now() - 40 * 86400;
        let mk = |start: i64, end: i64| SessionDraft {
            started_at: start,
            ended_at: end,
            app_name: "X".into(),
            process_name: "chrome.exe".into(),
            window_title: "Home / X".into(),
            browser_domain: Some("x.com".into()),
            browser_title: None,
            is_idle: false,
        };
        let (old_id, recent_id) = db
            .with(|conn| {
                let old = sessions::insert(conn, &mk(old_end - 600, old_end), &outcome(Classification::Distracted), None, false)?[0];
                let recent = sessions::insert(conn, &mk(now() - 600, now()), &outcome(Classification::Distracted), None, false)?[0];
                // One grouped memory with a copy on each side of the cutoff,
                // and an ungrouped correction on the recent session.
                for id in [old, recent] {
                    conn.execute(
                        "INSERT INTO activity_corrections(session_id, group_id, process_name,
                            browser_domain, normalized_title, commitment_id, project_id,
                            old_classification, new_classification, reason, created_at)
                         VALUES(?1, 9, 'chrome.exe', 'x.com', 'Home / X', NULL, NULL,
                            'distracted', 'supporting', 'Confirmed as work during intervention', ?2)",
                        rusqlite::params![id, now()],
                    )?;
                }
                Ok((old, recent))
            })
            .unwrap();
        db.with(|conn| {
            sessions::apply_correction(
                conn,
                &sessions::CorrectionRecord {
                    session_id: recent_id,
                    new_classification: "focused".into(),
                    reason: None,
                    commitment_id: None,
                    project_id: None,
                },
            )
        })
        .unwrap();

        // Retention: the expired session takes its correction metadata with
        // it — every copy of the grouped memory, but not the recent
        // ungrouped correction.
        db.with(|conn| sessions::prune_older_than(conn, 30)).unwrap();
        let rows: Vec<(Option<i64>, Option<i64>)> = db
            .with(|conn| {
                let mut stmt =
                    conn.prepare("SELECT session_id, group_id FROM activity_corrections")?;
                let r = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(r.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap();
        assert_eq!(rows, vec![(Some(recent_id), None)]);
        // Everything past the retention cutoff is gone.
        let expired_left: i64 = db
            .with(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM activity_sessions WHERE ended_at < ?1",
                    [now() - 30 * 86400],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(expired_left, 0);
        let _ = old_id;
    }

    #[test]
    fn prune_invalidates_derived_data_for_partial_cutoff_day() {
        let db = Db::open_in_memory().unwrap();
        let cutoff_date = today_local();
        let plan_id = db
            .with(|conn| {
                conn.execute(
                    "INSERT INTO daily_plans(date, created_at) VALUES(?1, ?2)",
                    rusqlite::params![cutoff_date, now()],
                )?;
                let plan_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO daily_reviews(plan_id, reviewed_at, ai_summary, created_at)
                     VALUES(?1, ?2, 'Stale partial-day summary', ?2)",
                    rusqlite::params![plan_id, now()],
                )?;
                conn.execute(
                    "INSERT INTO daily_scores(date, total, computed_at) VALUES(?1, 88.0, ?2)",
                    rusqlite::params![cutoff_date, now()],
                )?;
                Ok(plan_id)
            })
            .unwrap();

        // A zero-day window makes the cutoff fall inside today. Whole-day
        // derived values must be discarded because part of their source
        // interval has just been pruned.
        db.with(|conn| sessions::prune_older_than(conn, 0)).unwrap();
        db.with(|conn| {
            let score_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM daily_scores WHERE date=?1",
                [&cutoff_date],
                |row| row.get(0),
            )?;
            assert_eq!(score_count, 0);
            assert!(engine_data::review_summary(conn, plan_id)?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn retention_and_delete_all_remove_focus_and_break_history() {
        let db = Db::open_in_memory().unwrap();
        let old_end = now() - 40 * 86400;
        let recent_end = now() - 60;
        let (old_focus, recent_focus, old_break, recent_break) = db
            .with(|conn| {
                conn.execute(
                    "INSERT INTO daily_plans(date, created_at) VALUES('2000-01-01', 1)",
                    [],
                )?;
                let plan_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO daily_commitments(plan_id, title, created_at)
                     VALUES(?1, 'Retention target', 1)",
                    [plan_id],
                )?;
                let commitment_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO focus_sessions(
                        commitment_id, started_at, ended_at, outcome, created_at
                     ) VALUES(?1, ?2, ?3, 'completed', ?2)",
                    rusqlite::params![commitment_id, old_end - 600, old_end],
                )?;
                let old_focus = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO focus_sessions(
                        commitment_id, started_at, ended_at, outcome, created_at
                     ) VALUES(?1, ?2, ?3, 'completed', ?2)",
                    rusqlite::params![commitment_id, recent_end - 600, recent_end],
                )?;
                let recent_focus = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO breaks(started_at, planned_end_at, actual_end_at, created_at)
                     VALUES(?1, ?2, ?2, ?1)",
                    rusqlite::params![old_end - 300, old_end],
                )?;
                let old_break = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO breaks(started_at, planned_end_at, actual_end_at, created_at)
                     VALUES(?1, ?2, ?2, ?1)",
                    rusqlite::params![recent_end - 300, recent_end],
                )?;
                let recent_break = conn.last_insert_rowid();
                Ok((old_focus, recent_focus, old_break, recent_break))
            })
            .unwrap();

        db.with(|conn| sessions::prune_older_than(conn, 30)).unwrap();
        db.with(|conn| {
            let focus_ids: Vec<i64> = {
                let mut stmt = conn.prepare("SELECT id FROM focus_sessions ORDER BY id")?;
                let rows = stmt.query_map([], |row| row.get(0))?;
                rows.collect::<Result<_, _>>()?
            };
            let break_ids: Vec<i64> = {
                let mut stmt = conn.prepare("SELECT id FROM breaks ORDER BY id")?;
                let rows = stmt.query_map([], |row| row.get(0))?;
                rows.collect::<Result<_, _>>()?
            };
            assert_eq!(focus_ids, vec![recent_focus]);
            assert_eq!(break_ids, vec![recent_break]);
            Ok(())
        })
        .unwrap();
        let _ = (old_focus, old_break);

        db.with(sessions::delete_all).unwrap();
        db.with(|conn| {
            for table in ["focus_sessions", "breaks"] {
                let count: i64 = conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table}"),
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(count, 0, "{table} should be erased by delete_all");
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn activity_deletion_finds_the_intervention_that_survived() {
        let db = Db::open_in_memory().unwrap();
        let today = today_local();
        let (today_start, _) = local_day_bounds(&today).unwrap();
        let prior_date = local_date_of(today_start - 1);
        let create_at = |conn: &Connection, started_at: i64| -> AppResult<i64> {
            let id = engine_data::create_interruption(
                conn,
                "intervention",
                None,
                &engine_data::InterruptionContext {
                    app_name: "Browser",
                    process_name: "browser.exe",
                    browser_domain: Some("example.test"),
                    window_title: "Unanswered prompt",
                },
                420,
                None,
            )?;
            conn.execute(
                "UPDATE interruptions SET started_at=?1 WHERE id=?2",
                rusqlite::params![started_at, id],
            )?;
            Ok(id)
        };

        let (prior_id, current_id) = db
            .with(|conn| {
                Ok((
                    create_at(conn, today_start - 1)?,
                    create_at(conn, today_start + 1)?,
                ))
            })
            .unwrap();
        assert_eq!(
            db.with(engine_data::open_interruption)
                .unwrap()
                .map(|row| row.id),
            Some(current_id)
        );

        db.with_tx(|tx| sessions::delete_range(tx, &today, &today))
            .unwrap();
        assert_eq!(
            db.with(engine_data::open_interruption)
                .unwrap()
                .map(|row| row.id),
            Some(prior_id),
            "deleting today must preserve yesterday's unanswered prompt"
        );

        let replacement_current = db
            .with(|conn| create_at(conn, today_start + 2))
            .unwrap();
        db.with_tx(|tx| sessions::delete_range(tx, &prior_date, &prior_date))
            .unwrap();
        assert_eq!(
            db.with(engine_data::open_interruption)
                .unwrap()
                .map(|row| row.id),
            Some(replacement_current),
            "deleting yesterday must preserve today's unanswered prompt"
        );
    }

    #[test]
    fn activity_deletion_erases_interventions_whose_episode_crosses_midnight() {
        let db = Db::open_in_memory().unwrap();
        let today = today_local();
        let (today_start, _) = local_day_bounds(&today).unwrap();
        let prior_date = local_date_of(today_start - 1);
        let context = engine_data::InterruptionContext {
            app_name: "Browser",
            process_name: "browser.exe",
            browser_domain: Some("example.test"),
            window_title: "Cross-midnight distraction",
        };

        let (spanning_id, today_only_id) = db
            .with(|conn| {
                let spanning_id = engine_data::create_interruption(
                    conn,
                    "intervention",
                    None,
                    &context,
                    420,
                    Some(today_start - 60),
                )?;
                conn.execute(
                    "UPDATE interruptions SET started_at=?1 WHERE id=?2",
                    rusqlite::params![today_start + 60, spanning_id],
                )?;
                let today_only_id = engine_data::create_interruption(
                    conn,
                    "intervention",
                    None,
                    &context,
                    420,
                    Some(today_start + 120),
                )?;
                conn.execute(
                    "UPDATE interruptions SET started_at=?1 WHERE id=?2",
                    rusqlite::params![today_start + 180, today_only_id],
                )?;
                Ok((spanning_id, today_only_id))
            })
            .unwrap();

        db.with_tx(|tx| sessions::delete_range(tx, &prior_date, &prior_date))
            .unwrap();
        db.with(|conn| {
            let spanning_exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM interruptions WHERE id=?1)",
                [spanning_id],
                |row| row.get(0),
            )?;
            let today_only_exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM interruptions WHERE id=?1)",
                [today_only_id],
                |row| row.get(0),
            )?;
            assert!(!spanning_exists, "the prior day's episode metadata must be erased");
            assert!(today_only_exists, "unrelated current-day history must survive");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn recovery_completion_never_retargets_an_older_or_deleted_intervention() {
        let db = Db::open_in_memory().unwrap();
        let context = engine_data::InterruptionContext {
            app_name: "Browser",
            process_name: "browser.exe",
            browser_domain: Some("example.test"),
            window_title: "Recovery target",
        };
        let (completed_id, deleted_id) = db
            .with(|conn| {
                let completed_id = engine_data::create_interruption(
                    conn,
                    "intervention",
                    None,
                    &context,
                    420,
                    None,
                )?;
                engine_data::respond_interruption(conn, completed_id, "return", None)?;
                assert!(engine_data::record_recovery(conn, completed_id, 30)?);

                let deleted_id = engine_data::create_interruption(
                    conn,
                    "intervention",
                    None,
                    &context,
                    480,
                    None,
                )?;
                engine_data::respond_interruption(conn, deleted_id, "return", None)?;
                conn.execute("DELETE FROM interruptions WHERE id=?1", [deleted_id])?;
                Ok((completed_id, deleted_id))
            })
            .unwrap();

        db.with(|conn| {
            assert!(engine_data::get_interruption(conn, deleted_id).is_err());
            assert!(
                !engine_data::record_recovery(conn, deleted_id, 90)?,
                "a deleted recovery must not be applied to an older completed row"
            );
            let completed = engine_data::get_interruption(conn, completed_id)?;
            assert_eq!(completed.recovery_secs, Some(30));
            assert!(completed.returned_at.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn corrections_persist_and_feed_the_matcher() {
        let db = Db::open_in_memory().unwrap();
        let date = today_local();
        let (day_start, _) = local_day_bounds(&date).unwrap();
        let draft = SessionDraft {
            started_at: day_start + 10 * 3600,
            ended_at: day_start + 10 * 3600 + 600,
            app_name: "Chrome".into(),
            process_name: "chrome.exe".into(),
            window_title: "Tauri tutorial - YouTube".into(),
            browser_domain: Some("youtube.com".into()),
            browser_title: Some("Tauri tutorial - YouTube".into()),
            is_idle: false,
        };
        let id = db
            .with(|conn| sessions::insert(conn, &draft, &outcome(Classification::Distracted), Some(7), false))
            .unwrap()[0];
        let updated = db
            .with(|conn| {
                sessions::apply_correction(
                    conn,
                    &sessions::CorrectionRecord {
                        session_id: id,
                        new_classification: "focused".into(),
                        reason: Some("Training video".into()),
                        commitment_id: Some(7),
                        project_id: None,
                    },
                )
            })
            .unwrap();
        assert_eq!(updated.classification, "focused");
        assert_eq!(updated.classification_source, "manual");

        let corrections = db.with(sessions::load_corrections).unwrap();
        assert_eq!(corrections.len(), 1);
        let matcher = aos_core::classify::CorrectionMatcher { corrections };
        let ctx = aos_core::types::ActivityContext {
            app_name: "Chrome".into(),
            process_name: "chrome.exe".into(),
            window_title: "Tauri tutorial - YouTube".into(),
            browser_domain: Some("youtube.com".into()),
            browser_title: None,
            commitment_id: Some(7),
            project_id: None,
            in_focus_session: true,
            is_idle: false,
        };
        let hit = matcher.title_match(&ctx).expect("correction should match");
        assert_eq!(hit.classification, Classification::Focused);
    }

    #[test]
    fn focus_state_has_one_active_row_and_targeted_completion_is_safe() {
        let db = Db::open_in_memory().unwrap();
        let date = today_local();
        let (_, commitments) = db
            .with_tx(|tx| {
                plans::lock_day(
                    tx,
                    &plans::LockDayInput {
                        date,
                        commitments: vec![
                            plans::CommitmentInput {
                                task_id: None,
                                title: "Ship the audit".into(),
                                done_definition: "The reviewed audit is published to the team.".into(),
                                estimated_minutes: Some(60),
                                priority: "must".into(),
                                steps: vec![],
                            },
                            plans::CommitmentInput {
                                task_id: None,
                                title: "Repair the release".into(),
                                done_definition: "The repaired release passes every local check.".into(),
                                estimated_minutes: Some(90),
                                priority: "should".into(),
                                steps: vec![],
                            },
                        ],
                        likely_distraction: "Chat".into(),
                        countermeasure: "Capture it, then return".into(),
                        most_important_when: "now".into(),
                        interview_answers: serde_json::Value::Null,
                    },
                )
            })
            .unwrap();

        db.with_tx(|tx| {
            plans::activate_commitment(tx, commitments[0].id)?;
            engine_data::start_focus(tx, commitments[0].id)?;
            Ok(())
        })
        .unwrap();
        db.with_tx(|tx| {
            plans::activate_commitment(tx, commitments[1].id)?;
            engine_data::start_focus(tx, commitments[1].id)?;
            Ok(())
        })
        .unwrap();

        db.with(|conn| {
            let active_rows: i64 = conn.query_row(
                "SELECT COUNT(*) FROM daily_commitments WHERE status='active'",
                [],
                |row| row.get(0),
            )?;
            let open_focus_rows: i64 = conn.query_row(
                "SELECT COUNT(*) FROM focus_sessions WHERE ended_at IS NULL",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(active_rows, 1);
            assert_eq!(open_focus_rows, 1);
            assert!(engine_data::end_focus_for_commitment(
                conn,
                commitments[0].id,
                "completed"
            )?
            .is_none());
            assert_eq!(
                engine_data::active_focus(conn)?.unwrap().commitment_id,
                commitments[1].id
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reopening_a_completed_commitment_reopens_its_linked_task_consistently() {
        let db = Db::open_in_memory().unwrap();
        let task = db
            .with(|conn| {
                tasks::create(
                    conn,
                    &tasks::TaskInput {
                        title: "Linked task".into(),
                        description: String::new(),
                        project_id: None,
                        parent_task_id: None,
                        status: "planned".into(),
                        priority: "must".into(),
                        estimated_minutes: Some(30),
                        due_date: None,
                        tags: vec![],
                    },
                )
            })
            .unwrap();
        let (_, commitments) = db
            .with_tx(|tx| {
                plans::lock_day(
                    tx,
                    &plans::LockDayInput {
                        date: today_local(),
                        commitments: vec![plans::CommitmentInput {
                            task_id: Some(task.id),
                            title: task.title.clone(),
                            done_definition: "The linked task has a verified final result.".into(),
                            estimated_minutes: Some(30),
                            priority: "must".into(),
                            steps: vec![],
                        }],
                        likely_distraction: String::new(),
                        countermeasure: String::new(),
                        most_important_when: "now".into(),
                        interview_answers: serde_json::Value::Null,
                    },
                )
            })
            .unwrap();
        db.with(|conn| {
            let completed = plans::set_commitment_status(
                conn,
                commitments[0].id,
                "completed",
                None,
                None,
            )?;
            assert!(completed.completed_at.is_some());
            assert_eq!(tasks::get(conn, task.id)?.status, "completed");

            let reopened = plans::set_commitment_status(
                conn,
                commitments[0].id,
                "pending",
                Some("review_correction"),
                None,
            )?;
            assert!(reopened.completed_at.is_none());
            let reopened_task = tasks::get(conn, task.id)?;
            assert_eq!(reopened_task.status, "committed");
            assert!(reopened_task.completed_at.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn invalid_manual_correction_is_rejected_without_partial_write() {
        let db = Db::open_in_memory().unwrap();
        let date = today_local();
        let (day_start, _) = local_day_bounds(&date).unwrap();
        let draft = SessionDraft {
            started_at: day_start + 3600,
            ended_at: day_start + 3660,
            app_name: "Chrome".into(),
            process_name: "chrome.exe".into(),
            window_title: "Social feed".into(),
            browser_domain: Some("example.test".into()),
            browser_title: None,
            is_idle: false,
        };
        let id = db
            .with(|conn| {
                sessions::insert(
                    conn,
                    &draft,
                    &outcome(Classification::Distracted),
                    None,
                    false,
                )
            })
            .unwrap()[0];

        let result = db.with(|conn| {
            sessions::apply_correction(
                conn,
                &sessions::CorrectionRecord {
                    session_id: id,
                    new_classification: "idle".into(),
                    reason: None,
                    commitment_id: None,
                    project_id: None,
                },
            )
        });
        assert!(result.is_err());
        db.with(|conn| {
            assert_eq!(sessions::get(conn, id)?.classification, "distracted");
            let correction_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM activity_corrections",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(correction_count, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delayed_ai_result_cannot_overwrite_a_manual_correction() {
        let db = Db::open_in_memory().unwrap();
        let date = today_local();
        let (day_start, _) = local_day_bounds(&date).unwrap();
        let draft = SessionDraft {
            started_at: day_start + 7200,
            ended_at: day_start + 7260,
            app_name: "Chrome".into(),
            process_name: "chrome.exe".into(),
            window_title: "Ambiguous page".into(),
            browser_domain: Some("example.test".into()),
            browser_title: None,
            is_idle: false,
        };
        let id = db
            .with(|conn| {
                sessions::insert(
                    conn,
                    &draft,
                    &outcome(Classification::Unknown),
                    None,
                    true,
                )
            })
            .unwrap()[0];
        db.with(|conn| {
            sessions::apply_correction(
                conn,
                &sessions::CorrectionRecord {
                    session_id: id,
                    new_classification: "focused".into(),
                    reason: Some("User knows the context".into()),
                    commitment_id: None,
                    project_id: None,
                },
            )?;
            assert!(!sessions::update_classification(
                conn,
                id,
                &outcome(Classification::Distracted)
            )?);
            let row = sessions::get(conn, id)?;
            assert_eq!(row.classification, "focused");
            assert_eq!(row.classification_source, "manual");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn repeated_break_and_checkin_actions_are_idempotent() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            let first = engine_data::start_break(conn, 300)?;
            let second = engine_data::start_break(conn, 600)?;
            assert_ne!(first.id, second.id);
            assert!(engine_data::open_break(conn)?.is_some_and(|row| row.id == second.id));
            let open_breaks: i64 = conn.query_row(
                "SELECT COUNT(*) FROM breaks WHERE actual_end_at IS NULL",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(open_breaks, 1);
            assert!(!engine_data::close_open_break_at(conn, first.id, now())?);
            assert!(engine_data::close_open_break_at(
                conn,
                second.id,
                second.planned_end_at
            )?);
            assert!(!engine_data::close_open_break_at(conn, second.id, now())?);
            let actual_end_at: i64 = conn.query_row(
                "SELECT actual_end_at FROM breaks WHERE id=?1",
                [second.id],
                |row| row.get(0),
            )?;
            assert_eq!(actual_end_at, second.planned_end_at);

            let checkin_id = engine_data::create_checkin(conn, now(), None, &serde_json::json!({}))?;
            engine_data::answer_checkin(conn, checkin_id, "yes", None)?;
            engine_data::answer_checkin(conn, checkin_id, "blocked", Some("retry"))?;
            let checkin = engine_data::get_checkin(conn, checkin_id)?;
            assert_eq!(checkin.response.as_deref(), Some("yes"));
            let answers: i64 = conn.query_row(
                "SELECT COUNT(*) FROM checkin_responses WHERE checkin_id=?1",
                [checkin_id],
                |row| row.get(0),
            )?;
            assert_eq!(answers, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn deleting_activity_removes_derived_private_data_but_keeps_plans() {
        let db = Db::open_in_memory().unwrap();
        let date = today_local();
        let (day_start, _) = local_day_bounds(&date).unwrap();
        let (plan, commitments) = db
            .with_tx(|tx| {
                plans::lock_day(
                    tx,
                    &plans::LockDayInput {
                        date: date.clone(),
                        commitments: vec![plans::CommitmentInput {
                            task_id: None,
                            title: "Preserve this plan".into(),
                            done_definition: "The plan remains after monitoring data is erased.".into(),
                            estimated_minutes: Some(30),
                            priority: "must".into(),
                            steps: vec![],
                        }],
                        likely_distraction: String::new(),
                        countermeasure: String::new(),
                        most_important_when: "flexible".into(),
                        interview_answers: serde_json::Value::Null,
                    },
                )
            })
            .unwrap();
        let session_id = db
            .with(|conn| {
                sessions::insert(
                    conn,
                    &SessionDraft {
                        started_at: day_start + 60,
                        ended_at: day_start + 120,
                        app_name: "PrivateApp".into(),
                        process_name: "private.exe".into(),
                        window_title: "Sensitive title".into(),
                        browser_domain: None,
                        browser_title: None,
                        is_idle: false,
                    },
                    &outcome(Classification::Focused),
                    Some(commitments[0].id),
                    false,
                )
            })
            .unwrap()[0];
        db.with(|conn| {
            sessions::apply_correction(
                conn,
                &sessions::CorrectionRecord {
                    session_id,
                    new_classification: "supporting".into(),
                    reason: Some("Sensitive correction".into()),
                    commitment_id: Some(commitments[0].id),
                    project_id: None,
                },
            )?;
            let cache_key = aos_core::classify::cache_key(
                Some(commitments[0].id),
                "private.exe",
                None,
                "Sensitive title",
            );
            conn.execute(
                "INSERT INTO classification_cache(cache_key, classification, confidence, reason, created_at)
                 VALUES(?1, 'focused', 1.0, 'sensitive', ?2)",
                rusqlite::params![cache_key, day_start - 1],
            )?;
            engine_data::create_interruption(
                conn,
                "intervention",
                Some(commitments[0].id),
                &engine_data::InterruptionContext {
                    app_name: "PrivateApp",
                    process_name: "private.exe",
                    browser_domain: None,
                    window_title: "Sensitive title",
                },
                420,
                None,
            )?;
            let checkin = engine_data::create_checkin(
                conn,
                day_start + 10,
                Some(commitments[0].id),
                &serde_json::json!({"focused_secs": 60}),
            )?;
            engine_data::answer_checkin(conn, checkin, "yes", None)?;
            engine_data::start_focus(conn, commitments[0].id)?;
            engine_data::start_break(conn, 900)?;
            engine_data::upsert_review(conn, plan.id, Some("Sensitive AI summary"))?;
            let score = scores::compute_day_score(conn, &date)?;
            scores::store_day_score(conn, &score)?;
            conn.execute(
                "INSERT INTO ai_insights(period, metric, text, source, created_at)
                 VALUES('week', 'private', 'Sensitive narrative', 'ai', ?1)",
                [day_start + 1],
            )?;
            Ok(())
        })
        .unwrap();

        assert_eq!(
            db.with_tx(|tx| sessions::delete_range(tx, &date, &date))
                .unwrap(),
            1
        );
        db.with(|conn| {
            for table in [
                "activity_sessions",
                "activity_corrections",
                "classification_cache",
                "interruptions",
                "checkins",
                "checkin_responses",
                "focus_sessions",
                "breaks",
                "daily_scores",
                "ai_insights",
            ] {
                let count: i64 = conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table}"),
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(count, 0, "{table} should be erased");
            }
            assert_eq!(plans::list_commitments(conn, plan.id)?.len(), 1);
            assert!(engine_data::review_summary(conn, plan.id)?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn corrupt_settings_are_reported_instead_of_silently_reset() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            conn.execute(
                "INSERT INTO settings(key, value) VALUES('app_settings', '{not-json')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(db.with(settings::load).is_err());
    }
}
