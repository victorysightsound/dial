pub mod models;

use crate::db::{get_db, with_transaction};
use crate::errors::{DialError, Result};
use crate::output::{bold, dim, green, red, yellow};
use chrono::Local;
use models::Task;
use rusqlite::Connection;

pub fn task_add(description: &str, priority: i32, spec_section_id: Option<i64>) -> Result<i64> {
    let conn = get_db(None)?;
    conn.execute(
        "INSERT INTO tasks (description, priority, spec_section_id) VALUES (?1, ?2, ?3)",
        rusqlite::params![description, priority, spec_section_id],
    )?;
    let task_id = conn.last_insert_rowid();
    Ok(task_id)
}

pub fn task_list(show_all: bool) -> Result<()> {
    let conn = get_db(None)?;

    let sql = if show_all {
        "SELECT id, description, status, priority, blocked_by, created_at
         FROM tasks ORDER BY priority, id"
    } else {
        "SELECT id, description, status, priority, blocked_by, created_at
         FROM tasks WHERE status NOT IN ('completed', 'cancelled')
         ORDER BY priority, id"
    };

    let mut stmt = conn.prepare(sql)?;
    let rows: Vec<(i64, String, String, i32, Option<String>, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        println!("{}", dim("No tasks found."));
        return Ok(());
    }

    println!("{}", bold("Tasks"));
    println!("{}", "=".repeat(60));

    for (id, description, status, priority, blocked_by, _created_at) in rows {
        let status_str = match status.as_str() {
            "pending" => dim(&format!("[{}]", status)),
            "in_progress" => yellow(&format!("[{}]", status)),
            "completed" => green(&format!("[{}]", status)),
            "blocked" => red(&format!("[{}]", status)),
            "cancelled" => dim(&format!("[{}]", status)),
            _ => format!("[{}]", status),
        };

        let priority_str = if priority != 5 {
            format!("P{}", priority)
        } else {
            String::new()
        };

        let blocked_str = if let Some(reason) = blocked_by {
            red(&format!(" (blocked: {})", reason))
        } else {
            String::new()
        };

        println!(
            "  #{:3} {:20} {:4} {}{}",
            id, status_str, priority_str, description, blocked_str
        );
    }

    Ok(())
}

pub fn task_next() -> Result<Option<Task>> {
    let conn = get_db(None)?;

    let mut stmt = conn.prepare(
        "SELECT id, description, status, priority, blocked_by, spec_section_id, created_at, started_at, completed_at
         FROM tasks WHERE status = 'pending'
         AND id NOT IN (
             SELECT td.task_id FROM task_dependencies td
             INNER JOIN tasks dep ON dep.id = td.depends_on_id
             WHERE dep.status != 'completed'
         )
         ORDER BY priority, id LIMIT 1",
    )?;

    let task = stmt.query_row([], |row| Task::from_row(row)).ok();

    match &task {
        Some(t) => {
            println!("{}", bold("Next task:"));
            println!("  #{}: {}", t.id, t.description);
            if let Some(spec_id) = t.spec_section_id {
                println!("{}", dim(&format!("  Spec section: {}", spec_id)));
            }
        }
        None => {
            println!("{}", dim("No pending tasks."));
        }
    }

    Ok(task)
}

pub fn task_done(task_id: i64) -> Result<()> {
    let conn = get_db(None)?;

    with_transaction(&conn, |conn| {
        let now = Local::now().to_rfc3339();

        let changed = conn.execute(
            "UPDATE tasks SET status = 'completed', completed_at = ?1 WHERE id = ?2",
            rusqlite::params![now, task_id],
        )?;

        if changed == 0 {
            return Err(DialError::TaskNotFound(task_id));
        }

        // Auto-unblock dependents whose deps are now all satisfied
        auto_unblock_dependents(conn, task_id)?;

        Ok(())
    })
}

/// Check dependents of a completed task and unblock any whose dependencies are all satisfied.
pub fn auto_unblock_dependents(conn: &rusqlite::Connection, completed_task_id: i64) -> Result<()> {
    let mut stmt =
        conn.prepare("SELECT task_id FROM task_dependencies WHERE depends_on_id = ?1")?;
    let dependents: Vec<i64> = stmt
        .query_map([completed_task_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    for dep_id in dependents {
        // Check if this dependent is blocked and all its deps are now completed
        let status: String = conn
            .query_row("SELECT status FROM tasks WHERE id = ?1", [dep_id], |row| {
                row.get(0)
            })
            .map_err(|_| DialError::TaskNotFound(dep_id))?;

        if status == "blocked" {
            let unsatisfied: i64 = conn.query_row(
                "SELECT COUNT(*) FROM task_dependencies td
                 INNER JOIN tasks t ON t.id = td.depends_on_id
                 WHERE td.task_id = ?1 AND t.status != 'completed'",
                [dep_id],
                |row| row.get(0),
            )?;

            if unsatisfied == 0 {
                conn.execute(
                    "UPDATE tasks SET status = 'pending', blocked_by = NULL WHERE id = ?1",
                    [dep_id],
                )?;
            }
        }
    }

    Ok(())
}

pub fn task_block(task_id: i64, reason: &str) -> Result<()> {
    let conn = get_db(None)?;

    let changed = conn.execute(
        "UPDATE tasks SET status = 'blocked', blocked_by = ?1 WHERE id = ?2",
        rusqlite::params![reason, task_id],
    )?;

    if changed == 0 {
        return Err(DialError::TaskNotFound(task_id));
    }

    Ok(())
}

pub fn task_cancel(task_id: i64) -> Result<()> {
    let conn = get_db(None)?;

    let changed = conn.execute(
        "UPDATE tasks SET status = 'cancelled' WHERE id = ?1",
        [task_id],
    )?;

    if changed == 0 {
        return Err(DialError::TaskNotFound(task_id));
    }

    Ok(())
}

pub fn task_search(query: &str) -> Result<()> {
    let conn = get_db(None)?;

    let mut stmt = conn.prepare(
        "SELECT t.id, t.description, t.status, t.priority
         FROM tasks t
         INNER JOIN tasks_fts fts ON t.id = fts.rowid
         WHERE tasks_fts MATCH ?1
         ORDER BY rank",
    )?;

    let rows: Vec<(i64, String, String, i32)> = stmt
        .query_map([query], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        println!("{}", dim(&format!("No tasks matching '{}'.", query)));
        return Ok(());
    }

    println!("{}", bold(&format!("Tasks matching '{}':", query)));
    for (id, description, status, _priority) in rows {
        println!("  #{} [{}] {}", id, status, description);
    }

    Ok(())
}

pub fn get_task_by_id(task_id: i64) -> Result<Task> {
    let conn = get_db(None)?;

    let mut stmt = conn.prepare(
        "SELECT id, description, status, priority, blocked_by, spec_section_id, created_at, started_at, completed_at
         FROM tasks WHERE id = ?1",
    )?;

    stmt.query_row([task_id], |row| Task::from_row(row))
        .map_err(|_| DialError::TaskNotFound(task_id))
}

// --- Dependency Management ---

/// Add a dependency: task_id depends on depends_on_id.
/// Checks for self-dependency and cycles before inserting.
pub fn task_depends(task_id: i64, depends_on_id: i64) -> Result<()> {
    if task_id == depends_on_id {
        return Err(DialError::SelfDependency(task_id));
    }

    let conn = get_db(None)?;

    // Verify both tasks exist
    let _t1 = conn
        .query_row("SELECT id FROM tasks WHERE id = ?1", [task_id], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|_| DialError::TaskNotFound(task_id))?;
    let _t2 = conn
        .query_row(
            "SELECT id FROM tasks WHERE id = ?1",
            [depends_on_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| DialError::TaskNotFound(depends_on_id))?;

    // Check for cycles: would adding this edge create a path from depends_on_id back to task_id?
    if would_create_cycle(&conn, task_id, depends_on_id)? {
        return Err(DialError::CyclicDependency(task_id));
    }

    conn.execute(
        "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id) VALUES (?1, ?2)",
        rusqlite::params![task_id, depends_on_id],
    )?;

    Ok(())
}

/// Remove a dependency.
pub fn task_undepend(task_id: i64, depends_on_id: i64) -> Result<()> {
    let conn = get_db(None)?;
    conn.execute(
        "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on_id = ?2",
        rusqlite::params![task_id, depends_on_id],
    )?;
    Ok(())
}

/// Get all tasks that task_id depends on (its prerequisites).
pub fn task_get_dependencies(task_id: i64) -> Result<Vec<i64>> {
    let conn = get_db(None)?;
    let mut stmt =
        conn.prepare("SELECT depends_on_id FROM task_dependencies WHERE task_id = ?1")?;
    let deps: Vec<i64> = stmt
        .query_map([task_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(deps)
}

/// Get all tasks that depend on task_id (its dependents).
pub fn task_get_dependents(task_id: i64) -> Result<Vec<i64>> {
    let conn = get_db(None)?;
    let mut stmt =
        conn.prepare("SELECT task_id FROM task_dependencies WHERE depends_on_id = ?1")?;
    let deps: Vec<i64> = stmt
        .query_map([task_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(deps)
}

/// Show dependency info for a task.
pub fn task_show_deps(task_id: i64) -> Result<()> {
    let deps = task_get_dependencies(task_id)?;
    let dependents = task_get_dependents(task_id)?;

    println!("{}", bold(&format!("Dependencies for task #{}", task_id)));

    if deps.is_empty() {
        println!("  {}", dim("No prerequisites (can run immediately)"));
    } else {
        println!("  Depends on:");
        for dep_id in &deps {
            let task = get_task_by_id(*dep_id)?;
            let status_str = match task.status.to_string().as_str() {
                "completed" => green("[completed]"),
                "pending" => dim("[pending]"),
                _ => yellow(&format!("[{}]", task.status)),
            };
            println!("    #{} {} {}", dep_id, status_str, task.description);
        }
    }

    if !dependents.is_empty() {
        println!("  Depended on by:");
        for dep_id in &dependents {
            let task = get_task_by_id(*dep_id)?;
            println!("    #{} {}", dep_id, task.description);
        }
    }

    Ok(())
}

/// Check if all dependencies of a task are completed.
pub fn task_deps_satisfied(task_id: i64) -> Result<bool> {
    let conn = get_db(None)?;
    let unsatisfied: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_dependencies td
         INNER JOIN tasks t ON t.id = td.depends_on_id
         WHERE td.task_id = ?1 AND t.status != 'completed'",
        [task_id],
        |row| row.get(0),
    )?;
    Ok(unsatisfied == 0)
}

// --- Similar Completed Task Context ---

/// Strip common stop words from a description to produce a cleaner FTS query.
fn strip_stop_words(description: &str) -> String {
    const STOP_WORDS: &[&str] = &["the", "a", "an", "is", "for", "to", "of", "in", "and", "or"];
    description
        .split_whitespace()
        .filter(|word| !STOP_WORDS.contains(&word.to_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Find completed tasks similar to the given description using FTS.
/// Returns `Vec<(Task, String)>` where the String contains the most recent
/// successful iteration's notes and commit hash, ordered by FTS rank.
pub fn find_similar_completed_tasks(
    conn: &Connection,
    description: &str,
    limit: usize,
) -> Result<Vec<(Task, String)>> {
    let query = strip_stop_words(description);
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT t.id, t.description, t.status, t.priority, t.blocked_by,
                t.spec_section_id, t.created_at, t.started_at, t.completed_at
         FROM tasks t
         INNER JOIN tasks_fts fts ON t.id = fts.rowid
         WHERE tasks_fts MATCH ?1 AND t.status = 'completed'
         ORDER BY rank
         LIMIT ?2",
    )?;

    let tasks: Vec<Task> = stmt
        .query_map(rusqlite::params![query, limit as i64], |row| {
            Task::from_row(row)
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut results = Vec::new();
    for task in tasks {
        let context = conn
            .query_row(
                "SELECT COALESCE(notes, ''), COALESCE(commit_hash, '')
                 FROM iterations
                 WHERE task_id = ?1 AND status = 'completed'
                 ORDER BY ended_at DESC LIMIT 1",
                [task.id],
                |row| {
                    let notes: String = row.get(0)?;
                    let commit_hash: String = row.get(1)?;
                    Ok(format!("Approach: {}\nCommit: {}", notes, commit_hash))
                },
            )
            .unwrap_or_else(|_| "No iteration data available".to_string());

        results.push((task, context));
    }

    Ok(results)
}

// --- Cross-Iteration Failure Tracking ---

/// Increment total_attempts for a task when an iteration starts.
pub fn increment_total_attempts(conn: &Connection, task_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE tasks SET total_attempts = COALESCE(total_attempts, 0) + 1 WHERE id = ?1",
        [task_id],
    )?;
    Ok(())
}

/// Increment total_failures and set last_failure_at when an iteration fails.
pub fn increment_total_failures(conn: &Connection, task_id: i64) -> Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "UPDATE tasks SET total_failures = COALESCE(total_failures, 0) + 1, last_failure_at = ?1 WHERE id = ?2",
        rusqlite::params![now, task_id],
    )?;
    Ok(())
}

/// A task that has exceeded a chronic failure threshold.
#[derive(Debug, Clone)]
pub struct ChronicFailureInfo {
    pub task_id: i64,
    pub description: String,
    pub total_failures: i64,
    pub total_attempts: i64,
    pub last_failure_at: Option<String>,
}

/// Return tasks where total_failures >= threshold.
pub fn get_chronic_failures(threshold: i64) -> Result<Vec<ChronicFailureInfo>> {
    let conn = get_db(None)?;
    get_chronic_failures_with_conn(&conn, threshold)
}

/// Return tasks where total_failures >= threshold (using an existing connection).
pub fn get_chronic_failures_with_conn(
    conn: &Connection,
    threshold: i64,
) -> Result<Vec<ChronicFailureInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, description, COALESCE(total_failures, 0), COALESCE(total_attempts, 0), last_failure_at
         FROM tasks
         WHERE COALESCE(total_failures, 0) >= ?1
         ORDER BY total_failures DESC, id",
    )?;

    let rows = stmt
        .query_map([threshold], |row| {
            Ok(ChronicFailureInfo {
                task_id: row.get(0)?,
                description: row.get(1)?,
                total_failures: row.get(2)?,
                total_attempts: row.get(3)?,
                last_failure_at: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Check if adding an edge (task_id -> depends_on_id) would create a cycle.
/// A cycle exists if there's already a path from depends_on_id to task_id.
fn would_create_cycle(
    conn: &rusqlite::Connection,
    task_id: i64,
    depends_on_id: i64,
) -> Result<bool> {
    // BFS from depends_on_id following dependency edges to see if we reach task_id
    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(depends_on_id);

    while let Some(current) = queue.pop_front() {
        if current == task_id {
            return Ok(true); // Cycle detected
        }
        if !visited.insert(current) {
            continue;
        }
        // Get what `current` depends on
        let mut stmt =
            conn.prepare("SELECT depends_on_id FROM task_dependencies WHERE task_id = ?1")?;
        let deps: Vec<i64> = stmt
            .query_map([current], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        for dep in deps {
            if !visited.contains(&dep) {
                queue.push_back(dep);
            }
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;
    use crate::db::schema;

    fn setup_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .unwrap();
        conn.execute_batch(schema::SCHEMA).unwrap();
        migrations::run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn test_increment_total_attempts() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO tasks (description, priority) VALUES ('test task', 5)",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Initial value should be 0
        let val: i64 = conn
            .query_row(
                "SELECT COALESCE(total_attempts, 0) FROM tasks WHERE id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, 0);

        // Increment once
        increment_total_attempts(&conn, task_id).unwrap();
        let val: i64 = conn
            .query_row(
                "SELECT total_attempts FROM tasks WHERE id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, 1);

        // Increment again
        increment_total_attempts(&conn, task_id).unwrap();
        let val: i64 = conn
            .query_row(
                "SELECT total_attempts FROM tasks WHERE id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, 2);
    }

    #[test]
    fn test_increment_total_failures() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO tasks (description, priority) VALUES ('failing task', 5)",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Initial value should be 0
        let val: i64 = conn
            .query_row(
                "SELECT COALESCE(total_failures, 0) FROM tasks WHERE id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, 0);

        // Increment once
        increment_total_failures(&conn, task_id).unwrap();
        let val: i64 = conn
            .query_row(
                "SELECT total_failures FROM tasks WHERE id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, 1);

        // last_failure_at should be set
        let last: Option<String> = conn
            .query_row(
                "SELECT last_failure_at FROM tasks WHERE id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(last.is_some());

        // Increment multiple times
        increment_total_failures(&conn, task_id).unwrap();
        increment_total_failures(&conn, task_id).unwrap();
        let val: i64 = conn
            .query_row(
                "SELECT total_failures FROM tasks WHERE id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, 3);
    }

    #[test]
    fn test_get_chronic_failures_empty() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO tasks (description, priority) VALUES ('healthy task', 5)",
            [],
        )
        .unwrap();

        let results = get_chronic_failures_with_conn(&conn, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_chronic_failures_threshold() {
        let conn = setup_test_db();

        // Create two tasks
        conn.execute(
            "INSERT INTO tasks (description, priority) VALUES ('chronic task', 5)",
            [],
        )
        .unwrap();
        let chronic_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO tasks (description, priority) VALUES ('ok task', 5)",
            [],
        )
        .unwrap();
        let ok_id = conn.last_insert_rowid();

        // Give chronic task 5 failures, ok task 2
        for _ in 0..5 {
            increment_total_failures(&conn, chronic_id).unwrap();
            increment_total_attempts(&conn, chronic_id).unwrap();
        }
        for _ in 0..2 {
            increment_total_failures(&conn, ok_id).unwrap();
            increment_total_attempts(&conn, ok_id).unwrap();
        }

        // Threshold 5: only chronic task
        let results = get_chronic_failures_with_conn(&conn, 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].task_id, chronic_id);
        assert_eq!(results[0].total_failures, 5);
        assert_eq!(results[0].total_attempts, 5);
        assert!(results[0].last_failure_at.is_some());

        // Threshold 2: both tasks
        let results = get_chronic_failures_with_conn(&conn, 2).unwrap();
        assert_eq!(results.len(), 2);

        // Threshold 10: neither
        let results = get_chronic_failures_with_conn(&conn, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_strip_stop_words() {
        assert_eq!(
            strip_stop_words("Add the logging for an API endpoint"),
            "Add logging API endpoint"
        );
        assert_eq!(strip_stop_words("the a an is for to of in and or"), "");
        assert_eq!(strip_stop_words("implement feature"), "implement feature");
        assert_eq!(strip_stop_words(""), "");
    }

    #[test]
    fn test_strip_stop_words_case_insensitive() {
        assert_eq!(strip_stop_words("The AND Or IS"), "");
    }

    #[test]
    fn test_find_similar_completed_tasks_empty() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO tasks (description, priority, status) VALUES ('pending task', 5, 'pending')",
            [],
        ).unwrap();

        let results = find_similar_completed_tasks(&conn, "pending task", 3).unwrap();
        assert!(results.is_empty(), "Should not match non-completed tasks");
    }

    #[test]
    fn test_find_similar_completed_tasks_matches() {
        let conn = setup_test_db();

        // Insert a completed task
        conn.execute(
            "INSERT INTO tasks (id, description, priority, status, completed_at) VALUES (1, 'implement database migration system', 5, 'completed', '2025-01-01T00:00:00Z')",
            [],
        ).unwrap();

        // Insert a successful iteration with notes and commit hash
        conn.execute(
            "INSERT INTO iterations (task_id, status, notes, commit_hash, ended_at) VALUES (1, 'completed', 'Used ALTER TABLE for schema changes', 'abc123', '2025-01-01T00:00:00Z')",
            [],
        ).unwrap();

        let results = find_similar_completed_tasks(&conn, "database migration", 3).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.id, 1);
        assert!(results[0].1.contains("Used ALTER TABLE for schema changes"));
        assert!(results[0].1.contains("abc123"));
    }

    #[test]
    fn test_find_similar_completed_tasks_limit() {
        let conn = setup_test_db();

        for i in 1..=5 {
            conn.execute(
                "INSERT INTO tasks (id, description, priority, status, completed_at) VALUES (?1, ?2, 5, 'completed', '2025-01-01T00:00:00Z')",
                rusqlite::params![i, format!("implement feature variant {}", i)],
            ).unwrap();
            conn.execute(
                "INSERT INTO iterations (task_id, status, notes, commit_hash, ended_at) VALUES (?1, 'completed', ?2, ?3, '2025-01-01T00:00:00Z')",
                rusqlite::params![i, format!("notes for {}", i), format!("hash{}", i)],
            ).unwrap();
        }

        let results = find_similar_completed_tasks(&conn, "implement feature", 2).unwrap();
        assert_eq!(results.len(), 2, "Should respect the limit parameter");
    }

    #[test]
    fn test_find_similar_completed_tasks_only_stop_words() {
        let conn = setup_test_db();
        let results = find_similar_completed_tasks(&conn, "the a an is", 3).unwrap();
        assert!(
            results.is_empty(),
            "All-stop-words query should return empty"
        );
    }

    #[test]
    fn test_find_similar_completed_tasks_no_iteration_data() {
        let conn = setup_test_db();

        // Completed task but no iterations
        conn.execute(
            "INSERT INTO tasks (id, description, priority, status, completed_at) VALUES (1, 'setup logging infrastructure', 5, 'completed', '2025-01-01T00:00:00Z')",
            [],
        ).unwrap();

        let results = find_similar_completed_tasks(&conn, "logging infrastructure", 3).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.contains("No iteration data available"));
    }

    #[test]
    fn test_find_similar_completed_tasks_uses_latest_iteration() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO tasks (id, description, priority, status, completed_at) VALUES (1, 'build auth system', 5, 'completed', '2025-01-01T00:00:00Z')",
            [],
        ).unwrap();

        // Older iteration
        conn.execute(
            "INSERT INTO iterations (task_id, status, notes, commit_hash, ended_at) VALUES (1, 'completed', 'old approach', 'old111', '2025-01-01T00:00:00Z')",
            [],
        ).unwrap();

        // Newer iteration (should be returned)
        conn.execute(
            "INSERT INTO iterations (task_id, status, notes, commit_hash, ended_at) VALUES (1, 'completed', 'final approach with JWT', 'new222', '2025-01-02T00:00:00Z')",
            [],
        ).unwrap();

        let results = find_similar_completed_tasks(&conn, "auth system", 3).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.contains("final approach with JWT"));
        assert!(results[0].1.contains("new222"));
    }

    #[test]
    fn test_chronic_failures_ordered_by_failure_count() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO tasks (description, priority) VALUES ('task A', 5)",
            [],
        )
        .unwrap();
        let a_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO tasks (description, priority) VALUES ('task B', 5)",
            [],
        )
        .unwrap();
        let b_id = conn.last_insert_rowid();

        // B has more failures than A
        for _ in 0..3 {
            increment_total_failures(&conn, a_id).unwrap();
        }
        for _ in 0..7 {
            increment_total_failures(&conn, b_id).unwrap();
        }

        let results = get_chronic_failures_with_conn(&conn, 3).unwrap();
        assert_eq!(results.len(), 2);
        // B should come first (more failures)
        assert_eq!(results[0].task_id, b_id);
        assert_eq!(results[0].total_failures, 7);
        assert_eq!(results[1].task_id, a_id);
        assert_eq!(results[1].total_failures, 3);
    }
}
