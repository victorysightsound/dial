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
