use crate::db::get_db;
use crate::errors::{DialError, Result};
use crate::output::{blue, bold, dim, yellow};
use rusqlite::Connection;

pub const LEARNING_CATEGORIES: &[&str] = &["build", "test", "setup", "gotcha", "pattern", "tool", "other"];

pub fn add_learning(description: &str, category: Option<&str>) -> Result<i64> {
    add_learning_linked(description, category, None, None)
}

pub fn add_learning_linked(
    description: &str,
    category: Option<&str>,
    pattern_id: Option<i64>,
    iteration_id: Option<i64>,
) -> Result<i64> {
    let conn = get_db(None)?;
    add_learning_with_conn(&conn, description, category, pattern_id, iteration_id)
}

pub fn add_learning_with_conn(
    conn: &Connection,
    description: &str,
    category: Option<&str>,
    pattern_id: Option<i64>,
    iteration_id: Option<i64>,
) -> Result<i64> {
    // Validate category
    let category = match category {
        Some(c) if LEARNING_CATEGORIES.contains(&c) => Some(c),
        Some(c) => {
            println!("{}", yellow(&format!("Warning: Unknown category '{}'. Using 'other'.", c)));
            Some("other")
        }
        None => None,
    };

    conn.execute(
        "INSERT INTO learnings (category, description, pattern_id, iteration_id) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![category, description, pattern_id, iteration_id],
    )?;

    let learning_id = conn.last_insert_rowid();
    Ok(learning_id)
}

pub fn list_learnings(category: Option<&str>) -> Result<()> {
    let conn = get_db(None)?;

    let rows: Vec<(i64, Option<String>, String, String, i64)> = if let Some(cat) = category {
        let mut stmt = conn.prepare(
            "SELECT id, category, description, discovered_at, times_referenced
             FROM learnings WHERE category = ?1
             ORDER BY discovered_at DESC",
        )?;
        let result = stmt.query_map([cat], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        result
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, category, description, discovered_at, times_referenced
             FROM learnings ORDER BY discovered_at DESC",
        )?;
        let result = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        result
    };

    if rows.is_empty() {
        println!("{}", dim("No learnings recorded."));
        return Ok(());
    }

    let title = if let Some(cat) = category {
        format!("Learnings ({})", cat)
    } else {
        "Learnings".to_string()
    };

    println!("{}", bold(&title));
    println!("{}", "=".repeat(60));

    for (id, cat, description, discovered_at, times_referenced) in rows {
        let cat_str = cat
            .map(|c| format!("[{}]", c))
            .unwrap_or_else(|| "[uncategorized]".to_string());

        let ref_str = if times_referenced > 0 {
            format!("(referenced {}x)", times_referenced)
        } else {
            String::new()
        };

        println!("\n  #{} {} {}", id, blue(&cat_str), ref_str);
        println!("     {}", description);
        println!("{}", dim(&format!("     Discovered: {}", &discovered_at[..10])));
    }

    Ok(())
}

pub fn search_learnings(query: &str) -> Result<Vec<LearningResult>> {
    let conn = get_db(None)?;

    let mut stmt = conn.prepare(
        "SELECT l.id, l.category, l.description, l.times_referenced
         FROM learnings l
         INNER JOIN learnings_fts fts ON l.id = fts.rowid
         WHERE learnings_fts MATCH ?1
         ORDER BY rank LIMIT 10",
    )?;

    let rows: Vec<LearningResult> = stmt
        .query_map([query], |row| {
            Ok(LearningResult {
                id: row.get(0)?,
                category: row.get(1)?,
                description: row.get(2)?,
                times_referenced: row.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        println!("{}", dim(&format!("No learnings matching '{}'.", query)));
        return Ok(rows);
    }

    println!("{}", bold(&format!("Learnings matching '{}':", query)));
    println!("{}", "=".repeat(60));

    for row in &rows {
        let cat_str = row
            .category
            .as_ref()
            .map(|c| format!("[{}]", c))
            .unwrap_or_default();
        println!("\n  #{} {}", row.id, blue(&cat_str));
        println!("     {}", row.description);
    }

    Ok(rows)
}

#[derive(Debug, Clone)]
pub struct LearningResult {
    pub id: i64,
    pub category: Option<String>,
    pub description: String,
    pub times_referenced: i64,
}

pub fn delete_learning(learning_id: i64) -> Result<()> {
    let conn = get_db(None)?;

    let changed = conn.execute("DELETE FROM learnings WHERE id = ?1", [learning_id])?;

    if changed == 0 {
        return Err(DialError::LearningNotFound(learning_id));
    }

    Ok(())
}

pub fn increment_learning_reference(conn: &Connection, learning_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE learnings SET times_referenced = times_referenced + 1 WHERE id = ?1",
        [learning_id],
    )?;
    Ok(())
}

/// Query learnings linked to a specific failure pattern.
pub fn learnings_for_pattern(conn: &Connection, pattern_id: i64) -> Result<Vec<LearningResult>> {
    let mut stmt = conn.prepare(
        "SELECT id, category, description, times_referenced
         FROM learnings
         WHERE pattern_id = ?1
         ORDER BY times_referenced DESC, discovered_at DESC",
    )?;

    let rows: Vec<LearningResult> = stmt
        .query_map([pattern_id], |row| {
            Ok(LearningResult {
                id: row.get(0)?,
                category: row.get(1)?,
                description: row.get(2)?,
                times_referenced: row.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Display learnings for a specific pattern (used by CLI --pattern flag).
pub fn list_learnings_for_pattern(pattern_id: i64) -> Result<()> {
    let conn = get_db(None)?;

    // Get pattern key for the header
    let pattern_key: String = conn
        .query_row(
            "SELECT pattern_key FROM failure_patterns WHERE id = ?1",
            [pattern_id],
            |row| row.get(0),
        )
        .map_err(|_| DialError::UserError(format!("Pattern #{} not found", pattern_id)))?;

    let learnings = learnings_for_pattern(&conn, pattern_id)?;

    if learnings.is_empty() {
        println!("{}", dim(&format!("No learnings linked to pattern '{}' (#{})", pattern_key, pattern_id)));
        return Ok(());
    }

    println!("{}", bold(&format!("Learnings for pattern '{}' (#{})", pattern_key, pattern_id)));
    println!("{}", "=".repeat(60));

    for learning in &learnings {
        let cat_str = learning
            .category
            .as_ref()
            .map(|c| format!("[{}]", c))
            .unwrap_or_else(|| "[uncategorized]".to_string());

        let ref_str = if learning.times_referenced > 0 {
            format!("(referenced {}x)", learning.times_referenced)
        } else {
            String::new()
        };

        println!("\n  #{} {} {}", learning.id, blue(&cat_str), ref_str);
        println!("     {}", learning.description);
    }

    Ok(())
}

/// Auto-link a learning to the most recent failure's pattern for a given iteration.
/// Returns the pattern_id if found.
pub fn auto_link_pattern_for_iteration(conn: &Connection, iteration_id: i64) -> Option<i64> {
    conn.query_row(
        "SELECT f.pattern_id FROM failures f
         WHERE f.iteration_id = ?1
         ORDER BY f.created_at DESC LIMIT 1",
        [iteration_id],
        |row| row.get(0),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    /// Set up an in-memory DB with base schema + migration columns for learnings.
    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .unwrap();
        conn.execute_batch(schema::SCHEMA).unwrap();
        // Add migration 011 columns
        conn.execute_batch(
            r#"
            ALTER TABLE learnings ADD COLUMN pattern_id INTEGER REFERENCES failure_patterns(id);
            ALTER TABLE learnings ADD COLUMN iteration_id INTEGER REFERENCES iterations(id);
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_add_learning_with_conn_no_links() {
        let conn = setup_test_db();
        let id = add_learning_with_conn(&conn, "test learning", Some("pattern"), None, None).unwrap();
        assert!(id > 0);

        let (cat, desc, pid, iid): (Option<String>, String, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT category, description, pattern_id, iteration_id FROM learnings WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(cat.as_deref(), Some("pattern"));
        assert_eq!(desc, "test learning");
        assert!(pid.is_none());
        assert!(iid.is_none());
    }

    #[test]
    fn test_add_learning_with_conn_linked() {
        let conn = setup_test_db();

        // Create a pattern and iteration for linking
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('TestErr', 'Test', 'test')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number) VALUES (?1, 1)",
            [task_id],
        )
        .unwrap();
        let iteration_id = conn.last_insert_rowid();

        let id = add_learning_with_conn(
            &conn,
            "linked learning",
            Some("gotcha"),
            Some(pattern_id),
            Some(iteration_id),
        )
        .unwrap();

        let (pid, iid): (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT pattern_id, iteration_id FROM learnings WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(pid, Some(pattern_id));
        assert_eq!(iid, Some(iteration_id));
    }

    #[test]
    fn test_learnings_for_pattern() {
        let conn = setup_test_db();

        // Create two patterns
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('ErrA', 'Error A', 'build')",
            [],
        )
        .unwrap();
        let pattern_a = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('ErrB', 'Error B', 'test')",
            [],
        )
        .unwrap();
        let pattern_b = conn.last_insert_rowid();

        // Add learnings: 2 linked to pattern A, 1 to pattern B, 1 unlinked
        add_learning_with_conn(&conn, "learning A1", Some("pattern"), Some(pattern_a), None).unwrap();
        add_learning_with_conn(&conn, "learning A2", Some("gotcha"), Some(pattern_a), None).unwrap();
        add_learning_with_conn(&conn, "learning B1", Some("build"), Some(pattern_b), None).unwrap();
        add_learning_with_conn(&conn, "unlinked learning", Some("other"), None, None).unwrap();

        // Query for pattern A
        let results_a = learnings_for_pattern(&conn, pattern_a).unwrap();
        assert_eq!(results_a.len(), 2);
        assert!(results_a.iter().any(|l| l.description == "learning A1"));
        assert!(results_a.iter().any(|l| l.description == "learning A2"));

        // Query for pattern B
        let results_b = learnings_for_pattern(&conn, pattern_b).unwrap();
        assert_eq!(results_b.len(), 1);
        assert_eq!(results_b[0].description, "learning B1");

        // Query for non-existent pattern
        let results_none = learnings_for_pattern(&conn, 999).unwrap();
        assert!(results_none.is_empty());
    }

    #[test]
    fn test_auto_link_pattern_for_iteration() {
        let conn = setup_test_db();

        // Set up task + iteration + pattern + failure
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number) VALUES (?1, 1)",
            [task_id],
        )
        .unwrap();
        let iteration_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('CompileErr', 'Compile', 'build')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'error')",
            rusqlite::params![iteration_id, pattern_id],
        )
        .unwrap();

        // Should find the pattern
        let result = auto_link_pattern_for_iteration(&conn, iteration_id);
        assert_eq!(result, Some(pattern_id));

        // Iteration with no failures returns None
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number) VALUES (?1, 2)",
            [task_id],
        )
        .unwrap();
        let iter2 = conn.last_insert_rowid();
        let result2 = auto_link_pattern_for_iteration(&conn, iter2);
        assert_eq!(result2, None);
    }

    #[test]
    fn test_auto_link_returns_most_recent_pattern() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number) VALUES (?1, 1)",
            [task_id],
        )
        .unwrap();
        let iteration_id = conn.last_insert_rowid();

        // Two patterns
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('ErrOld', 'Old', 'build')",
            [],
        )
        .unwrap();
        let old_pattern = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('ErrNew', 'New', 'test')",
            [],
        )
        .unwrap();
        let new_pattern = conn.last_insert_rowid();

        // Insert two failures — the second (newer) one should be returned
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text, created_at) VALUES (?1, ?2, 'old error', '2026-01-01T00:00:00')",
            rusqlite::params![iteration_id, old_pattern],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text, created_at) VALUES (?1, ?2, 'new error', '2026-01-02T00:00:00')",
            rusqlite::params![iteration_id, new_pattern],
        )
        .unwrap();

        let result = auto_link_pattern_for_iteration(&conn, iteration_id);
        assert_eq!(result, Some(new_pattern));
    }
}
