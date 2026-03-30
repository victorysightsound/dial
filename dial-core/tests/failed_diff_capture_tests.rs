use std::env;
use std::fs;
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

use dial_core::budget::FAILED_DIFF_PRIORITY;
use dial_core::db::schema;
use dial_core::git::{git_diff, git_diff_stat, git_has_changes};
use dial_core::iteration::context::{
    extract_failed_diff_parts, gather_context, gather_context_items,
};
use dial_core::task::models::{Task, TaskStatus};
use rusqlite::Connection;

// Serialize tests that change the process-global current directory.
static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Set up a temp directory initialized as a git repo with an initial commit.
fn setup_git_repo() -> (TempDir, std::path::PathBuf) {
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    Command::new("git").args(["init"]).output().unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@dial.dev"])
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "DIAL Test"])
        .output()
        .unwrap();

    // Create an initial file and commit so there's a HEAD
    fs::write(tmp.path().join("README.md"), "# Test project\n").unwrap();
    Command::new("git").args(["add", "-A"]).output().unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .output()
        .unwrap();

    (tmp, original_dir)
}

/// Set up an in-memory DB with schema + migration columns for testing.
fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .unwrap();
    conn.execute_batch(schema::SCHEMA).unwrap();
    conn.execute_batch(
        r#"
        ALTER TABLE learnings ADD COLUMN pattern_id INTEGER REFERENCES failure_patterns(id);
        ALTER TABLE learnings ADD COLUMN iteration_id INTEGER REFERENCES iterations(id);
        ALTER TABLE tasks ADD COLUMN prd_section_id TEXT;
        ALTER TABLE tasks ADD COLUMN total_attempts INTEGER DEFAULT 0;
        ALTER TABLE tasks ADD COLUMN total_failures INTEGER DEFAULT 0;
        ALTER TABLE tasks ADD COLUMN last_failure_at TEXT;
        ALTER TABLE tasks ADD COLUMN acceptance_criteria_json TEXT;
        ALTER TABLE tasks ADD COLUMN requires_browser_verification INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE failure_patterns ADD COLUMN regex_pattern TEXT;
        ALTER TABLE failure_patterns ADD COLUMN status TEXT NOT NULL DEFAULT 'trusted';
        ALTER TABLE solutions ADD COLUMN source TEXT NOT NULL DEFAULT 'auto-learned';
        ALTER TABLE solutions ADD COLUMN last_validated_at TEXT;
        ALTER TABLE solutions ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
        "#,
    )
    .unwrap();
    conn
}

fn make_test_task(id: i64) -> Task {
    Task {
        id,
        description: "test task".to_string(),
        status: TaskStatus::InProgress,
        priority: 5,
        blocked_by: None,
        spec_section_id: None,
        prd_section_id: None,
        created_at: "2026-01-01T00:00:00".to_string(),
        started_at: None,
        completed_at: None,
        total_attempts: 0,
        total_failures: 0,
        last_failure_at: None,
        acceptance_criteria: Vec::new(),
        requires_browser_verification: false,
    }
}

// ---- git_diff integration tests ----

#[test]
fn test_git_diff_captures_unstaged_changes() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (tmp, original_dir) = setup_git_repo();

    // Modify the tracked file
    fs::write(
        tmp.path().join("README.md"),
        "# Modified project\nNew line\n",
    )
    .unwrap();
    assert!(git_has_changes());

    let diff = git_diff().unwrap();
    assert!(
        diff.contains("+# Modified project"),
        "Diff should contain the added line. Got:\n{}",
        diff
    );
    assert!(
        diff.contains("+New line"),
        "Diff should contain the new line. Got:\n{}",
        diff
    );

    let stat = git_diff_stat().unwrap();
    assert!(
        stat.contains("README.md"),
        "Diff stat should mention the changed file. Got:\n{}",
        stat
    );
    assert!(
        stat.contains("changed"),
        "Diff stat should indicate changes. Got:\n{}",
        stat
    );

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_git_diff_empty_on_clean_tree() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_tmp, original_dir) = setup_git_repo();

    // No changes — diff should be empty
    let diff = git_diff().unwrap();
    assert!(diff.is_empty(), "Diff should be empty on clean tree");

    let stat = git_diff_stat().unwrap();
    assert!(stat.is_empty(), "Diff stat should be empty on clean tree");

    env::set_current_dir(original_dir).unwrap();
}

// ---- Diff truncation tests ----

#[test]
fn test_diff_truncation_to_2000_chars() {
    // Simulate what happens in validate_current_with_details when a diff is > 2000 chars
    let long_diff = "x".repeat(5000);
    let error_output = "build failed";

    // This is the exact truncation logic from iteration/mod.rs
    let truncated_diff = if long_diff.len() > 2000 {
        &long_diff[..2000]
    } else {
        &long_diff
    };
    let notes = format!(
        "{}\nFAILED_DIFF_STAT:\nstat line\nFAILED_DIFF:\n{}",
        error_output, truncated_diff
    );

    // Verify truncation happened
    let result = extract_failed_diff_parts(&notes);
    assert!(result.is_some());
    let (error, stat, diff) = result.unwrap();
    assert_eq!(error, "build failed");
    assert!(stat.contains("stat line"));
    assert_eq!(diff.len(), 2000);
}

#[test]
fn test_diff_not_truncated_when_under_limit() {
    let short_diff = "x".repeat(500);
    let notes = format!(
        "error\nFAILED_DIFF_STAT:\nstat\nFAILED_DIFF:\n{}",
        short_diff
    );

    let result = extract_failed_diff_parts(&notes);
    assert!(result.is_some());
    let (_, _, diff) = result.unwrap();
    assert_eq!(diff.len(), 500);
}

// ---- Full fail→capture→retry context cycle ----

#[test]
fn test_full_fail_capture_retry_cycle() {
    // Simulates:
    // 1. Task fails validation → diff captured in notes
    // 2. On retry, context assembly includes the failed diff

    let conn = setup_test_db();

    // Create a task
    conn.execute(
        "INSERT INTO tasks (description, status) VALUES ('implement feature X', 'in_progress')",
        [],
    )
    .unwrap();
    let task_id = conn.last_insert_rowid();

    // Simulate first attempt failure with diff captured in notes
    let error = "error[E0308]: mismatched types";
    let diff_stat = " src/lib.rs | 5 +++++\n 1 file changed, 5 insertions(+)";
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,8 @@\n fn main() {\n-    println!(\"hello\");\n+    let x: i32 = \"not a number\";\n+    println!(\"{}\", x);\n }";

    let notes = format!(
        "{}\nFAILED_DIFF_STAT:\n{}\nFAILED_DIFF:\n{}",
        error, diff_stat, diff
    );

    conn.execute(
        "INSERT INTO iterations (task_id, attempt_number, status, notes) VALUES (?1, 1, 'failed', ?2)",
        rusqlite::params![task_id, notes],
    )
    .unwrap();

    // Now simulate retry: context assembly should include the previous diff
    let task = make_test_task(task_id);

    // Test with gather_context (plain text)
    let context = gather_context(&conn, &task).unwrap();
    assert!(
        context.contains("PREVIOUS ATTEMPT (failed):"),
        "Retry context should include previous attempt header"
    );
    assert!(
        context.contains("error[E0308]: mismatched types"),
        "Retry context should include error message"
    );
    assert!(
        context.contains("not a number"),
        "Retry context should include diff content showing the bad code"
    );
    assert!(
        context.contains("DO NOT repeat this approach"),
        "Retry context should warn against repeating the approach"
    );

    // Test with gather_context_items (structured)
    let items = gather_context_items(&conn, &task).unwrap();
    let diff_item = items.iter().find(|i| i.label == "Previous Failed Attempt");
    assert!(
        diff_item.is_some(),
        "Should have Previous Failed Attempt context item"
    );
    let item = diff_item.unwrap();
    assert_eq!(
        item.priority, FAILED_DIFF_PRIORITY,
        "Priority should be FAILED_DIFF_PRIORITY (12)"
    );
    assert!(item.content.contains("PREVIOUS ATTEMPT (failed):"));
    assert!(item.content.contains("not a number"));

    // Verify it uses the MOST RECENT failed iteration
    // Add a second failed iteration with different content
    let notes2 =
        format!("second error\nFAILED_DIFF_STAT:\nsecond stat\nFAILED_DIFF:\nsecond diff content");
    conn.execute(
        "INSERT INTO iterations (task_id, attempt_number, status, notes) VALUES (?1, 2, 'failed', ?2)",
        rusqlite::params![task_id, notes2],
    )
    .unwrap();

    let context2 = gather_context(&conn, &task).unwrap();
    assert!(
        context2.contains("second diff content"),
        "Should use most recent failed iteration's diff"
    );
    assert!(
        !context2.contains("not a number"),
        "Should NOT include older failed iteration's diff"
    );
}
