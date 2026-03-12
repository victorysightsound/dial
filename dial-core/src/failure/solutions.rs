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

/// Find solutions for a pattern that are above the trust threshold.
/// Returns tuples of (solution_id, description, confidence) for use in auto-suggestions.
pub fn find_solutions_for_pattern(conn: &Connection, pattern_id: i64) -> Result<Vec<(i64, String, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT id, description, confidence
         FROM solutions
         WHERE pattern_id = ?1 AND confidence >= ?2
         ORDER BY confidence DESC",
    )?;

    let solutions = stmt
        .query_map(rusqlite::params![pattern_id, TRUST_THRESHOLD], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(solutions)
}

/// Record that a solution was suggested for a failure (creates a solution_application row).
pub fn record_solution_application(
    conn: &Connection,
    solution_id: i64,
    failure_id: i64,
    iteration_id: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO solution_applications (solution_id, failure_id, iteration_id)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![solution_id, failure_id, iteration_id],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Get solution IDs with pending (unresolved) applications for a task.
pub fn get_pending_solution_applications(conn: &Connection, task_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT sa.solution_id
         FROM solution_applications sa
         INNER JOIN failures f ON sa.failure_id = f.id
         INNER JOIN iterations i ON f.iteration_id = i.id
         WHERE i.task_id = ?1 AND sa.success IS NULL",
    )?;

    let ids = stmt
        .query_map([task_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(ids)
}

/// Mark all pending solution applications for a task as successful and
/// increment each solution's confidence. Returns the list of boosted solution IDs.
pub fn mark_solution_applications_success(conn: &Connection, task_id: i64) -> Result<Vec<i64>> {
    let solution_ids = get_pending_solution_applications(conn, task_id)?;

    for &solution_id in &solution_ids {
        apply_solution_success(conn, solution_id)?;

        conn.execute(
            "UPDATE solution_applications SET success = 1
             WHERE solution_id = ?1 AND success IS NULL
             AND failure_id IN (
                 SELECT f.id FROM failures f
                 INNER JOIN iterations i ON f.iteration_id = i.id
                 WHERE i.task_id = ?2
             )",
            rusqlite::params![solution_id, task_id],
        )?;
    }

    Ok(solution_ids)
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use crate::db::schema;

    /// Set up an in-memory DB with schema + migration columns for testing.
    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;").unwrap();
        conn.execute_batch(schema::SCHEMA).unwrap();
        // Apply migration 7 columns needed for solutions
        conn.execute_batch(r#"
            ALTER TABLE solutions ADD COLUMN source TEXT NOT NULL DEFAULT 'auto-learned';
            ALTER TABLE solutions ADD COLUMN last_validated_at TEXT;
            ALTER TABLE solutions ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
            CREATE TABLE IF NOT EXISTS solution_history (
                id INTEGER PRIMARY KEY,
                solution_id INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                old_confidence REAL,
                new_confidence REAL,
                notes TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (solution_id) REFERENCES solutions(id)
            );
        "#).unwrap();
        // Apply migration 8 columns for failure_patterns
        conn.execute_batch(r#"
            ALTER TABLE failure_patterns ADD COLUMN regex_pattern TEXT;
            ALTER TABLE failure_patterns ADD COLUMN status TEXT NOT NULL DEFAULT 'suggested';
        "#).unwrap();
        conn
    }

    /// Insert a task, iteration, failure_pattern, and failure for testing.
    fn seed_test_data(conn: &Connection) -> (i64, i64, i64, i64) {
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('test task', 'in_progress')",
            [],
        ).unwrap();
        let task_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number) VALUES (?1, 1)",
            [task_id],
        ).unwrap();
        let iteration_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('TestError', 'Test error pattern', 'test')",
            [],
        ).unwrap();
        let pattern_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'test error')",
            rusqlite::params![iteration_id, pattern_id],
        ).unwrap();
        let failure_id = conn.last_insert_rowid();

        (task_id, iteration_id, pattern_id, failure_id)
    }

    #[test]
    fn test_find_solutions_for_pattern_no_solutions() {
        let conn = setup_test_db();
        let (_task_id, _iter_id, pattern_id, _failure_id) = seed_test_data(&conn);

        let solutions = find_solutions_for_pattern(&conn, pattern_id).unwrap();
        assert!(solutions.is_empty());
    }

    #[test]
    fn test_find_solutions_for_pattern_below_threshold() {
        let conn = setup_test_db();
        let (_task_id, _iter_id, pattern_id, _failure_id) = seed_test_data(&conn);

        // Insert a solution with low confidence (below TRUST_THRESHOLD=0.6)
        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'low confidence fix', 0.3)",
            [pattern_id],
        ).unwrap();

        let solutions = find_solutions_for_pattern(&conn, pattern_id).unwrap();
        assert!(solutions.is_empty(), "Should not return solutions below threshold");
    }

    #[test]
    fn test_find_solutions_for_pattern_above_threshold() {
        let conn = setup_test_db();
        let (_task_id, _iter_id, pattern_id, _failure_id) = seed_test_data(&conn);

        // Insert a solution above TRUST_THRESHOLD
        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'trusted fix', 0.85)",
            [pattern_id],
        ).unwrap();

        let solutions = find_solutions_for_pattern(&conn, pattern_id).unwrap();
        assert_eq!(solutions.len(), 1);
        assert_eq!(solutions[0].1, "trusted fix");
        assert!((solutions[0].2 - 0.85).abs() < 0.001);
    }

    #[test]
    fn test_find_solutions_for_pattern_sorted_by_confidence() {
        let conn = setup_test_db();
        let (_task_id, _iter_id, pattern_id, _failure_id) = seed_test_data(&conn);

        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'good fix', 0.7)",
            [pattern_id],
        ).unwrap();
        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'best fix', 0.95)",
            [pattern_id],
        ).unwrap();

        let solutions = find_solutions_for_pattern(&conn, pattern_id).unwrap();
        assert_eq!(solutions.len(), 2);
        assert_eq!(solutions[0].1, "best fix");
        assert_eq!(solutions[1].1, "good fix");
    }

    #[test]
    fn test_record_solution_application() {
        let conn = setup_test_db();
        let (_task_id, iteration_id, pattern_id, failure_id) = seed_test_data(&conn);

        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'a fix', 0.8)",
            [pattern_id],
        ).unwrap();
        let solution_id = conn.last_insert_rowid();

        let app_id = record_solution_application(&conn, solution_id, failure_id, iteration_id).unwrap();
        assert!(app_id > 0);

        // Verify the row exists with success=NULL
        let success: Option<i64> = conn.query_row(
            "SELECT success FROM solution_applications WHERE id = ?1",
            [app_id],
            |row| row.get(0),
        ).unwrap();
        assert!(success.is_none(), "success should be NULL initially");
    }

    #[test]
    fn test_get_pending_solution_applications() {
        let conn = setup_test_db();
        let (task_id, iteration_id, pattern_id, failure_id) = seed_test_data(&conn);

        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'fix A', 0.8)",
            [pattern_id],
        ).unwrap();
        let sol_id = conn.last_insert_rowid();

        record_solution_application(&conn, sol_id, failure_id, iteration_id).unwrap();

        let pending = get_pending_solution_applications(&conn, task_id).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], sol_id);
    }

    #[test]
    fn test_mark_solution_applications_success() {
        let conn = setup_test_db();
        let (task_id, iteration_id, pattern_id, failure_id) = seed_test_data(&conn);

        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'fix X', 0.7)",
            [pattern_id],
        ).unwrap();
        let sol_id = conn.last_insert_rowid();

        record_solution_application(&conn, sol_id, failure_id, iteration_id).unwrap();

        // Verify initial confidence
        let initial_conf: f64 = conn.query_row(
            "SELECT confidence FROM solutions WHERE id = ?1",
            [sol_id],
            |row| row.get(0),
        ).unwrap();
        assert!((initial_conf - 0.7).abs() < 0.001);

        // Mark success — should increment confidence by TRUST_INCREMENT
        let boosted = mark_solution_applications_success(&conn, task_id).unwrap();
        assert_eq!(boosted.len(), 1);
        assert_eq!(boosted[0], sol_id);

        // Verify confidence was incremented
        let new_conf: f64 = conn.query_row(
            "SELECT confidence FROM solutions WHERE id = ?1",
            [sol_id],
            |row| row.get(0),
        ).unwrap();
        assert!((new_conf - (0.7 + TRUST_INCREMENT)).abs() < 0.001);

        // Verify application marked as success=1
        let success: i64 = conn.query_row(
            "SELECT success FROM solution_applications WHERE solution_id = ?1",
            [sol_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(success, 1);

        // Pending should now be empty
        let pending = get_pending_solution_applications(&conn, task_id).unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn test_mark_solution_applications_no_pending() {
        let conn = setup_test_db();
        let (task_id, _iter_id, _pattern_id, _failure_id) = seed_test_data(&conn);

        // No solution applications exist
        let boosted = mark_solution_applications_success(&conn, task_id).unwrap();
        assert!(boosted.is_empty());
    }

    #[test]
    fn test_integration_record_failure_with_auto_suggestion() {
        let conn = setup_test_db();
        let (task_id, iteration_id, pattern_id, _failure_id) = seed_test_data(&conn);

        // Insert a trusted solution for the pattern
        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'Use --release flag', 0.8)",
            [pattern_id],
        ).unwrap();
        let sol_id = conn.last_insert_rowid();

        // Record a new failure for the same pattern
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'another test error')",
            rusqlite::params![iteration_id, pattern_id],
        ).unwrap();
        let new_failure_id = conn.last_insert_rowid();

        // Simulate what record_failure does: find solutions and record applications
        let solutions = find_solutions_for_pattern(&conn, pattern_id).unwrap();
        assert_eq!(solutions.len(), 1);
        assert_eq!(solutions[0].0, sol_id);
        assert_eq!(solutions[0].1, "Use --release flag");

        for &(sid, _, _) in &solutions {
            record_solution_application(&conn, sid, new_failure_id, iteration_id).unwrap();
        }

        // Verify pending application exists
        let pending = get_pending_solution_applications(&conn, task_id).unwrap();
        assert_eq!(pending, vec![sol_id]);

        // Simulate successful validation: mark applications as success
        let boosted = mark_solution_applications_success(&conn, task_id).unwrap();
        assert_eq!(boosted, vec![sol_id]);

        // Verify confidence increased from 0.8 to 0.8 + TRUST_INCREMENT
        let final_conf: f64 = conn.query_row(
            "SELECT confidence FROM solutions WHERE id = ?1",
            [sol_id],
            |row| row.get(0),
        ).unwrap();
        assert!((final_conf - (0.8 + TRUST_INCREMENT)).abs() < 0.001);

        // success_count should be incremented
        let success_count: i64 = conn.query_row(
            "SELECT success_count FROM solutions WHERE id = ?1",
            [sol_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(success_count, 1);
    }
}
