use crate::config::config_get;
use crate::db::{get_db, get_dial_dir};
use crate::errors::{DialError, Result};
use crate::failure::{find_trusted_solutions, record_failure};
use crate::git::{git_commit, git_has_changes, git_is_repo};
use crate::learning::add_learning;
use crate::output::{bold, dim, green, red, yellow};
use crate::task::models::Task;
use crate::MAX_FIX_ATTEMPTS;
use chrono::Local;
use regex::Regex;
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use super::context::generate_subagent_prompt;
use super::validation::run_validation;
use super::{complete_iteration, create_iteration};

/// Supported AI CLI tools for orchestration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AiCli {
    ClaudeCode,
    Codex,
    Gemini,
}

impl AiCli {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" | "claude-code" | "claude_code" => Some(AiCli::ClaudeCode),
            "codex" => Some(AiCli::Codex),
            "gemini" => Some(AiCli::Gemini),
            _ => None,
        }
    }

    /// Build the shell command to run this AI CLI with a prompt file
    pub fn build_command(&self, prompt_file: &str) -> String {
        match self {
            // claude -p "prompt" (reads prompt, outputs response, exits)
            AiCli::ClaudeCode => format!(
                "claude -p \"$(cat {})\" 2>&1",
                prompt_file
            ),
            // codex exec "prompt" (non-interactive mode, skip git check for temp dirs)
            AiCli::Codex => format!(
                "cat {} | codex exec --skip-git-repo-check 2>&1",
                prompt_file
            ),
            // gemini -p "prompt" (reads from stdin with -)
            AiCli::Gemini => format!(
                "cat {} | gemini -p - 2>&1",
                prompt_file
            ),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            AiCli::ClaudeCode => "Claude Code",
            AiCli::Codex => "Codex CLI",
            AiCli::Gemini => "Gemini CLI",
        }
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
                    result.learnings.push(("other".to_string(), rest.to_string()));
                }
            }
        }

        result
    }
}

/// Run a single task with a fresh AI subprocess
fn run_subagent(ai_cli: AiCli, prompt_file: &str, timeout_secs: u64) -> Result<SubagentResult> {
    println!("{}", dim(&format!("Spawning {} subprocess...", ai_cli.name())));

    let shell_cmd = ai_cli.build_command(prompt_file);
    println!("{}", dim(&format!("Command: {}", shell_cmd)));

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&shell_cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DialError::CommandFailed(format!("Failed to spawn {}: {}", ai_cli.name(), e)))?;

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
            println!("{}", yellow(&format!("  └─ Process exited with code: {}", s)));
        }
        Err(e) => {
            println!("{}", red(&format!("  └─ Process error: {}", e)));
        }
    }

    Ok(SubagentResult::parse(&output))
}

/// The main auto-run orchestration loop
pub fn auto_run(max_iterations: Option<u32>, ai_cli_name: Option<&str>) -> Result<()> {
    // Determine which AI CLI to use
    let ai_cli = if let Some(name) = ai_cli_name {
        AiCli::from_str(name).ok_or_else(|| {
            DialError::InvalidConfig(format!(
                "Unknown AI CLI: {}. Use 'claude', 'codex', or 'gemini'",
                name
            ))
        })?
    } else if let Some(configured) = config_get("ai_cli")? {
        AiCli::from_str(&configured).ok_or_else(|| {
            DialError::InvalidConfig(format!(
                "Invalid ai_cli config: {}. Use 'claude', 'codex', or 'gemini'",
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
                println!("{}", yellow(&format!("\nReached max iterations ({}). Stopping.", max)));
                break;
            }
        }

        let conn = get_db(None)?;

        // Get next pending task
        let mut stmt = conn.prepare(
            "SELECT id, description, status, priority, blocked_by, spec_section_id, created_at, started_at, completed_at
             FROM tasks WHERE status = 'pending'
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
        println!("{}", bold(&format!("Task #{}: {}", task.id, task.description)));
        println!("{}", bold(&"=".repeat(70)));

        // Check attempt count
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
        let prompt = generate_subagent_prompt(&conn, &task)?;
        fs::write(&prompt_file, &prompt)?;
        println!("{}", dim(&format!("Prompt written to: {}", prompt_file.display())));

        // Spawn the sub-agent
        println!();
        let result = run_subagent(ai_cli, prompt_file.to_str().unwrap(), timeout_secs)?;

        // Process learnings
        for (category, description) in &result.learnings {
            println!("{}", green(&format!("Learning captured: [{}] {}", category, description)));
            let _ = add_learning(&description, Some(category));
        }

        // Handle blocked
        if result.blocked {
            let msg = result.blocked_message.as_deref().unwrap_or("Unknown blocker");
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
            let msg = result.complete_message.as_deref().unwrap_or("Task completed");
            println!("{}", green(&format!("\nSubagent completed: {}", msg)));

            // Run validation
            println!();
            println!("{}", bold("Running validation..."));
            let (success, error_output) = run_validation(&conn, iteration_id)?;

            if success {
                // Commit changes
                let commit_hash = if git_is_repo() && git_has_changes() {
                    let commit_msg = format!("DIAL: {}", task.description);
                    if let Some(hash) = git_commit(&commit_msg)? {
                        println!("{}", green(&format!("Committed: {}", &hash[..8])));
                        Some(hash)
                    } else {
                        None
                    }
                } else {
                    None
                };

                complete_iteration(&conn, iteration_id, "completed", commit_hash.as_deref(), Some(msg))?;

                let now = Local::now().to_rfc3339();
                conn.execute(
                    "UPDATE tasks SET status = 'completed', completed_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, task.id],
                )?;

                println!("{}", green(&format!("Task #{} completed successfully!", task.id)));
                completed_count += 1;
            } else {
                // Validation failed
                println!("{}", red("Validation failed."));

                let (failure_id, pattern_id) = record_failure(&conn, iteration_id, &error_output, None, None)?;
                println!("{}", dim(&format!("Recorded failure #{}", failure_id)));

                // Check for trusted solutions
                let solutions = find_trusted_solutions(&conn, pattern_id)?;
                if !solutions.is_empty() {
                    println!("{}", yellow("Trusted solutions available for next attempt:"));
                    for sol in &solutions {
                        println!("  - {}", sol.description);
                    }
                }

                complete_iteration(&conn, iteration_id, "failed", None, Some(&error_output[..error_output.len().min(500)]))?;

                // Reset task for retry
                conn.execute(
                    "UPDATE tasks SET status = 'pending' WHERE id = ?1",
                    [task.id],
                )?;

                let remaining = MAX_FIX_ATTEMPTS as i32 - attempt_number;
                println!("{}", yellow(&format!("Task reset. {} attempts remaining.", remaining)));
            }
        } else {
            // No completion signal - treat as incomplete
            println!("{}", yellow("\nNo DIAL_COMPLETE signal received. Treating as incomplete."));

            complete_iteration(&conn, iteration_id, "failed", None, Some("No completion signal"))?;

            conn.execute(
                "UPDATE tasks SET status = 'pending' WHERE id = ?1",
                [task.id],
            )?;

            let remaining = MAX_FIX_ATTEMPTS as i32 - attempt_number;
            println!("{}", yellow(&format!("Task reset. {} attempts remaining.", remaining)));
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
    println!("  Failed:    {}", if failed > 0 { red(&failed.to_string()) } else { failed.to_string() });
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
    use super::*;

    #[test]
    fn test_parse_complete_signal() {
        let output = "Some output\nDIAL_COMPLETE: Task done successfully\nMore output";
        let result = SubagentResult::parse(output);
        assert!(result.complete);
        assert_eq!(result.complete_message, Some("Task done successfully".to_string()));
    }

    #[test]
    fn test_parse_complete_with_spaces() {
        let output = "DIAL COMPLETE: Also works with space";
        let result = SubagentResult::parse(output);
        assert!(result.complete);
        assert_eq!(result.complete_message, Some("Also works with space".to_string()));
    }

    #[test]
    fn test_parse_complete_with_markdown() {
        let output = "**DIAL_COMPLETE:** Markdown formatted";
        let result = SubagentResult::parse(output);
        assert!(result.complete);
        assert_eq!(result.complete_message, Some("Markdown formatted".to_string()));
    }

    #[test]
    fn test_parse_blocked_signal() {
        let output = "DIAL_BLOCKED: Missing credentials";
        let result = SubagentResult::parse(output);
        assert!(result.blocked);
        assert_eq!(result.blocked_message, Some("Missing credentials".to_string()));
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
        assert_eq!(result.complete_message, Some("Implemented the feature successfully".to_string()));
        assert!(!result.blocked); // Should NOT match the template
        assert!(result.learnings.is_empty()); // Should NOT match the template
    }

    #[test]
    fn test_parse_ignores_instruction_lines() {
        let output = "When done, output: DIAL_COMPLETE: your message here";
        let result = SubagentResult::parse(output);
        assert!(!result.complete); // Should be ignored as it contains "output:"
    }
}
