pub mod models;

use crate::db::get_db;
use crate::errors::{DialError, Result};
use crate::output::{bold, dim, green, print_success, red, yellow};
use chrono::Local;
use models::Task;

pub fn task_add(description: &str, priority: i32, spec_section_id: Option<i64>) -> Result<i64> {
    let conn = get_db(None)?;
    conn.execute(
        "INSERT INTO tasks (description, priority, spec_section_id) VALUES (?1, ?2, ?3)",
        rusqlite::params![description, priority, spec_section_id],
    )?;
    let task_id = conn.last_insert_rowid();
    print_success(&format!("Added task #{}: {}", task_id, description));
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

    let task = stmt
        .query_row([], |row| Task::from_row(row))
        .ok();

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
    let now = Local::now().to_rfc3339();

    let changed = conn.execute(
        "UPDATE tasks SET status = 'completed', completed_at = ?1 WHERE id = ?2",
        rusqlite::params![now, task_id],
    )?;

    if changed == 0 {
        return Err(DialError::TaskNotFound(task_id));
    }

    print_success(&format!("Task #{} marked as completed.", task_id));

    // Auto-unblock dependents whose deps are now all satisfied
    auto_unblock_dependents(&conn, task_id)?;

    Ok(())
}

/// Check dependents of a completed task and unblock any whose dependencies are all satisfied.
pub fn auto_unblock_dependents(conn: &rusqlite::Connection, completed_task_id: i64) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT task_id FROM task_dependencies WHERE depends_on_id = ?1"
    )?;
    let dependents: Vec<i64> = stmt
        .query_map([completed_task_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    for dep_id in dependents {
        // Check if this dependent is blocked and all its deps are now completed
        let status: String = conn.query_row(
            "SELECT status FROM tasks WHERE id = ?1",
            [dep_id],
            |row| row.get(0),
        ).map_err(|_| DialError::TaskNotFound(dep_id))?;

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
                print_success(&format!("Task #{} auto-unblocked (all dependencies satisfied).", dep_id));
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

    println!("{}", yellow(&format!("Task #{} marked as blocked: {}", task_id, reason)));
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

    println!("{}", dim(&format!("Task #{} cancelled.", task_id)));
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
        .query_map([query], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
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
        .query_row("SELECT id FROM tasks WHERE id = ?1", [task_id], |row| row.get::<_, i64>(0))
        .map_err(|_| DialError::TaskNotFound(task_id))?;
    let _t2 = conn
        .query_row("SELECT id FROM tasks WHERE id = ?1", [depends_on_id], |row| row.get::<_, i64>(0))
        .map_err(|_| DialError::TaskNotFound(depends_on_id))?;

    // Check for cycles: would adding this edge create a path from depends_on_id back to task_id?
    if would_create_cycle(&conn, task_id, depends_on_id)? {
        return Err(DialError::CyclicDependency(task_id));
    }

    conn.execute(
        "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id) VALUES (?1, ?2)",
        rusqlite::params![task_id, depends_on_id],
    )?;

    print_success(&format!("Task #{} now depends on task #{}", task_id, depends_on_id));
    Ok(())
}

/// Remove a dependency.
pub fn task_undepend(task_id: i64, depends_on_id: i64) -> Result<()> {
    let conn = get_db(None)?;
    conn.execute(
        "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on_id = ?2",
        rusqlite::params![task_id, depends_on_id],
    )?;
    print_success(&format!("Removed dependency: task #{} no longer depends on #{}", task_id, depends_on_id));
    Ok(())
}

/// Get all tasks that task_id depends on (its prerequisites).
pub fn task_get_dependencies(task_id: i64) -> Result<Vec<i64>> {
    let conn = get_db(None)?;
    let mut stmt = conn.prepare(
        "SELECT depends_on_id FROM task_dependencies WHERE task_id = ?1"
    )?;
    let deps: Vec<i64> = stmt
        .query_map([task_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(deps)
}

/// Get all tasks that depend on task_id (its dependents).
pub fn task_get_dependents(task_id: i64) -> Result<Vec<i64>> {
    let conn = get_db(None)?;
    let mut stmt = conn.prepare(
        "SELECT task_id FROM task_dependencies WHERE depends_on_id = ?1"
    )?;
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

/// Check if adding an edge (task_id -> depends_on_id) would create a cycle.
/// A cycle exists if there's already a path from depends_on_id to task_id.
fn would_create_cycle(conn: &rusqlite::Connection, task_id: i64, depends_on_id: i64) -> Result<bool> {
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
        let mut stmt = conn.prepare(
            "SELECT depends_on_id FROM task_dependencies WHERE task_id = ?1"
        )?;
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
