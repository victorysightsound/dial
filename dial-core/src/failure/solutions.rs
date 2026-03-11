use crate::errors::Result;
use crate::{TRUST_DECREMENT, TRUST_INCREMENT, TRUST_THRESHOLD};
use chrono::Local;
use rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct Solution {
    pub id: i64,
    pub pattern_id: i64,
    pub description: String,
    pub code_example: Option<String>,
    pub confidence: f64,
    pub success_count: i64,
    pub failure_count: i64,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

impl Solution {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Solution {
            id: row.get("id")?,
            pattern_id: row.get("pattern_id")?,
            description: row.get("description")?,
            code_example: row.get("code_example")?,
            confidence: row.get("confidence")?,
            success_count: row.get("success_count")?,
            failure_count: row.get("failure_count")?,
            created_at: row.get("created_at")?,
            last_used_at: row.get("last_used_at")?,
        })
    }

    pub fn is_trusted(&self) -> bool {
        self.confidence >= TRUST_THRESHOLD
    }
}

pub fn find_trusted_solutions(conn: &Connection, pattern_id: i64) -> Result<Vec<Solution>> {
    let mut stmt = conn.prepare(
        "SELECT id, pattern_id, description, code_example, confidence, success_count, failure_count, created_at, last_used_at
         FROM solutions
         WHERE pattern_id = ?1 AND confidence >= ?2
         ORDER BY confidence DESC",
    )?;

    let solutions = stmt
        .query_map(rusqlite::params![pattern_id, TRUST_THRESHOLD], |row| {
            Solution::from_row(row)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(solutions)
}

pub fn record_solution(
    conn: &Connection,
    pattern_id: i64,
    description: &str,
    code_example: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO solutions (pattern_id, description, code_example)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![pattern_id, description, code_example],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn apply_solution_success(conn: &Connection, solution_id: i64) -> Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "UPDATE solutions
         SET confidence = MIN(1.0, confidence + ?1),
             success_count = success_count + 1,
             last_used_at = ?2
         WHERE id = ?3",
        rusqlite::params![TRUST_INCREMENT, now, solution_id],
    )?;
    Ok(())
}

pub fn apply_solution_failure(conn: &Connection, solution_id: i64) -> Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "UPDATE solutions
         SET confidence = MAX(0.0, confidence - ?1),
             failure_count = failure_count + 1,
             last_used_at = ?2
         WHERE id = ?3",
        rusqlite::params![TRUST_DECREMENT, now, solution_id],
    )?;
    Ok(())
}

/// Record a solution with source tracking.
pub fn record_solution_with_source(
    conn: &Connection,
    pattern_id: i64,
    description: &str,
    code_example: Option<&str>,
    source: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO solutions (pattern_id, description, code_example, source)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![pattern_id, description, code_example, source],
    )?;
    let id = conn.last_insert_rowid();

    // Record creation in history
    record_solution_event(conn, id, "created", None, None, Some(&format!("source: {}", source)))?;

    Ok(id)
}

/// Record an event in solution_history.
pub fn record_solution_event(
    conn: &Connection,
    solution_id: i64,
    event_type: &str,
    old_confidence: Option<f64>,
    new_confidence: Option<f64>,
    notes: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO solution_history (solution_id, event_type, old_confidence, new_confidence, notes)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![solution_id, event_type, old_confidence, new_confidence, notes],
    )?;
    Ok(())
}

/// Apply confidence decay to stale solutions.
/// Decrements confidence by `decay_per_period` for each `period_days` since last validation.
/// Returns the number of solutions decayed.
pub fn apply_confidence_decay(
    conn: &Connection,
    decay_per_period: f64,
    period_days: i64,
) -> Result<usize> {
    let now = Local::now();
    let mut count = 0;

    // Find solutions that haven't been validated within the period
    let mut stmt = conn.prepare(
        "SELECT id, confidence, last_validated_at FROM solutions WHERE confidence > 0.0",
    )?;

    let solutions: Vec<(i64, f64, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();

    for (id, old_confidence, last_validated) in solutions {
        let days_since = match &last_validated {
            Some(ts) => {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                    (now - dt.with_timezone(&chrono::Local)).num_days()
                } else {
                    period_days + 1 // If can't parse, treat as stale
                }
            }
            None => period_days + 1, // Never validated = stale
        };

        if days_since >= period_days {
            let periods = days_since / period_days;
            let total_decay = decay_per_period * periods as f64;
            let new_confidence = (old_confidence - total_decay).max(0.0);

            if (new_confidence - old_confidence).abs() > 0.001 {
                conn.execute(
                    "UPDATE solutions SET confidence = ?1 WHERE id = ?2",
                    rusqlite::params![new_confidence, id],
                )?;

                record_solution_event(
                    conn, id, "decay", Some(old_confidence), Some(new_confidence),
                    Some(&format!("{} days since validation, {} periods of {:.2} decay", days_since, periods, decay_per_period)),
                )?;

                count += 1;
            }
        }
    }

    Ok(count)
}

/// Mark a solution as validated (refreshes its last_validated_at timestamp).
pub fn validate_solution(conn: &Connection, solution_id: i64) -> Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "UPDATE solutions SET last_validated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, solution_id],
    )?;
    record_solution_event(conn, solution_id, "validated", None, None, None)?;
    Ok(())
}

/// Get solution history events.
pub fn get_solution_history(conn: &Connection, solution_id: i64) -> Result<Vec<SolutionEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, old_confidence, new_confidence, notes, created_at
         FROM solution_history WHERE solution_id = ?1 ORDER BY created_at DESC",
    )?;

    let events = stmt
        .query_map([solution_id], |row| {
            Ok(SolutionEvent {
                id: row.get(0)?,
                event_type: row.get(1)?,
                old_confidence: row.get(2)?,
                new_confidence: row.get(3)?,
                notes: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(events)
}

#[derive(Debug, Clone)]
pub struct SolutionEvent {
    pub id: i64,
    pub event_type: String,
    pub old_confidence: Option<f64>,
    pub new_confidence: Option<f64>,
    pub notes: Option<String>,
    pub created_at: String,
}
