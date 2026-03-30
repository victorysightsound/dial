pub mod context;
pub mod orchestrator;
pub mod signal;
pub mod validation;

use crate::artifacts::{
    append_progress_log_entry, sync_operator_artifacts, sync_patterns_digest, ProgressLogEntry,
    ProgressOutcome,
};
use crate::db::{get_db, get_dial_dir, with_transaction};
use crate::errors::{DialError, Result};
use crate::failure::record_failure;
use crate::git::{
    checkpoint_create, checkpoint_drop, checkpoint_restore, checkpoints_enabled, git_commit,
    git_diff, git_diff_stat, git_has_changes, git_is_repo, git_revert_to,
};
use crate::output::{bold, dim, green, print_success, red, yellow};
use crate::task::models::Task;
use crate::MAX_FIX_ATTEMPTS;
use chrono::Local;
use rusqlite::Connection;
use std::fs;

pub use context::{
    gather_context, gather_context_budgeted, gather_context_items, gather_context_items_pure,
    generate_subagent_prompt,
};
pub use orchestrator::auto_run;
pub use signal::{read_signal_file, write_signal_file, SignalFile, SubagentSignal};
pub use validation::{
    run_validation, run_validation_with_details, PipelineStepResult, ValidationResult,
};

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

    // Increment cross-iteration attempt counter
    crate::task::increment_total_attempts(conn, task_id)?;

    Ok(iteration_id)
}

pub fn complete_iteration(
    conn: &Connection,
    iteration_id: i64,
    status: &str,
    commit_hash: Option<&str>,
    notes: Option<&str>,
) -> Result<()> {
    let (started_at, task_id): (String, i64) = conn.query_row(
        "SELECT started_at, task_id FROM iterations WHERE id = ?1",
        [iteration_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
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

    // Increment cross-iteration failure counter when iteration fails
    if status == "failed" {
        crate::task::increment_total_failures(conn, task_id)?;
    }

    Ok(())
}

pub fn iterate_once() -> Result<(bool, String)> {
    let conn = get_db(None)?;

    // Get next task (dependency-aware: skip tasks with unsatisfied deps)
    let mut stmt = conn.prepare(
        "SELECT id, description, status, priority, blocked_by, spec_section_id, created_at, started_at, completed_at,
                prd_section_id, total_attempts, total_failures, last_failure_at,
                acceptance_criteria_json, requires_browser_verification
         FROM tasks WHERE status = 'pending'
         AND id NOT IN (
             SELECT td.task_id FROM task_dependencies td
             INNER JOIN tasks dep ON dep.id = td.depends_on_id
             WHERE dep.status != 'completed'
         )
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
    if task.requires_browser_verification {
        println!(
            "{}",
            yellow(
                "This task requires browser verification before completion. After checking the UI manually, record it with `dial task verify-browser <task-id> --page <screen-or-route>`."
            )
        );
        println!();
    }

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

        // Wrap the block operation in a transaction
        with_transaction(&conn, |conn| {
            conn.execute(
                "UPDATE tasks SET status = 'blocked', blocked_by = ?1 WHERE id = ?2",
                rusqlite::params![format!("Failed {} times", MAX_FIX_ATTEMPTS), task.id],
            )?;
            Ok(())
        })?;

        return Ok((true, "max_attempts".to_string()));
    }

    // Create iteration + update task status atomically
    let iteration_id = with_transaction(&conn, |conn| {
        create_iteration(conn, task.id, attempt_number)
    })?;
    println!("Attempt {} of {}", attempt_number, MAX_FIX_ATTEMPTS);
    let _ = sync_operator_artifacts(&conn);

    // Create checkpoint before task execution (if enabled and in a git repo)
    if git_is_repo() && checkpoints_enabled() {
        let checkpoint_id = format!("{}", iteration_id);
        match checkpoint_create(&checkpoint_id) {
            Ok(true) => {
                println!(
                    "{}",
                    dim(&format!("Checkpoint created (iteration #{})", iteration_id))
                );
            }
            Ok(false) => {
                // Working tree clean — no checkpoint needed
            }
            Err(e) => {
                println!(
                    "{}",
                    yellow(&format!("Warning: checkpoint creation failed: {}", e))
                );
            }
        }
    }

    // Gather context
    let context = gather_context(&conn, &task)?;
    if !context.is_empty() {
        println!(
            "{}",
            dim("\nContext gathered. Relevant specs and solutions loaded.")
        );
    }

    // Store context for the agent
    let context_file = get_dial_dir().join("current_context.md");
    let context_content = format!("# Task: {}\n\n{}", task.description, context);
    fs::write(&context_file, context_content)?;
    println!("Context written to: {}", context_file.display());
    let _ = sync_patterns_digest(&conn);

    println!(
        "{}",
        yellow("\n>>> Agent should now implement the task <<<")
    );
    println!(
        "{}",
        yellow(">>> Run 'dial validate' when ready to validate <<<")
    );
    println!(
        "{}",
        yellow(">>> Or 'dial complete' to mark complete without validation <<<\n")
    );

    Ok((true, "awaiting_work".to_string()))
}

/// Result of validate_current including per-step details.
pub struct ValidateResult {
    pub success: bool,
    pub step_results: Vec<PipelineStepResult>,
    /// Task ID for the validated iteration (used for post-validation solution tracking).
    pub task_id: Option<i64>,
    /// Solutions suggested for failures during this validation (failure_id, solution_id, description, confidence).
    pub suggested_solutions: Vec<(i64, i64, String, f64)>,
}

pub fn validate_current() -> Result<bool> {
    let result = validate_current_with_details()?;
    Ok(result.success)
}

pub fn validate_current_with_details() -> Result<ValidateResult> {
    let conn = get_db(None)?;

    // Find current in-progress iteration
    let iteration: Option<(i64, i64, String, i32)> = conn
        .query_row(
            "SELECT i.id, i.task_id, t.description, i.attempt_number
             FROM iterations i
             INNER JOIN tasks t ON i.task_id = t.id
             WHERE i.status IN ('in_progress', 'awaiting_approval')
             ORDER BY i.id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .ok();

    let (iteration_id, task_id, task_description, attempt_number) = match iteration {
        Some(i) => i,
        None => {
            return Err(DialError::NoIterationInProgress);
        }
    };

    println!(
        "Validating iteration #{} for task #{}",
        iteration_id, task_id
    );

    // Run validation with detailed step results
    let validation = run_validation_with_details(&conn, iteration_id)?;
    let success = validation.success;
    let error_output = validation.error_output;
    let step_results = validation.step_results;

    if success {
        if let Some(message) = crate::task::current_browser_verification_requirement_message(
            &conn,
            task_id,
            iteration_id,
        )? {
            complete_iteration(
                &conn,
                iteration_id,
                "awaiting_approval",
                None,
                Some(&message),
            )?;
            println!();
            println!("{}", yellow(&message));
            println!(
                "{}",
                yellow("Record the verification, then rerun `dial validate` or `dial approve`.")
            );
            let _ = append_progress_log_entry(&ProgressLogEntry {
                task_id,
                task_description: task_description.clone(),
                iteration_id,
                attempt_number,
                outcome: ProgressOutcome::AwaitingVerification,
                summary: Some(message.clone()),
                changed_files_summary: if git_is_repo() {
                    let summary = git_diff_stat().unwrap_or_default();
                    if summary.trim().is_empty() {
                        None
                    } else {
                        Some(summary)
                    }
                } else {
                    None
                },
                commit_hash: None,
                learnings: Vec::new(),
            });
            let _ = sync_operator_artifacts(&conn);
            return Ok(ValidateResult {
                success: false,
                step_results,
                task_id: Some(task_id),
                suggested_solutions: Vec::new(),
            });
        }

        let changed_files_summary = if git_is_repo() {
            Some(git_diff_stat().unwrap_or_default())
        } else {
            None
        };

        // Commit changes (git operations happen outside the DB transaction)
        let commit_hash = if git_is_repo() && git_has_changes() {
            let message = task_description.to_string();
            match git_commit(&message) {
                Ok(Some(hash)) => {
                    println!("{}", green(&format!("Committed: {}", &hash[..8])));
                    Some(hash)
                }
                Ok(None) => None,
                Err(err) => {
                    let commit_error = format!("Validation passed but commit failed: {}", err);
                    with_transaction(&conn, |conn| {
                        complete_iteration(
                            conn,
                            iteration_id,
                            "failed",
                            None,
                            Some(&commit_error),
                        )?;
                        conn.execute(
                            "UPDATE tasks SET status = 'pending' WHERE id = ?1",
                            [task_id],
                        )?;
                        Ok(())
                    })?;
                    let _ = sync_operator_artifacts(&conn);
                    return Err(DialError::GitError(commit_error));
                }
            }
        } else {
            None
        };

        // Drop checkpoint on success — the stash is no longer needed
        if git_is_repo() && checkpoints_enabled() {
            match checkpoint_drop() {
                Ok(true) => println!("{}", dim("Checkpoint dropped (validation passed)")),
                Ok(false) => {} // no stash to drop
                Err(e) => println!(
                    "{}",
                    yellow(&format!("Warning: checkpoint drop failed: {}", e))
                ),
            }
        }

        // Complete iteration + task + auto-unblock atomically
        with_transaction(&conn, |conn| {
            complete_iteration(
                conn,
                iteration_id,
                "completed",
                commit_hash.as_deref(),
                None,
            )?;

            let now = Local::now().to_rfc3339();
            conn.execute(
                "UPDATE tasks SET status = 'completed', completed_at = ?1 WHERE id = ?2",
                rusqlite::params![now, task_id],
            )?;

            crate::task::auto_unblock_dependents(conn, task_id)?;
            Ok(())
        })?;

        println!(
            "{}",
            green(&format!(
                "\nIteration #{} completed successfully!",
                iteration_id
            ))
        );
        println!(
            "{}",
            green(&format!("Task #{} marked as completed.", task_id))
        );
        let _ = append_progress_log_entry(&ProgressLogEntry {
            task_id,
            task_description: task_description.clone(),
            iteration_id,
            attempt_number,
            outcome: ProgressOutcome::Completed,
            summary: Some("Validation passed".to_string()),
            changed_files_summary,
            commit_hash: commit_hash.clone(),
            learnings: Vec::new(),
        });
        let _ = sync_operator_artifacts(&conn);

        // Prompt for learning capture after success
        println!();
        println!("{}", bold("📝 Learning Capture"));
        println!(
            "{}",
            dim("Did you learn something during this task? Record it now:")
        );
        println!(
            "{}",
            yellow("  dial learn \"what you learned\" -c <category>")
        );
        println!(
            "{}",
            dim("Categories: build, test, setup, gotcha, pattern, tool, other")
        );
        println!();

        Ok(ValidateResult {
            success: true,
            step_results,
            task_id: Some(task_id),
            suggested_solutions: Vec::new(),
        })
    } else {
        // Capture diff before restoring checkpoint (so we can include in retry context)
        let (failed_diff, failed_diff_stat) = if git_is_repo() {
            (
                git_diff().unwrap_or_default(),
                git_diff_stat().unwrap_or_default(),
            )
        } else {
            (String::new(), String::new())
        };

        // Restore checkpoint on failure (rolls back working tree to pre-task state)
        if git_is_repo() && checkpoints_enabled() {
            match checkpoint_restore() {
                Ok(true) => println!("{}", yellow("Checkpoint restored (rolling back changes)")),
                Ok(false) => {} // no checkpoint to restore
                Err(e) => println!(
                    "{}",
                    yellow(&format!("Warning: checkpoint restore failed: {}", e))
                ),
            }
        }

        // Record failure (already wrapped in its own transaction internally)
        let (failure_id, _pattern_id, suggested_solutions) =
            record_failure(&conn, iteration_id, &error_output, None, None)?;
        println!("{}", red(&format!("Recorded failure #{}", failure_id)));

        // Show auto-suggested solutions
        if !suggested_solutions.is_empty() {
            println!("{}", yellow("\nKnown fixes available:"));
            for (_, desc, confidence) in &suggested_solutions {
                println!("  - KNOWN FIX (confidence: {:.2}): {}", confidence, desc);
            }
        }

        // Build notes with diff info for retry context
        let error_preview = if error_output.len() > 500 {
            &error_output[..500]
        } else {
            &error_output
        };

        let notes_string = if !failed_diff.is_empty() || !failed_diff_stat.is_empty() {
            let truncated_diff = if failed_diff.len() > 2000 {
                &failed_diff[..2000]
            } else {
                &failed_diff
            };
            format!(
                "{}\nFAILED_DIFF_STAT:\n{}\nFAILED_DIFF:\n{}",
                error_preview, failed_diff_stat, truncated_diff
            )
        } else {
            error_preview.to_string()
        };

        let mut blocked_due_to_max = false;

        with_transaction(&conn, |conn| {
            complete_iteration(conn, iteration_id, "failed", None, Some(&notes_string))?;

            let fail_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM iterations WHERE task_id = ?1 AND status = 'failed'",
                [task_id],
                |row| row.get(0),
            )?;

            if fail_count >= MAX_FIX_ATTEMPTS as i64 {
                blocked_due_to_max = true;
                println!(
                    "{}",
                    red(&format!("\nMax attempts ({}) reached.", MAX_FIX_ATTEMPTS))
                );

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
                        println!(
                            "{}",
                            yellow(&format!("Reverting to last good commit: {}", &hash[..8]))
                        );
                        git_revert_to(&hash)?;
                    }
                }

                conn.execute(
                    "UPDATE tasks SET status = 'blocked', blocked_by = ?1 WHERE id = ?2",
                    rusqlite::params![format!("Failed {} attempts", MAX_FIX_ATTEMPTS), task_id],
                )?;
            } else {
                conn.execute(
                    "UPDATE tasks SET status = 'pending' WHERE id = ?1",
                    [task_id],
                )?;
                let remaining = MAX_FIX_ATTEMPTS as i64 - fail_count;
                println!(
                    "{}",
                    yellow(&format!(
                        "\nTask reset to pending. {} attempts remaining.",
                        remaining
                    ))
                );
            }

            Ok(())
        })?;

        let _ = append_progress_log_entry(&ProgressLogEntry {
            task_id,
            task_description: task_description.clone(),
            iteration_id,
            attempt_number,
            outcome: if blocked_due_to_max {
                ProgressOutcome::Blocked
            } else {
                ProgressOutcome::Failed
            },
            summary: Some(error_preview.to_string()),
            changed_files_summary: if failed_diff_stat.trim().is_empty() {
                None
            } else {
                Some(failed_diff_stat.clone())
            },
            commit_hash: None,
            learnings: Vec::new(),
        });
        let _ = sync_operator_artifacts(&conn);

        // Build suggested_solutions list for event emission by engine
        let suggested_for_event: Vec<(i64, i64, String, f64)> = suggested_solutions
            .iter()
            .map(|(sol_id, desc, conf)| (failure_id, *sol_id, desc.clone(), *conf))
            .collect();

        Ok(ValidateResult {
            success: false,
            step_results,
            task_id: Some(task_id),
            suggested_solutions: suggested_for_event,
        })
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
                println!(
                    "{}",
                    yellow(&format!("\nReached max iterations ({}). Stopping.", max))
                );
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
            println!(
                "{}",
                dim("\nWaiting for work. Run 'dial validate' after implementing.")
            );
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
            "SELECT id, task_id
             FROM iterations
             WHERE status IN ('in_progress', 'awaiting_approval')
             ORDER BY id DESC LIMIT 1",
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
    println!(
        "{}",
        yellow("Stop flag created. DIAL will stop after current iteration.")
    );
    Ok(())
}

/// Show fresh context for current or next task
pub fn show_context() -> Result<()> {
    let conn = get_db(None)?;

    // Try to find current in-progress task first
    let task: Option<Task> = conn
        .query_row(
            "SELECT t.id, t.description, t.status, t.priority, t.blocked_by, t.spec_section_id, t.created_at, t.started_at, t.completed_at,
                    t.prd_section_id, t.total_attempts, t.total_failures, t.last_failure_at,
                    t.acceptance_criteria_json, t.requires_browser_verification
             FROM tasks t
             INNER JOIN iterations i ON i.task_id = t.id
             WHERE i.status IN ('in_progress', 'awaiting_approval')
             ORDER BY i.id DESC LIMIT 1",
            [],
            |row| Task::from_row(row),
        )
        .ok();

    // If no in-progress task, get next pending task
    let task = match task {
        Some(t) => t,
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, description, status, priority, blocked_by, spec_section_id, created_at, started_at, completed_at,
                        prd_section_id, total_attempts, total_failures, last_failure_at,
                        acceptance_criteria_json, requires_browser_verification
                 FROM tasks WHERE status = 'pending'
                 AND id NOT IN (
                     SELECT td.task_id FROM task_dependencies td
                     INNER JOIN tasks dep ON dep.id = td.depends_on_id
                     WHERE dep.status != 'completed'
                 )
                 ORDER BY priority, id LIMIT 1",
            )?;

            match stmt.query_row([], |row| Task::from_row(row)).ok() {
                Some(t) => t,
                None => {
                    println!("{}", dim("No pending tasks. Task queue empty."));
                    return Ok(());
                }
            }
        }
    };

    println!("{}", bold(&"=".repeat(60)));
    println!("{}", bold(&format!("Fresh Context: Task #{}", task.id)));
    println!("{}", bold(&"=".repeat(60)));
    println!();

    let _ = sync_patterns_digest(&conn);
    let context = gather_context(&conn, &task)?;
    let full_context = format!("# Task: {}\n\n{}", task.description, context);

    println!("{}", full_context);

    // Also write to file
    let context_file = get_dial_dir().join("current_context.md");
    fs::write(&context_file, &full_context)?;
    println!(
        "\n{}",
        dim(&format!("Context written to: {}", context_file.display()))
    );

    Ok(())
}

/// Generate orchestrator prompt for running tasks with fresh sub-agents
pub fn orchestrate() -> Result<()> {
    let conn = get_db(None)?;

    // Get next pending task (dependency-aware)
    let mut stmt = conn.prepare(
        "SELECT id, description, status, priority, blocked_by, spec_section_id, created_at, started_at, completed_at,
                prd_section_id, total_attempts, total_failures, last_failure_at,
                acceptance_criteria_json, requires_browser_verification
         FROM tasks WHERE status = 'pending'
         AND id NOT IN (
             SELECT td.task_id FROM task_dependencies td
             INNER JOIN tasks dep ON dep.id = td.depends_on_id
             WHERE dep.status != 'completed'
         )
         ORDER BY priority, id LIMIT 1",
    )?;

    let task = match stmt.query_row([], |row| Task::from_row(row)).ok() {
        Some(t) => t,
        None => {
            println!("{}", green("All tasks completed! Nothing to orchestrate."));
            return Ok(());
        }
    };

    // Generate the sub-agent prompt
    let _ = sync_patterns_digest(&conn);
    let prompt = generate_subagent_prompt(&conn, &task)?;

    println!("{}", bold(&"=".repeat(70)));
    println!("{}", bold("DIAL Orchestrator Mode"));
    println!("{}", bold(&"=".repeat(70)));
    println!();
    println!(
        "{}",
        dim("Copy the prompt below to spawn a fresh sub-agent for this task.")
    );
    println!(
        "{}",
        dim("After the sub-agent completes, run `dial validate` to commit.")
    );
    println!();
    println!("{}", bold("--- SUB-AGENT PROMPT START ---"));
    println!();
    println!("{}", prompt);
    println!("{}", bold("--- SUB-AGENT PROMPT END ---"));
    println!();

    // Write prompt to file for easy access
    let prompt_file = get_dial_dir().join("subagent_prompt.md");
    fs::write(&prompt_file, &prompt)?;
    println!(
        "{}",
        dim(&format!("Prompt also saved to: {}", prompt_file.display()))
    );

    // Platform hints
    println!();
    println!("{}", bold("Platform Commands:"));
    println!(
        "  {}",
        dim("Claude Code: claude -p \"$(cat .dial/subagent_prompt.md)\"")
    );
    println!(
        "  {}",
        dim("Codex CLI:   codex --task \"$(cat .dial/subagent_prompt.md)\"")
    );
    println!(
        "  {}",
        dim("Gemini:      Copy prompt to new Gemini session")
    );
    println!();

    Ok(())
}
