pub mod context;
pub mod validation;

use crate::db::{get_db, get_dial_dir};
use crate::errors::{DialError, Result};
use crate::failure::{find_trusted_solutions, record_failure};
use crate::git::{git_commit, git_has_changes, git_is_repo, git_revert_to};
use crate::output::{bold, dim, green, print_success, red, yellow};
use crate::task::models::Task;
use crate::MAX_FIX_ATTEMPTS;
use chrono::Local;
use rusqlite::Connection;
use std::fs;

pub use context::gather_context;
pub use validation::run_validation;

pub fn create_iteration(conn: &Connection, task_id: i64, attempt_number: i32) -> Result<i64> {
    let now = Local::now().to_rfc3339();

    conn.execute(
        "INSERT INTO iterations (task_id, attempt_number, started_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![task_id, attempt_number, now],
    )?;

    let iteration_id = conn.last_insert_rowid();

    // Update task status
    conn.execute(
        "UPDATE tasks SET status = 'in_progress', started_at = ?1 WHERE id = ?2",
        rusqlite::params![now, task_id],
    )?;

    Ok(iteration_id)
}

pub fn complete_iteration(
    conn: &Connection,
    iteration_id: i64,
    status: &str,
    commit_hash: Option<&str>,
    notes: Option<&str>,
) -> Result<()> {
    let started_at: String = conn.query_row(
        "SELECT started_at FROM iterations WHERE id = ?1",
        [iteration_id],
        |row| row.get(0),
    )?;

    let started = chrono::DateTime::parse_from_rfc3339(&started_at)
        .map(|dt| dt.with_timezone(&chrono::Local))
        .unwrap_or_else(|_| Local::now());

    let duration = (Local::now() - started).num_milliseconds() as f64 / 1000.0;
    let now = Local::now().to_rfc3339();

    conn.execute(
        "UPDATE iterations
         SET status = ?1, ended_at = ?2, duration_seconds = ?3, commit_hash = ?4, notes = ?5
         WHERE id = ?6",
        rusqlite::params![status, now, duration, commit_hash, notes, iteration_id],
    )?;

    Ok(())
}

pub fn iterate_once() -> Result<(bool, String)> {
    let conn = get_db(None)?;

    // Get next task
    let mut stmt = conn.prepare(
        "SELECT id, description, status, priority, blocked_by, spec_section_id, created_at, started_at, completed_at
         FROM tasks WHERE status = 'pending'
         ORDER BY priority, id LIMIT 1",
    )?;

    let task = stmt.query_row([], |row| Task::from_row(row)).ok();

    let task = match task {
        Some(t) => t,
        None => {
            println!("{}", dim("No pending tasks. Task queue empty."));
            return Ok((false, "empty_queue".to_string()));
        }
    };

    println!("{}", bold(&"=".repeat(60)));
    println!("{}", bold(&format!("Iteration: Task #{}", task.id)));
    println!("Description: {}", task.description);
    println!("{}", bold(&"=".repeat(60)));
    println!();

    // Check for existing failed iterations
    let max_attempt: Option<i32> = conn
        .query_row(
            "SELECT MAX(attempt_number) FROM iterations WHERE task_id = ?1 AND status = 'failed'",
            [task.id],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    let attempt_number = max_attempt.unwrap_or(0) + 1;

    if attempt_number > MAX_FIX_ATTEMPTS as i32 {
        println!(
            "{}",
            red(&format!(
                "Task #{} has failed {} times. Skipping.",
                task.id, MAX_FIX_ATTEMPTS
            ))
        );

        conn.execute(
            "UPDATE tasks SET status = 'blocked', blocked_by = ?1 WHERE id = ?2",
            rusqlite::params![format!("Failed {} times", MAX_FIX_ATTEMPTS), task.id],
        )?;

        return Ok((true, "max_attempts".to_string()));
    }

    // Create iteration
    let _iteration_id = create_iteration(&conn, task.id, attempt_number)?;
    println!("Attempt {} of {}", attempt_number, MAX_FIX_ATTEMPTS);

    // Gather context
    let context = gather_context(&conn, &task)?;
    if !context.is_empty() {
        println!("{}", dim("\nContext gathered. Relevant specs and solutions loaded."));
    }

    // Store context for the agent
    let context_file = get_dial_dir().join("current_context.md");
    let context_content = format!("# Task: {}\n\n{}", task.description, context);
    fs::write(&context_file, context_content)?;
    println!("Context written to: {}", context_file.display());

    println!("{}", yellow("\n>>> Agent should now implement the task <<<"));
    println!("{}", yellow(">>> Run 'dial validate' when ready to validate <<<"));
    println!("{}", yellow(">>> Or 'dial complete' to mark complete without validation <<<\n"));

    Ok((true, "awaiting_work".to_string()))
}

pub fn validate_current() -> Result<bool> {
    let conn = get_db(None)?;

    // Find current in-progress iteration
    let iteration: Option<(i64, i64, String)> = conn
        .query_row(
            "SELECT i.id, i.task_id, t.description
             FROM iterations i
             INNER JOIN tasks t ON i.task_id = t.id
             WHERE i.status = 'in_progress'
             ORDER BY i.id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();

    let (iteration_id, task_id, task_description) = match iteration {
        Some(i) => i,
        None => {
            return Err(DialError::NoIterationInProgress);
        }
    };

    println!("Validating iteration #{} for task #{}", iteration_id, task_id);

    // Run validation
    let (success, error_output) = run_validation(&conn, iteration_id)?;

    if success {
        // Commit changes
        let commit_hash = if git_is_repo() && git_has_changes() {
            let message = format!("DIAL: {}", task_description);
            if let Some(hash) = git_commit(&message)? {
                println!("{}", green(&format!("Committed: {}", &hash[..8])));
                Some(hash)
            } else {
                None
            }
        } else {
            None
        };

        // Complete iteration
        complete_iteration(&conn, iteration_id, "completed", commit_hash.as_deref(), None)?;

        // Complete task
        let now = Local::now().to_rfc3339();
        conn.execute(
            "UPDATE tasks SET status = 'completed', completed_at = ?1 WHERE id = ?2",
            rusqlite::params![now, task_id],
        )?;

        println!("{}", green(&format!("\nIteration #{} completed successfully!", iteration_id)));
        println!("{}", green(&format!("Task #{} marked as completed.", task_id)));

        Ok(true)
    } else {
        // Record failure
        let (failure_id, pattern_id) = record_failure(&conn, iteration_id, &error_output, None, None)?;
        println!("{}", red(&format!("Recorded failure #{}", failure_id)));

        // Check for trusted solutions
        let solutions = find_trusted_solutions(&conn, pattern_id)?;
        if !solutions.is_empty() {
            println!("{}", yellow("\nTrusted solutions available:"));
            for sol in solutions {
                println!("  - {}", sol.description);
            }
        }

        // Complete iteration as failed
        let notes = if error_output.len() > 500 {
            &error_output[..500]
        } else {
            &error_output
        };
        complete_iteration(&conn, iteration_id, "failed", None, Some(notes))?;

        // Check if we should revert
        let fail_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM iterations WHERE task_id = ?1 AND status = 'failed'",
            [task_id],
            |row| row.get(0),
        )?;

        if fail_count >= MAX_FIX_ATTEMPTS as i64 {
            println!("{}", red(&format!("\nMax attempts ({}) reached.", MAX_FIX_ATTEMPTS)));

            // Find last successful commit
            let last_good_commit: Option<String> = conn
                .query_row(
                    "SELECT commit_hash FROM iterations
                     WHERE status = 'completed' AND commit_hash IS NOT NULL
                     ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .ok();

            if let Some(hash) = last_good_commit {
                if git_is_repo() {
                    println!("{}", yellow(&format!("Reverting to last good commit: {}", &hash[..8])));
                    git_revert_to(&hash)?;
                }
            }

            // Block the task
            conn.execute(
                "UPDATE tasks SET status = 'blocked', blocked_by = ?1 WHERE id = ?2",
                rusqlite::params![format!("Failed {} attempts", MAX_FIX_ATTEMPTS), task_id],
            )?;
        } else {
            // Reset task to pending for retry
            conn.execute(
                "UPDATE tasks SET status = 'pending' WHERE id = ?1",
                [task_id],
            )?;
            let remaining = MAX_FIX_ATTEMPTS as i64 - fail_count;
            println!("{}", yellow(&format!("\nTask reset to pending. {} attempts remaining.", remaining)));
        }

        Ok(false)
    }
}

pub fn run_loop(max_iterations: Option<u32>) -> Result<()> {
    let dial_dir = get_dial_dir();
    let stop_file = dial_dir.join("stop");

    // Remove any existing stop file
    if stop_file.exists() {
        fs::remove_file(&stop_file)?;
    }

    println!("{}", bold("Starting DIAL run loop..."));
    println!("{}", dim("Create .dial/stop file to stop gracefully.\n"));

    let mut iteration_count = 0u32;

    loop {
        // Check stop flag
        if stop_file.exists() {
            println!("{}", yellow("\nStop flag detected. Stopping gracefully."));
            fs::remove_file(&stop_file)?;
            break;
        }

        // Check iteration limit
        if let Some(max) = max_iterations {
            if iteration_count >= max {
                println!("{}", yellow(&format!("\nReached max iterations ({}). Stopping.", max)));
                break;
            }
        }

        // Run one iteration
        let (_success, result) = iterate_once()?;

        if result == "empty_queue" {
            println!("\n{}", bold(&"=".repeat(60)));
            println!("{}", bold("Task queue empty. DIAL run complete."));
            show_run_summary()?;
            break;
        }

        if result == "awaiting_work" {
            println!("{}", dim("\nWaiting for work. Run 'dial validate' after implementing."));
            break;
        }

        iteration_count += 1;
    }

    Ok(())
}

fn show_run_summary() -> Result<()> {
    let conn = get_db(None)?;

    let (_total, completed, failed): (i64, i64, i64) = conn.query_row(
        "SELECT
            COUNT(*),
            SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END),
            SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END)
         FROM iterations",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    println!("\nCompleted: {}", completed);
    println!("Failed: {}", failed);

    let solutions_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM solutions WHERE confidence >= ?1",
        [crate::TRUST_THRESHOLD],
        |row| row.get(0),
    )?;

    println!("Solutions learned: {}", solutions_count);

    Ok(())
}

pub fn revert_to_last_good() -> Result<bool> {
    if !git_is_repo() {
        return Err(DialError::NotGitRepo);
    }

    let conn = get_db(None)?;

    let commit_hash: Option<String> = conn
        .query_row(
            "SELECT commit_hash FROM iterations
             WHERE status = 'completed' AND commit_hash IS NOT NULL
             ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();

    match commit_hash {
        Some(hash) => {
            println!("{}", yellow(&format!("Reverting to: {}", hash)));
            if git_revert_to(&hash)? {
                print_success("Reverted successfully.");
                Ok(true)
            } else {
                println!("{}", red("Revert failed."));
                Ok(false)
            }
        }
        None => {
            println!("{}", red("No successful commits found."));
            Ok(false)
        }
    }
}

pub fn reset_current() -> Result<()> {
    let conn = get_db(None)?;

    let iteration: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, task_id FROM iterations WHERE status = 'in_progress' ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    match iteration {
        Some((iteration_id, task_id)) => {
            let now = Local::now().to_rfc3339();

            // Mark iteration as reverted
            conn.execute(
                "UPDATE iterations SET status = 'reverted', ended_at = ?1 WHERE id = ?2",
                rusqlite::params![now, iteration_id],
            )?;

            // Reset task to pending
            conn.execute(
                "UPDATE tasks SET status = 'pending' WHERE id = ?1",
                [task_id],
            )?;

            print_success(&format!(
                "Reset iteration #{}. Task returned to pending.",
                iteration_id
            ));
        }
        None => {
            println!("{}", dim("No iteration in progress."));
        }
    }

    Ok(())
}

pub fn stop_loop() -> Result<()> {
    let stop_file = get_dial_dir().join("stop");
    fs::write(&stop_file, "")?;
    println!("{}", yellow("Stop flag created. DIAL will stop after current iteration."));
    Ok(())
}
