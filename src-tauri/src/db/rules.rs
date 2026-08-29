//! Domain/application rules + AI classification cache (spec §11, §33, §42).

use rusqlite::{params, Connection};

use aos_core::classify::{AppRule, DomainRule, DEFAULT_BLOCKED_DOMAINS};
use aos_core::types::{Classification, ClassificationSource, ClassifyOutcome};

use super::models::{AppRuleRow, DomainRuleRow};
use super::now;
use crate::error::{AppError, AppResult};

/// Seed the default blocked-domain rules once (spec §11 layer 1).
pub fn seed_defaults(conn: &Connection) -> AppResult<()> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM domain_rules WHERE is_default=1",
        [],
        |r| r.get(0),
    )?;
    if count > 0 {
        return Ok(());
    }
    let ts = now();
    for d in DEFAULT_BLOCKED_DOMAINS {
        conn.execute(
            "INSERT INTO domain_rules(domain, classification, only_in_focus, is_default, created_at)
             VALUES(?1,'distracted',1,1,?2)",
            params![d, ts],
        )?;
    }
    Ok(())
}

pub fn list_domain_rules(conn: &Connection) -> AppResult<Vec<DomainRuleRow>> {
    let mut stmt = conn.prepare("SELECT * FROM domain_rules ORDER BY domain")?;
    let rows = stmt.query_map([], |r| {
        Ok(DomainRuleRow {
            id: r.get("id")?,
            domain: r.get("domain")?,
            classification: r.get("classification")?,
            project_id: r.get("project_id")?,
            commitment_id: r.get("commitment_id")?,
            only_in_focus: r.get::<_, i64>("only_in_focus")? != 0,
            is_default: r.get::<_, i64>("is_default")? != 0,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn list_app_rules(conn: &Connection) -> AppResult<Vec<AppRuleRow>> {
    let mut stmt = conn.prepare("SELECT * FROM application_rules ORDER BY process_name")?;
    let rows = stmt.query_map([], |r| {
        Ok(AppRuleRow {
            id: r.get("id")?,
            process_name: r.get("process_name")?,
            classification: r.get("classification")?,
            project_id: r.get("project_id")?,
            commitment_id: r.get("commitment_id")?,
            only_in_focus: r.get::<_, i64>("only_in_focus")? != 0,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn validate_classification(c: &str) -> AppResult<()> {
    Classification::parse(c)
        .filter(|c| !matches!(c, Classification::Idle | Classification::Unknown))
        .map(|_| ())
        .ok_or_else(|| AppError::invalid(format!("Invalid classification: {c}")))
}

/// The one domain validator: normalization + the rules a stored domain rule
/// must satisfy. Callers that preflight BEFORE mutating state must use this
/// same function so nothing passes preflight and fails at insert.
pub fn normalize_valid_domain(domain: &str) -> AppResult<String> {
    let d = domain.trim().trim_start_matches("www.").to_lowercase();
    let valid = !d.is_empty()
        && d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        });
    if !valid {
        return Err(AppError::invalid("Enter a valid domain, e.g. reddit.com"));
    }
    Ok(d)
}

pub fn upsert_domain_rule(
    conn: &Connection,
    domain: &str,
    classification: &str,
    project_id: Option<i64>,
    commitment_id: Option<i64>,
    only_in_focus: bool,
) -> AppResult<i64> {
    validate_classification(classification)?;
    let domain = normalize_valid_domain(domain)?;
    conn.execute(
        "DELETE FROM domain_rules WHERE domain=?1 AND
            COALESCE(project_id,-1)=COALESCE(?2,-1) AND COALESCE(commitment_id,-1)=COALESCE(?3,-1)",
        params![domain, project_id, commitment_id],
    )?;
    conn.execute(
        "INSERT INTO domain_rules(domain, classification, project_id, commitment_id, only_in_focus, created_at)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![domain, classification, project_id, commitment_id, only_in_focus as i64, now()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn upsert_app_rule(
    conn: &Connection,
    process_name: &str,
    classification: &str,
    project_id: Option<i64>,
    commitment_id: Option<i64>,
    only_in_focus: bool,
) -> AppResult<i64> {
    validate_classification(classification)?;
    let process = process_name.trim().to_lowercase();
    if process.is_empty()
        || process.chars().count() > 260
        || process.chars().any(char::is_control)
    {
        return Err(AppError::invalid("Enter a valid process name of 260 characters or fewer."));
    }
    conn.execute(
        "DELETE FROM application_rules WHERE process_name=?1 AND
            COALESCE(project_id,-1)=COALESCE(?2,-1) AND COALESCE(commitment_id,-1)=COALESCE(?3,-1)",
        params![process, project_id, commitment_id],
    )?;
    conn.execute(
        "INSERT INTO application_rules(process_name, classification, project_id, commitment_id,
                                        only_in_focus, created_at)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            process,
            classification,
            project_id,
            commitment_id,
            only_in_focus as i64,
            now()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_domain_rule(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM domain_rules WHERE id=?1", [id])?;
    Ok(())
}

pub fn delete_app_rule(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM application_rules WHERE id=?1", [id])?;
    Ok(())
}

/// Load rules into the core engine's shape.
pub fn load_engine_rules(conn: &Connection) -> AppResult<(Vec<DomainRule>, Vec<AppRule>)> {
    let domain_rules = list_domain_rules(conn)?
        .into_iter()
        .filter_map(|r| {
            Classification::parse(&r.classification).map(|c| DomainRule {
                id: r.id,
                domain: r.domain,
                classification: c,
                project_id: r.project_id,
                commitment_id: r.commitment_id,
                only_in_focus: r.only_in_focus,
            })
        })
        .collect();
    let app_rules = list_app_rules(conn)?
        .into_iter()
        .filter_map(|r| {
            Classification::parse(&r.classification).map(|c| AppRule {
                id: r.id,
                process_name: r.process_name,
                classification: c,
                project_id: r.project_id,
                commitment_id: r.commitment_id,
                only_in_focus: r.only_in_focus,
            })
        })
        .collect();
    Ok((domain_rules, app_rules))
}

// -- AI classification cache (spec §33/§36) ---------------------------------

pub fn cache_get(conn: &Connection, key: &str) -> AppResult<Option<ClassifyOutcome>> {
    let row = conn.query_row(
        "SELECT classification, confidence, reason FROM classification_cache WHERE cache_key=?1",
        [key],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, String>(2)?,
            ))
        },
    );
    match row {
        Ok((class, confidence, reason)) => Ok(Classification::parse(&class).map(|c| ClassifyOutcome {
            classification: c,
            confidence,
            source: ClassificationSource::Cache,
            reason,
        })),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Complete hot-path cache snapshot used by the monitor loop. Keeping this
/// in the engine avoids taking the database mutex while the engine mutex is
/// held. Retention cleanup bounds the persisted table; never omit rows that
/// SQLite still considers valid, or an older hit becomes a duplicate AI call.
pub fn load_cache(
    conn: &Connection,
) -> AppResult<std::collections::HashMap<String, ClassifyOutcome>> {
    let mut stmt = conn.prepare(
        "SELECT cache_key, classification, confidence, reason
         FROM classification_cache ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut cache = std::collections::HashMap::new();
    for row in rows {
        let (key, classification, confidence, reason) = row?;
        if let Some(classification) = Classification::parse(&classification) {
            if confidence.is_finite() {
                cache.insert(
                    key,
                    ClassifyOutcome {
                        classification,
                        confidence: confidence.clamp(0.0, 1.0),
                        source: ClassificationSource::Cache,
                        reason,
                    },
                );
            }
        }
    }
    Ok(cache)
}

pub fn cache_put(conn: &Connection, key: &str, outcome: &ClassifyOutcome) -> AppResult<()> {
    conn.execute(
        "INSERT INTO classification_cache(cache_key, classification, confidence, reason, created_at)
         VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(cache_key) DO UPDATE SET classification=excluded.classification,
            confidence=excluded.confidence, reason=excluded.reason, created_at=excluded.created_at",
        params![
            key,
            outcome.classification.as_str(),
            outcome.confidence,
            outcome.reason,
            now()
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_snapshot_keeps_older_persisted_entries_past_five_thousand() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE classification_cache(
                cache_key TEXT PRIMARY KEY,
                classification TEXT NOT NULL,
                confidence REAL NOT NULL,
                reason TEXT NOT NULL,
                created_at INTEGER NOT NULL
             );",
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO classification_cache(
                        cache_key, classification, confidence, reason, created_at
                     ) VALUES(?1, 'focused', 1.0, 'persisted', ?2)",
                )
                .unwrap();
            for index in 0..5_001_i64 {
                insert.execute(params![format!("key-{index}"), index]).unwrap();
            }
        }
        tx.commit().unwrap();

        let cache = load_cache(&conn).unwrap();
        assert_eq!(cache.len(), 5_001);
        assert_eq!(
            cache.get("key-0").map(|item| item.classification),
            Some(Classification::Focused)
        );
    }
}
