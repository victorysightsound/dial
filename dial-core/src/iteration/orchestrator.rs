use crate::config::config_get;
use crate::db::{get_db, get_dial_dir};
use crate::errors::{DialError, Result};
use crate::failure::record_failure;
use crate::git::{git_commit, git_has_changes, git_is_repo};
use crate::learning::{add_learning_with_conn, auto_link_pattern_for_iteration};
use crate::output::{bold, dim, green, red, yellow};
use crate::task::auto_unblock_dependents;
use crate::task::models::Task;
use crate::MAX_FIX_ATTEMPTS;
use chrono::Local;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use super::context::generate_autonomous_subagent_prompt;
use super::signal::{read_signal_file, signal_file_to_result};
use super::validation::run_validation;
use super::{complete_iteration, create_iteration};

/// Supported AI CLI tools for orchestration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AiCli {
    ClaudeCode,
    Codex,
    Copilot,
    Gemini,
}

impl AiCli {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" | "claude-code" | "claude_code" => Some(AiCli::ClaudeCode),
            "codex" => Some(AiCli::Codex),
            "copilot" => Some(AiCli::Copilot),
            "gemini" => Some(AiCli::Gemini),
            _ => None,
        }
    }

    fn build_command_for_platform(&self, prompt_file: &str, windows: bool) -> String {
        if windows {
            let prompt_file_ps = prompt_file.replace('\'', "''");
            match self {
                AiCli::ClaudeCode => format!(
                    "claude -p (Get-Content -Raw '{}') 2>&1",
                    prompt_file_ps
                ),
                AiCli::Codex => format!(
                    "chcp 65001>nul && type .dial\\subagent_prompt.md | codex --dangerously-bypass-approvals-and-sandbox -C . exec --skip-git-repo-check 2>&1"
                ),
                AiCli::Copilot => format!(
                    "copilot -p (Get-Content -Raw '{}') -s --allow-all-tools --allow-all-paths --allow-all-urls 2>&1",
                    prompt_file_ps
                ),
                AiCli::Gemini => format!(
                    "Get-Content -Raw '{}' | gemini -p - 2>&1",
                    prompt_file_ps
                ),
            }
        } else {
            match self {
                // claude -p "prompt" (reads prompt, outputs response, exits)
                AiCli::ClaudeCode => format!("claude -p \"$(cat {})\" 2>&1", prompt_file),
                // codex exec "prompt" (non-interactive mode, skip git check for temp dirs)
                AiCli::Codex => format!(
                    "cat {} | codex -a never -s workspace-write -C . exec --skip-git-repo-check 2>&1",
                    prompt_file
                ),
                // copilot -p "prompt" (non-interactive mode, silent output)
                AiCli::Copilot => format!(
                    "copilot -p \"$(cat {})\" -s --allow-all-tools --allow-all-paths --allow-all-urls 2>&1",
                    prompt_file
                ),
                // gemini -p "prompt" (reads from stdin with -)
                AiCli::Gemini => format!("cat {} | gemini -p - 2>&1", prompt_file),
            }
        }
    }

    /// Build the shell command to run this AI CLI with a prompt file
    pub fn build_command(&self, prompt_file: &str) -> String {
        self.build_command_for_platform(prompt_file, cfg!(windows))
    }

    pub fn name(&self) -> &'static str {
        match self {
            AiCli::ClaudeCode => "Claude Code",
            AiCli::Codex => "Codex CLI",
            AiCli::Copilot => "GitHub Copilot CLI",
            AiCli::Gemini => "Gemini CLI",
        }
    }
}

fn shell_program_and_args(ai_cli: AiCli) -> (&'static str, &'static [&'static str]) {
    if cfg!(windows) {
        if ai_cli == AiCli::Codex {
            ("cmd", &["/d", "/s", "/c"])
        } else {
            (
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
                &["-Command"],
            )
        }
    } else {
        ("sh", &["-c"])
    }
}

fn prepend_current_exe_dir_to_path(command: &mut Command) {
    let Ok(exe_path) = std::env::current_exe() else {
        return;
    };
    let Some(exe_dir) = exe_path.parent() else {
        return;
    };

    let mut paths = vec![exe_dir.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }

    if let Ok(joined) = std::env::join_paths(paths) {
        command.env("PATH", joined);
    }
}

/// Parsed signals from AI output
#[derive(Debug, Default)]
pub struct SubagentResult {
    pub complete: bool,
    pub complete_message: Option<String>,
    pub blocked: bool,
    pub blocked_message: Option<String>,
    pub learnings: Vec<(String, String)>, // (category, description)
    pub raw_output: String,
}

impl SubagentResult {
    /// Parse AI output for DIAL signals using robust regex matching
    /// Handles variations like:
    /// - DIAL_COMPLETE: message
    /// - DIAL COMPLETE: message
    /// - **DIAL_COMPLETE:** message
    /// - `DIAL_COMPLETE: message`
    ///
    /// Ignores template placeholders like `<summary>` or `<category>`
    pub fn parse(output: &str) -> Self {
        let mut result = SubagentResult {
            raw_output: output.to_string(),
            ..Default::default()
        };

        // Regex patterns for signal detection (case-insensitive, flexible formatting)
        // Handles: DIAL_COMPLETE:, DIAL COMPLETE:, **DIAL_COMPLETE:**, `DIAL_COMPLETE:`
        let complete_re = Regex::new(r"(?i)[\*`]*DIAL[_\s]COMPLETE[\*`:]+\s*(.+)").unwrap();
        let blocked_re = Regex::new(r"(?i)[\*`]*DIAL[_\s]BLOCKED[\*`:]+\s*(.+)").unwrap();
        let learning_re = Regex::new(r"(?i)[\*`]*DIAL[_\s]LEARNING[\*`:]+\s*(.+)").unwrap();

        // Pattern to detect template placeholders like <summary>, <category>, <reason>
        let placeholder_re = Regex::new(r"<[a-z_\s]+>").unwrap();

        for line in output.lines() {
            let line = line.trim();

            // Skip lines that look like instructions/templates (contain placeholders)
            if placeholder_re.is_match(line) {
                continue;
            }

            // Skip lines that are inside code blocks or are quoting the format
            if line.contains("output:") || line.contains("output `") || line.starts_with("#") {
                continue;
            }

            // DIAL_COMPLETE: <summary>
            if let Some(caps) = complete_re.captures(line) {
                let msg = caps[1].trim().to_string();
                // Only accept if it's real content, not a placeholder
                if !msg.is_empty() && !msg.starts_with('<') {
                    result.complete = true;
                    result.complete_message = Some(msg);
                }
            }
            // DIAL_BLOCKED: <reason>
            else if let Some(caps) = blocked_re.captures(line) {
                let msg = caps[1].trim().to_string();
                if !msg.is_empty() && !msg.starts_with('<') {
                    result.blocked = true;
                    result.blocked_message = Some(msg);
                }
            }
            // DIAL_LEARNING: <category>: <description>
            else if let Some(caps) = learning_re.captures(line) {
                let rest = caps[1].trim();
                // Skip if it looks like a template
                if rest.starts_with('<') {
                    continue;
                }
                if let Some((cat, desc)) = rest.split_once(':') {
                    let cat = cat.trim();
                    let desc = desc.trim();
                    // Skip if either part is a placeholder
                    if !cat.starts_with('<') && !desc.starts_with('<') && !desc.is_empty() {
                        result.learnings.push((cat.to_string(), desc.to_string()));
                    }
                } else if !rest.is_empty() {
                    // No category specified, use "other"
                    result
                        .learnings
                        .push(("other".to_string(), rest.to_string()));
                }
            }
        }

        result
    }
}

/// Run a single task with a fresh AI subprocess
fn run_subagent(ai_cli: AiCli, prompt_file: &str, timeout_secs: u64) -> Result<SubagentResult> {
    println!(
        "{}",
        dim(&format!("Spawning {} subprocess...", ai_cli.name()))
    );

    let shell_cmd = ai_cli.build_command(prompt_file);
    println!("{}", dim(&format!("Command: {}", shell_cmd)));

    let (shell_program, shell_args) = shell_program_and_args(ai_cli);
    let mut command = Command::new(shell_program);
    command.args(shell_args).arg(&shell_cmd);
    prepend_current_exe_dir_to_path(&mut command);

    let mut child = command
        .env_remove("CLAUDECODE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            DialError::CommandFailed(format!("Failed to spawn {}: {}", ai_cli.name(), e))
        })?;

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let mut output = String::new();
    let start = std::time::Instant::now();

    // Stream output and collect it
    for line in reader.lines() {
        if start.elapsed().as_secs() > timeout_secs {
            let _ = child.kill();
            return Err(DialError::CommandFailed(format!(
                "Subagent timed out after {}s",
                timeout_secs
            )));
        }

        match line {
            Ok(line) => {
                // Print output in real-time (dimmed for visual separation)
                println!("  │ {}", dim(&line));
                output.push_str(&line);
                output.push('\n');
            }
            Err(_) => break,
        }
    }

    // Wait for process to complete
    let status = child.wait();
    match status {
        Ok(s) if s.success() => {
            println!("{}", dim("  └─ Process exited successfully"));
        }
        Ok(s) => {
            println!(
                "{}",
                yellow(&format!("  └─ Process exited with code: {}", s))
            );
        }
        Err(e) => {
            println!("{}", red(&format!("  └─ Process error: {}", e)));
        }
    }

    // Prefer structured signal file over regex parsing of stdout
    match read_signal_file() {
        Ok(Some(signal_file)) => {
            println!(
                "{}",
                dim("  ├─ Signal file found: using structured signals")
            );
            Ok(signal_file_to_result(&signal_file, &output))
        }
        Ok(None) => {
            // No signal file — fall back to regex parsing of stdout
            println!(
                "{}",
                dim("  ├─ No signal file: falling back to output parsing")
            );
            Ok(SubagentResult::parse(&output))
        }
        Err(e) => {
            // Signal file exists but couldn't be parsed — warn and fall back
            println!(
                "{}",
                yellow(&format!(
                    "  ├─ Signal file error: {}. Falling back to output parsing.",
                    e
                ))
            );
            Ok(SubagentResult::parse(&output))
        }
    }
}

/// Iteration mode controlling review/approval behavior
#[derive(Debug, Clone, PartialEq)]
pub enum IterationMode {
    /// Run all tasks without stopping
    Autonomous,
    /// Pause for review after every N completed tasks
    ReviewEvery(u32),
    /// Pause after every task for approval
    ReviewEach,
}

impl IterationMode {
    /// Parse from config string (e.g., "autonomous", "review_every:3", "review_each")
    pub fn from_config(s: &str) -> Self {
        if s == "review_each" {
            IterationMode::ReviewEach
        } else if let Some(n_str) = s.strip_prefix("review_every:") {
            match n_str.parse::<u32>() {
                Ok(n) if n > 0 => IterationMode::ReviewEvery(n),
                _ => IterationMode::Autonomous,
            }
        } else {
            IterationMode::Autonomous
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            IterationMode::Autonomous => "autonomous".to_string(),
            IterationMode::ReviewEvery(n) => format!("review_every:{}", n),
            IterationMode::ReviewEach => "review_each".to_string(),
        }
    }
}

/// The main auto-run orchestration loop
pub fn auto_run(max_iterations: Option<u32>, ai_cli_name: Option<&str>) -> Result<()> {
    // Determine which AI CLI to use
    let ai_cli = if let Some(name) = ai_cli_name {
        AiCli::from_str(name).ok_or_else(|| {
            DialError::InvalidConfig(format!(
                "Unknown AI CLI: {}. Use 'claude', 'codex', 'copilot', or 'gemini'",
                name
            ))
        })?
    } else if let Some(configured) = config_get("ai_cli")? {
        AiCli::from_str(&configured).ok_or_else(|| {
            DialError::InvalidConfig(format!(
                "Invalid ai_cli config: {}. Use 'claude', 'codex', 'copilot', or 'gemini'",
                configured
            ))
        })?
    } else {
        // Default to Claude Code
        AiCli::ClaudeCode
    };

    let timeout_secs: u64 = config_get("subagent_timeout")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(1800); // 30 minutes default

    // Read iteration_mode from config
    let iteration_mode = config_get("iteration_mode")?
        .map(|s| IterationMode::from_config(&s))
        .unwrap_or(IterationMode::Autonomous);

    let dial_dir = get_dial_dir();
    let stop_file = dial_dir.join("stop");
    let prompt_file = dial_dir.join("subagent_prompt.md");

    // Remove any existing stop file
    if stop_file.exists() {
        fs::remove_file(&stop_file)?;
    }

    println!("{}", bold(&"=".repeat(70)));
    println!("{}", bold("DIAL Auto-Run: Automated Orchestration Mode"));
    println!("{}", bold(&"=".repeat(70)));
    println!();
    println!("AI CLI:     {}", ai_cli.name());
    println!("Timeout:    {}s per task", timeout_secs);
    println!("Mode:       {}", iteration_mode.display_name());
    if let Some(max) = max_iterations {
        println!("Max tasks:  {}", max);
    }
    println!();
    println!("{}", dim("Create .dial/stop file to stop gracefully."));
    println!();

    let mut completed_count = 0u32;
    let mut failed_count = 0u32;

    loop {
        // Check stop flag
        if stop_file.exists() {
            println!("{}", yellow("\nStop flag detected. Stopping gracefully."));
            fs::remove_file(&stop_file)?;
            break;
        }

        // Check iteration limit
        if let Some(max) = max_iterations {
            if completed_count >= max {
                println!(
                    "{}",
                    yellow(&format!("\nReached max iterations ({}). Stopping.", max))
                );
                break;
            }
        }

        let conn = get_db(None)?;

        // Get next pending task (dependency-aware)
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

        let task: Option<Task> = stmt.query_row([], |row| Task::from_row(row)).ok();

        let task = match task {
            Some(t) => t,
            None => {
                println!();
                println!("{}", bold(&"=".repeat(70)));
                println!("{}", green("All tasks completed!"));
                show_auto_run_summary(completed_count, failed_count)?;
                break;
            }
        };

        println!("{}", bold(&"=".repeat(70)));
        println!(
            "{}",
            bold(&format!("Task #{}: {}", task.id, task.description))
        );
        println!("{}", bold(&"=".repeat(70)));

        // Check cross-iteration chronic failure threshold
        let max_total_failures: i64 = config_get("max_total_failures")?
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);

        let task_total_failures: i64 = conn
            .query_row(
                "SELECT COALESCE(total_failures, 0) FROM tasks WHERE id = ?1",
                [task.id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let task_total_attempts: i64 = conn
            .query_row(
                "SELECT COALESCE(total_attempts, 0) FROM tasks WHERE id = ?1",
                [task.id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if task_total_failures >= max_total_failures {
            let reason = format!(
                "Chronic failure: {} failures across {} attempts",
                task_total_failures, task_total_attempts
            );
            println!(
                "{}",
                red(&format!(
                    "Task #{} is a chronic failure ({}). Auto-blocking.",
                    task.id, reason
                ))
            );

            conn.execute(
                "UPDATE tasks SET status = 'blocked', blocked_by = ?1 WHERE id = ?2",
                rusqlite::params![reason, task.id],
            )?;

            failed_count += 1;
            continue;
        }

        // Check attempt count (per-iteration MAX_FIX_ATTEMPTS)
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
                    "Task #{} has failed {} times. Blocking and skipping.",
                    task.id, MAX_FIX_ATTEMPTS
                ))
            );

            conn.execute(
                "UPDATE tasks SET status = 'blocked', blocked_by = ?1 WHERE id = ?2",
                rusqlite::params![format!("Failed {} times", MAX_FIX_ATTEMPTS), task.id],
            )?;

            failed_count += 1;
            continue;
        }

        println!("Attempt {} of {}", attempt_number, MAX_FIX_ATTEMPTS);

        // Create iteration record
        let iteration_id = create_iteration(&conn, task.id, attempt_number)?;

        // Generate sub-agent prompt
        let prompt = generate_autonomous_subagent_prompt(&conn, &task)?;
        fs::write(&prompt_file, &prompt)?;
        println!(
            "{}",
            dim(&format!("Prompt written to: {}", prompt_file.display()))
        );

        // Spawn the sub-agent
        println!();
        let result = run_subagent(ai_cli, prompt_file.to_str().unwrap(), timeout_secs)?;

        // Process learnings — auto-link to failure pattern if current iteration has failures
        let auto_pattern_id = auto_link_pattern_for_iteration(&conn, iteration_id);
        let mut seen_learnings = HashSet::new();
        for (category, description) in &result.learnings {
            let dedupe_key = (
                category.trim().to_ascii_lowercase(),
                description.trim().to_string(),
            );
            if !seen_learnings.insert(dedupe_key) {
                continue;
            }
            let pattern_str = auto_pattern_id
                .map(|pid| format!(" (linked to pattern #{})", pid))
                .unwrap_or_default();
            println!(
                "{}",
                green(&format!(
                    "Learning captured: [{}]{} {}",
                    category, pattern_str, description
                ))
            );
            let _ = add_learning_with_conn(
                &conn,
                description,
                Some(category),
                auto_pattern_id,
                Some(iteration_id),
            );
        }

        // Handle blocked
        if result.blocked {
            let msg = result
                .blocked_message
                .as_deref()
                .unwrap_or("Unknown blocker");
            println!("{}", red(&format!("\nSubagent blocked: {}", msg)));

            complete_iteration(&conn, iteration_id, "failed", None, Some(msg))?;

            conn.execute(
                "UPDATE tasks SET status = 'blocked', blocked_by = ?1 WHERE id = ?2",
                rusqlite::params![msg, task.id],
            )?;

            failed_count += 1;
            continue;
        }

        // Handle completion
        if result.complete {
            let msg = result
                .complete_message
                .as_deref()
                .unwrap_or("Task completed");
            println!("{}", green(&format!("\nSubagent completed: {}", msg)));

            // Run validation
            println!();
            println!("{}", bold("Running validation..."));
            let (success, error_output) = run_validation(&conn, iteration_id)?;

            if success {
                // Commit changes
                let commit_hash = if git_is_repo() && git_has_changes() {
                    let commit_msg = task.description.clone();
                    match git_commit(&commit_msg) {
                        Ok(Some(hash)) => {
                            println!("{}", green(&format!("Committed: {}", &hash[..8])));
                            Some(hash)
                        }
                        Ok(None) => None,
                        Err(err) => {
                            let commit_error =
                                format!("Validation passed but commit failed: {}", err);
                            println!("{}", red(&commit_error));
                            complete_iteration(
                                &conn,
                                iteration_id,
                                "failed",
                                None,
                                Some(&commit_error),
                            )?;
                            conn.execute(
                                "UPDATE tasks SET status = 'pending' WHERE id = ?1",
                                [task.id],
                            )?;
                            return Err(DialError::GitError(commit_error));
                        }
                    }
                } else {
                    None
                };

                complete_iteration(
                    &conn,
                    iteration_id,
                    "completed",
                    commit_hash.as_deref(),
                    Some(msg),
                )?;

                let now = Local::now().to_rfc3339();
                conn.execute(
                    "UPDATE tasks SET status = 'completed', completed_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, task.id],
                )?;

                // Auto-unblock dependents
                auto_unblock_dependents(&conn, task.id)?;

                println!(
                    "{}",
                    green(&format!("Task #{} completed successfully!", task.id))
                );
                completed_count += 1;

                // Check iteration_mode for review pause
                let should_pause = match &iteration_mode {
                    IterationMode::ReviewEach => true,
                    IterationMode::ReviewEvery(n) => completed_count % n == 0,
                    IterationMode::Autonomous => false,
                };

                if should_pause {
                    // Set the iteration to awaiting_approval so approve/reject flow works
                    conn.execute(
                        "UPDATE iterations SET status = 'awaiting_approval' WHERE id = ?1",
                        [iteration_id],
                    )?;

                    println!();
                    println!(
                        "{}",
                        yellow(&format!(
                            "Review pause ({}) — {} task(s) completed.",
                            iteration_mode.display_name(),
                            completed_count
                        ))
                    );
                    println!(
                        "{}",
                        yellow("Run `dial approve` to continue or `dial reject` to stop.")
                    );
                    break;
                }
            } else {
                // Validation failed
                println!("{}", red("Validation failed."));

                let (failure_id, _pattern_id, suggested_solutions) =
                    record_failure(&conn, iteration_id, &error_output, None, None)?;
                println!("{}", dim(&format!("Recorded failure #{}", failure_id)));

                // Show auto-suggested solutions
                if !suggested_solutions.is_empty() {
                    println!("{}", yellow("Known fixes available for next attempt:"));
                    for (_, desc, confidence) in &suggested_solutions {
                        println!("  - KNOWN FIX (confidence: {:.2}): {}", confidence, desc);
                    }
                }

                complete_iteration(
                    &conn,
                    iteration_id,
                    "failed",
                    None,
                    Some(&error_output[..error_output.len().min(500)]),
                )?;

                // Reset task for retry
                conn.execute(
                    "UPDATE tasks SET status = 'pending' WHERE id = ?1",
                    [task.id],
                )?;

                let remaining = MAX_FIX_ATTEMPTS as i32 - attempt_number;
                println!(
                    "{}",
                    yellow(&format!("Task reset. {} attempts remaining.", remaining))
                );
            }
        } else {
            // No completion signal - treat as incomplete
            println!(
                "{}",
                yellow("\nNo DIAL_COMPLETE signal received. Treating as incomplete.")
            );

            complete_iteration(
                &conn,
                iteration_id,
                "failed",
                None,
                Some("No completion signal"),
            )?;

            conn.execute(
                "UPDATE tasks SET status = 'pending' WHERE id = ?1",
                [task.id],
            )?;

            let remaining = MAX_FIX_ATTEMPTS as i32 - attempt_number;
            println!(
                "{}",
                yellow(&format!("Task reset. {} attempts remaining.", remaining))
            );
        }

        println!();
    }

    Ok(())
}

fn show_auto_run_summary(completed: u32, failed: u32) -> Result<()> {
    println!("{}", bold(&"=".repeat(70)));
    println!("{}", bold("Auto-Run Summary"));
    println!("{}", "=".repeat(70));
    println!();
    println!("  Completed: {}", green(&completed.to_string()));
    println!(
        "  Failed:    {}",
        if failed > 0 {
            red(&failed.to_string())
        } else {
            failed.to_string()
        }
    );
    println!();

    let conn = get_db(None)?;

    // Show remaining tasks
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
        [],
        |row| row.get(0),
    )?;

    let blocked: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'blocked'",
        [],
        |row| row.get(0),
    )?;

    if pending > 0 {
        println!("  Pending:   {}", pending);
    }
    if blocked > 0 {
        println!("  Blocked:   {}", red(&blocked.to_string()));
    }

    // Show learnings added
    let learnings: i64 = conn.query_row(
        "SELECT COUNT(*) FROM learnings WHERE DATE(discovered_at) = DATE('now')",
        [],
        |row| row.get(0),
    )?;

    if learnings > 0 {
        println!("  Learnings: {} captured today", learnings);
    }

    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::signal::{self, SignalFile, SubagentSignal};
    use super::*;

    #[test]
    fn test_parse_complete_signal() {
        let output = "Some output\nDIAL_COMPLETE: Task done successfully\nMore output";
        let result = SubagentResult::parse(output);
        assert!(result.complete);
        assert_eq!(
            result.complete_message,
            Some("Task done successfully".to_string())
        );
    }

    #[test]
    fn test_parse_complete_with_spaces() {
        let output = "DIAL COMPLETE: Also works with space";
        let result = SubagentResult::parse(output);
        assert!(result.complete);
        assert_eq!(
            result.complete_message,
            Some("Also works with space".to_string())
        );
    }

    #[test]
    fn test_parse_complete_with_markdown() {
        let output = "**DIAL_COMPLETE:** Markdown formatted";
        let result = SubagentResult::parse(output);
        assert!(result.complete);
        assert_eq!(
            result.complete_message,
            Some("Markdown formatted".to_string())
        );
    }

    #[test]
    fn test_parse_blocked_signal() {
        let output = "DIAL_BLOCKED: Missing credentials";
        let result = SubagentResult::parse(output);
        assert!(result.blocked);
        assert_eq!(
            result.blocked_message,
            Some("Missing credentials".to_string())
        );
    }

    #[test]
    fn test_parse_learning_with_category() {
        let output = "DIAL_LEARNING: pattern: Always use parameterized SQL";
        let result = SubagentResult::parse(output);
        assert_eq!(result.learnings.len(), 1);
        assert_eq!(result.learnings[0].0, "pattern");
        assert_eq!(result.learnings[0].1, "Always use parameterized SQL");
    }

    #[test]
    fn test_parse_learning_without_category() {
        let output = "DIAL_LEARNING: This is a general learning";
        let result = SubagentResult::parse(output);
        assert_eq!(result.learnings.len(), 1);
        assert_eq!(result.learnings[0].0, "other");
        assert_eq!(result.learnings[0].1, "This is a general learning");
    }

    #[test]
    fn test_parse_multiple_signals() {
        let output = r#"
Starting task...
DIAL_LEARNING: gotcha: Watch out for null pointers
Doing work...
DIAL_LEARNING: pattern: Use Option<T> instead of nulls
DIAL_COMPLETE: Implemented the feature
"#;
        let result = SubagentResult::parse(output);
        assert!(result.complete);
        assert_eq!(result.learnings.len(), 2);
    }

    #[test]
    fn test_parse_case_insensitive() {
        let output = "dial_complete: lowercase works too";
        let result = SubagentResult::parse(output);
        assert!(result.complete);
    }

    #[test]
    fn test_parse_no_signals() {
        let output = "Just regular output without any DIAL signals";
        let result = SubagentResult::parse(output);
        assert!(!result.complete);
        assert!(!result.blocked);
        assert!(result.learnings.is_empty());
    }

    #[test]
    fn test_codex_build_command_for_windows_uses_cmd_utf8_pipeline() {
        let cmd = AiCli::Codex.build_command_for_platform(r"C:\tmp\prompt.md", true);
        assert_eq!(
            cmd,
            "chcp 65001>nul && type .dial\\subagent_prompt.md | codex --dangerously-bypass-approvals-and-sandbox -C . exec --skip-git-repo-check 2>&1"
        );
    }

    #[test]
    fn test_codex_build_command_for_unix_sets_never_approval() {
        let cmd = AiCli::Codex.build_command_for_platform("/tmp/prompt.md", false);
        assert_eq!(
            cmd,
            "cat /tmp/prompt.md | codex -a never -s workspace-write -C . exec --skip-git-repo-check 2>&1"
        );
    }

    #[test]
    fn test_gemini_build_command_for_windows_uses_powershell_pipeline() {
        let cmd = AiCli::Gemini.build_command_for_platform(r"C:\tmp\prompt.md", true);
        assert_eq!(
            cmd,
            "Get-Content -Raw 'C:\\tmp\\prompt.md' | gemini -p - 2>&1"
        );
    }

    #[test]
    fn test_shell_program_and_args_match_current_platform() {
        let (program, args) = shell_program_and_args(AiCli::ClaudeCode);
        if cfg!(windows) {
            assert!(program.ends_with("powershell.exe"));
            assert_eq!(args, &["-Command"]);
            let (codex_program, codex_args) = shell_program_and_args(AiCli::Codex);
            assert_eq!(codex_program, "cmd");
            assert_eq!(codex_args, &["/d", "/s", "/c"]);
        } else {
            assert_eq!(program, "sh");
            assert_eq!(args, &["-c"]);
        }
    }

    #[test]
    fn test_parse_ignores_template_placeholders() {
        // This is what the prompt instructions look like - should be ignored
        let output = r#"
3. When done, output: `DIAL_COMPLETE: <summary of what was done>`
4. If blocked, output: `DIAL_BLOCKED: <what is blocking>`
5. If you learned something valuable, output: `DIAL_LEARNING: <category>: <what you learned>`

Now doing actual work...
DIAL_COMPLETE: Implemented the feature successfully
"#;
        let result = SubagentResult::parse(output);
        assert!(result.complete);
        assert_eq!(
            result.complete_message,
            Some("Implemented the feature successfully".to_string())
        );
        assert!(!result.blocked); // Should NOT match the template
        assert!(result.learnings.is_empty()); // Should NOT match the template
    }

    #[test]
    fn test_parse_ignores_instruction_lines() {
        let output = "When done, output: DIAL_COMPLETE: your message here";
        let result = SubagentResult::parse(output);
        assert!(!result.complete); // Should be ignored as it contains "output:"
    }

    #[test]
    fn test_iteration_mode_autonomous() {
        assert_eq!(
            IterationMode::from_config("autonomous"),
            IterationMode::Autonomous
        );
    }

    #[test]
    fn test_iteration_mode_review_each() {
        assert_eq!(
            IterationMode::from_config("review_each"),
            IterationMode::ReviewEach
        );
    }

    #[test]
    fn test_iteration_mode_review_every() {
        assert_eq!(
            IterationMode::from_config("review_every:3"),
            IterationMode::ReviewEvery(3)
        );
        assert_eq!(
            IterationMode::from_config("review_every:1"),
            IterationMode::ReviewEvery(1)
        );
        assert_eq!(
            IterationMode::from_config("review_every:10"),
            IterationMode::ReviewEvery(10)
        );
    }

    #[test]
    fn test_iteration_mode_review_every_invalid_falls_back() {
        // Zero or negative → falls back to autonomous
        assert_eq!(
            IterationMode::from_config("review_every:0"),
            IterationMode::Autonomous
        );
        assert_eq!(
            IterationMode::from_config("review_every:abc"),
            IterationMode::Autonomous
        );
    }

    #[test]
    fn test_iteration_mode_unknown_falls_back() {
        assert_eq!(
            IterationMode::from_config("unknown"),
            IterationMode::Autonomous
        );
        assert_eq!(IterationMode::from_config(""), IterationMode::Autonomous);
    }

    #[test]
    fn test_iteration_mode_display_name() {
        assert_eq!(IterationMode::Autonomous.display_name(), "autonomous");
        assert_eq!(IterationMode::ReviewEach.display_name(), "review_each");
        assert_eq!(
            IterationMode::ReviewEvery(5).display_name(),
            "review_every:5"
        );
    }

    // --- Signal file integration tests ---

    /// Helper: set up a temp dir with `.dial/` subdirectory, return TempDir and signal path.
    /// Uses path-based functions — no CWD change needed, safe for parallel tests.
    fn setup_temp_signal_path() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dial_dir = tmp.path().join(".dial");
        std::fs::create_dir_all(&dial_dir).unwrap();
        let path = signal::signal_file_path_at(tmp.path());
        (tmp, path)
    }

    #[test]
    fn test_signal_file_preferred_over_regex() {
        let (_tmp, path) = setup_temp_signal_path();

        // Write a signal file that says "complete"
        let sf = SignalFile {
            signals: vec![SubagentSignal::Complete {
                summary: "Done via signal file".to_string(),
            }],
            timestamp: "2026-03-12T10:00:00Z".to_string(),
        };
        signal::write_signal_file_at(&path, &sf).unwrap();

        // Also provide output with a DIFFERENT DIAL_COMPLETE message
        let output = "DIAL_COMPLETE: Done via regex";

        // read_signal_file should consume the file
        let signal_result = signal::read_signal_file_at(&path).unwrap();
        assert!(signal_result.is_some());

        let result = signal::signal_file_to_result(&signal_result.unwrap(), output);
        // Should use signal file values, not regex
        assert!(result.complete);
        assert_eq!(
            result.complete_message,
            Some("Done via signal file".to_string())
        );
    }

    #[test]
    fn test_fallback_to_regex_when_no_signal_file() {
        let (_tmp, path) = setup_temp_signal_path();

        // No signal file written — read returns None
        let signal_result = signal::read_signal_file_at(&path).unwrap();
        assert!(signal_result.is_none());

        // Regex parsing should work as fallback
        let output = "DIAL_COMPLETE: Done via regex fallback";
        let result = SubagentResult::parse(output);
        assert!(result.complete);
        assert_eq!(
            result.complete_message,
            Some("Done via regex fallback".to_string())
        );
    }

    #[test]
    fn test_fallback_to_regex_on_invalid_signal_file() {
        let (_tmp, path) = setup_temp_signal_path();

        // Write invalid JSON to the signal file
        std::fs::write(&path, "{{bad json}}").unwrap();

        // read_signal_file should return an error
        let result = signal::read_signal_file_at(&path);
        assert!(result.is_err());

        // In the orchestrator, this error triggers regex fallback
        let output = "DIAL_COMPLETE: Recovered via regex";
        let parsed = SubagentResult::parse(output);
        assert!(parsed.complete);
        assert_eq!(
            parsed.complete_message,
            Some("Recovered via regex".to_string())
        );
    }

    #[test]
    fn test_signal_file_with_mock_subagent_flow() {
        // Simulates the full orchestrator signal flow:
        // 1. Subagent writes signal.json
        // 2. Orchestrator reads it
        // 3. Converts to SubagentResult
        // 4. Processes learnings and completion
        let (_tmp, path) = setup_temp_signal_path();

        // Simulate subagent writing signal file
        let sf = SignalFile {
            signals: vec![
                SubagentSignal::Learning {
                    category: "pattern".to_string(),
                    description: "Always check return values".to_string(),
                },
                SubagentSignal::Learning {
                    category: "gotcha".to_string(),
                    description: "Timeouts need explicit handling".to_string(),
                },
                SubagentSignal::Complete {
                    summary: "Implemented error handling with proper timeouts".to_string(),
                },
            ],
            timestamp: "2026-03-12T10:30:00Z".to_string(),
        };
        signal::write_signal_file_at(&path, &sf).unwrap();

        // Simulate orchestrator reading signals after subprocess exits
        let raw_output = "... lots of subprocess output ...";
        let signal_result = signal::read_signal_file_at(&path).unwrap().unwrap();

        // File should be deleted after read
        assert!(!path.exists());

        // Convert to SubagentResult
        let result = signal::signal_file_to_result(&signal_result, raw_output);

        // Verify all signals were captured
        assert!(result.complete);
        assert_eq!(
            result.complete_message,
            Some("Implemented error handling with proper timeouts".to_string())
        );
        assert!(!result.blocked);
        assert_eq!(result.learnings.len(), 2);
        assert_eq!(result.learnings[0].0, "pattern");
        assert_eq!(result.learnings[0].1, "Always check return values");
        assert_eq!(result.learnings[1].0, "gotcha");
        assert_eq!(result.learnings[1].1, "Timeouts need explicit handling");
        assert_eq!(result.raw_output, raw_output);
    }
}
