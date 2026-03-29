use std::env;
use std::fs;
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

use dial_core::git::{
    checkpoint_create, checkpoint_drop, checkpoint_restore, checkpoints_enabled, git_has_changes,
    git_is_repo,
};
use dial_core::Engine;

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

// ---- Unit tests for checkpoint_create ----

#[test]
fn test_checkpoint_create_stashes_dirty_tree() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (tmp, original_dir) = setup_git_repo();

    // Create a dirty file
    fs::write(tmp.path().join("dirty.txt"), "some changes\n").unwrap();
    assert!(git_has_changes());

    // Create checkpoint
    let result = checkpoint_create("test-1");
    assert!(result.is_ok());
    assert!(result.unwrap(), "Should return true when stash is created");

    // Working tree should be clean after stash
    assert!(
        !git_has_changes(),
        "Working tree should be clean after checkpoint"
    );

    // Verify the stash exists with the expected message
    let stash_list = Command::new("git")
        .args(["stash", "list"])
        .output()
        .unwrap();
    let list_str = String::from_utf8_lossy(&stash_list.stdout);
    assert!(
        list_str.contains("dial-checkpoint-test-1"),
        "Stash message should contain the checkpoint id"
    );

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_checkpoint_create_noop_on_clean_tree() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_tmp, original_dir) = setup_git_repo();

    // No changes, should be a no-op
    assert!(!git_has_changes());

    let result = checkpoint_create("test-clean");
    assert!(result.is_ok());
    assert!(!result.unwrap(), "Should return false when tree is clean");

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_checkpoint_create_includes_untracked_files() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (tmp, original_dir) = setup_git_repo();

    // Create an untracked file (never staged)
    fs::write(tmp.path().join("new_file.rs"), "fn main() {}\n").unwrap();
    assert!(git_has_changes());

    let result = checkpoint_create("test-untracked");
    assert!(result.is_ok());
    assert!(result.unwrap());

    // The untracked file should be gone after stash (because -u flag)
    assert!(
        !tmp.path().join("new_file.rs").exists(),
        "Untracked file should be stashed away"
    );

    env::set_current_dir(original_dir).unwrap();
}

// ---- Unit tests for checkpoint_restore ----

#[test]
fn test_checkpoint_restore_pops_stash() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (tmp, original_dir) = setup_git_repo();

    // Create a dirty file and checkpoint it
    fs::write(tmp.path().join("work.txt"), "in-progress work\n").unwrap();
    checkpoint_create("test-restore").unwrap();
    assert!(!git_has_changes());

    // Restore the checkpoint
    let result = checkpoint_restore();
    assert!(result.is_ok());
    assert!(result.unwrap(), "Should return true when stash is popped");

    // The file should be back
    assert!(
        tmp.path().join("work.txt").exists(),
        "Stashed file should be restored"
    );
    let content = fs::read_to_string(tmp.path().join("work.txt")).unwrap();
    assert_eq!(content, "in-progress work\n");

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_checkpoint_restore_noop_no_stash() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_tmp, original_dir) = setup_git_repo();

    // No stash exists
    let result = checkpoint_restore();
    assert!(result.is_ok());
    assert!(!result.unwrap(), "Should return false when no stash exists");

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_checkpoint_restore_cleans_current_changes_first() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (tmp, original_dir) = setup_git_repo();

    // Create original work and checkpoint
    fs::write(tmp.path().join("code.rs"), "original\n").unwrap();
    checkpoint_create("test-overwrite").unwrap();

    // Now make different changes (simulating failed agent work)
    fs::write(tmp.path().join("code.rs"), "bad changes\n").unwrap();
    fs::write(tmp.path().join("extra.txt"), "extra file\n").unwrap();

    // Restore should clean current changes and pop stash
    let result = checkpoint_restore();
    assert!(result.is_ok());
    assert!(result.unwrap());

    // Should have the original content
    let content = fs::read_to_string(tmp.path().join("code.rs")).unwrap();
    assert_eq!(content, "original\n");

    env::set_current_dir(original_dir).unwrap();
}

// ---- Unit tests for checkpoint_drop ----

#[test]
fn test_checkpoint_drop_removes_stash() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (tmp, original_dir) = setup_git_repo();

    // Create a stash
    fs::write(tmp.path().join("temp.txt"), "temporary\n").unwrap();
    checkpoint_create("test-drop").unwrap();

    // Drop the stash
    let result = checkpoint_drop();
    assert!(result.is_ok());
    assert!(result.unwrap(), "Should return true when stash is dropped");

    // Verify stash list is empty
    let stash_list = Command::new("git")
        .args(["stash", "list"])
        .output()
        .unwrap();
    let list_str = String::from_utf8_lossy(&stash_list.stdout);
    assert!(
        list_str.trim().is_empty(),
        "Stash list should be empty after drop"
    );

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_checkpoint_drop_noop_no_stash() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_tmp, original_dir) = setup_git_repo();

    let result = checkpoint_drop();
    assert!(result.is_ok());
    assert!(!result.unwrap(), "Should return false when no stash exists");

    env::set_current_dir(original_dir).unwrap();
}

// ---- Unit tests for checkpoints_enabled ----

#[test]
fn test_checkpoints_enabled_defaults_true() {
    // When not in a DIAL project (no config), defaults to true
    let _lock = CWD_LOCK.lock().unwrap();
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    assert!(
        checkpoints_enabled(),
        "Checkpoints should be enabled by default"
    );

    env::set_current_dir(original_dir).unwrap();
}

// ---- Full cycle: create → restore ----

#[test]
fn test_checkpoint_full_cycle_create_restore() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (tmp, original_dir) = setup_git_repo();

    // Simulate pre-task state: one tracked file
    fs::write(tmp.path().join("main.rs"), "fn main() {}\n").unwrap();
    Command::new("git").args(["add", "-A"]).output().unwrap();
    Command::new("git")
        .args(["commit", "-m", "add main.rs"])
        .output()
        .unwrap();

    // Agent makes changes
    fs::write(
        tmp.path().join("main.rs"),
        "fn main() { println!(\"hello\"); }\n",
    )
    .unwrap();
    fs::write(tmp.path().join("lib.rs"), "pub fn helper() {}\n").unwrap();

    // Create checkpoint of agent's work
    assert!(checkpoint_create("cycle-1").unwrap());
    assert!(!git_has_changes());

    // Simulate new (bad) agent attempt
    fs::write(tmp.path().join("main.rs"), "fn main() { panic!(); }\n").unwrap();
    assert!(git_has_changes());

    // Restore checkpoint (rolls back bad changes, restores original agent work)
    assert!(checkpoint_restore().unwrap());

    // Verify the original agent work is back
    let main_content = fs::read_to_string(tmp.path().join("main.rs")).unwrap();
    assert_eq!(main_content, "fn main() { println!(\"hello\"); }\n");
    assert!(tmp.path().join("lib.rs").exists());

    env::set_current_dir(original_dir).unwrap();
}

// ---- Full cycle: create → drop (success path) ----

#[test]
fn test_checkpoint_full_cycle_create_drop() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (tmp, original_dir) = setup_git_repo();

    // Create some work
    fs::write(tmp.path().join("feature.rs"), "pub fn feature() {}\n").unwrap();

    // Checkpoint before agent work
    assert!(checkpoint_create("cycle-success").unwrap());

    // Agent succeeds, validation passes — drop the checkpoint
    assert!(checkpoint_drop().unwrap());

    // Stash should be empty now
    let stash_list = Command::new("git")
        .args(["stash", "list"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&stash_list.stdout)
        .trim()
        .is_empty());

    env::set_current_dir(original_dir).unwrap();
}

// ---- Not-a-git-repo error cases ----

#[test]
fn test_checkpoint_create_errors_outside_repo() {
    let _lock = CWD_LOCK.lock().unwrap();
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    assert!(!git_is_repo());
    let result = checkpoint_create("no-repo");
    assert!(result.is_err());

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_checkpoint_restore_errors_outside_repo() {
    let _lock = CWD_LOCK.lock().unwrap();
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    assert!(!git_is_repo());
    let result = checkpoint_restore();
    assert!(result.is_err());

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_checkpoint_drop_errors_outside_repo() {
    let _lock = CWD_LOCK.lock().unwrap();
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    assert!(!git_is_repo());
    let result = checkpoint_drop();
    assert!(result.is_err());

    env::set_current_dir(original_dir).unwrap();
}

// ---- Integration test: iterate → fail → checkpoint restore → retry ----

/// This test simulates the full DIAL cycle:
/// 1. Initialize a DIAL project in a git repo
/// 2. Add a task and start iteration (checkpoint created)
/// 3. Agent writes code, validation fails
/// 4. Checkpoint is restored (agent's bad changes rolled back)
/// 5. Retry: agent writes different code
///
/// We test the checkpoint primitives in the same sequence the iteration/mod.rs
/// flow uses them, since the full iterate()/validate() functions require
/// build/test commands and produce side effects difficult to mock.
#[tokio::test]
async fn test_integration_iterate_fail_restore_retry_cycle() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (tmp, original_dir) = setup_git_repo();

    // Initialize DIAL in this git repo
    let engine = Engine::init("test", None, false).await.unwrap();

    // Verify enable_checkpoints defaults to true
    let cp_config = engine.config_get("enable_checkpoints").await.unwrap();
    assert_eq!(cp_config, Some("true".to_string()));

    // Add a task
    let task_id = engine
        .task_add("Implement feature X", 5, None)
        .await
        .unwrap();
    assert!(task_id > 0);

    // Commit the DIAL init files so the tree is clean
    Command::new("git").args(["add", "-A"]).output().unwrap();
    Command::new("git")
        .args(["commit", "-m", "dial init"])
        .output()
        .unwrap();

    // --- Simulate iterate_once() checkpoint creation ---
    // The working tree is clean after committing DIAL init files
    let created = checkpoint_create(&format!("{}", task_id)).unwrap();
    assert!(!created, "Clean tree should not create a stash");

    // --- Agent attempt 1: writes some code ---
    fs::write(
        tmp.path().join("feature_x.rs"),
        "fn feature_x() { panic!(\"broken\"); }\n",
    )
    .unwrap();
    fs::write(tmp.path().join("tests.rs"), "// broken tests\n").unwrap();

    // Now there are changes — checkpoint them before "validation"
    let created = checkpoint_create(&format!("{}-attempt1", task_id)).unwrap();
    assert!(created, "Dirty tree should create a stash");
    assert!(!git_has_changes(), "Tree should be clean after checkpoint");

    // Simulate the agent making more (bad) changes after checkpoint
    fs::write(
        tmp.path().join("feature_x.rs"),
        "fn feature_x() { /* even worse */ }\n",
    )
    .unwrap();
    fs::write(tmp.path().join("garbage.txt"), "junk file\n").unwrap();

    // --- Validation fails → restore checkpoint ---
    let restored = checkpoint_restore().unwrap();
    assert!(restored, "Checkpoint should be restored");

    // Verify: the pre-checkpoint state (attempt 1's work) is restored
    let content = fs::read_to_string(tmp.path().join("feature_x.rs")).unwrap();
    assert_eq!(
        content, "fn feature_x() { panic!(\"broken\"); }\n",
        "Should have attempt 1's original code, not the garbage"
    );
    assert!(
        tmp.path().join("tests.rs").exists(),
        "tests.rs from attempt 1 should exist"
    );
    // garbage.txt was created after the checkpoint, so it should be gone
    // (it was cleaned before the pop)

    // --- Agent attempt 2: writes better code ---
    fs::write(
        tmp.path().join("feature_x.rs"),
        "fn feature_x() -> i32 { 42 }\n",
    )
    .unwrap();
    fs::write(
        tmp.path().join("tests.rs"),
        "#[test] fn test_x() { assert_eq!(feature_x(), 42); }\n",
    )
    .unwrap();

    // Checkpoint before validation attempt 2
    let created = checkpoint_create(&format!("{}-attempt2", task_id)).unwrap();
    assert!(created);

    // --- Validation succeeds → drop checkpoint ---
    // Simulate: agent's code passes build+test
    // First, pop the stash to get the code back (simulating what happens between checkpoint and commit)
    // In real flow, the code is already in working tree when validate runs.
    // But since we stashed it, let's restore it.
    let restored = checkpoint_restore().unwrap();
    assert!(restored);

    // Now commit (simulating successful validation path)
    Command::new("git").args(["add", "-A"]).output().unwrap();
    Command::new("git")
        .args(["commit", "-m", "Implement feature X"])
        .output()
        .unwrap();

    // Drop any remaining checkpoint
    let dropped = checkpoint_drop().unwrap();
    assert!(!dropped, "No stash should remain after pop + commit");

    // Verify final state
    let content = fs::read_to_string(tmp.path().join("feature_x.rs")).unwrap();
    assert_eq!(content, "fn feature_x() -> i32 { 42 }\n");
    assert!(!git_has_changes(), "Should be clean after commit");

    // Verify the commit exists
    let log = Command::new("git")
        .args(["log", "--oneline", "-1"])
        .output()
        .unwrap();
    let log_str = String::from_utf8_lossy(&log.stdout);
    assert!(log_str.contains("Implement feature X"));

    env::set_current_dir(original_dir).unwrap();
}
