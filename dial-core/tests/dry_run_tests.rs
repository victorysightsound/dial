use dial_core::Engine;
use std::env;
use std::sync::Mutex;
use tempfile::TempDir;

// Mutex to serialize tests that change the global current directory.
static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Helper: create an Engine in a temp directory with a task added.
/// Returns the Engine, TempDir, and the original working directory to restore.
async fn setup_engine_with_task() -> (Engine, TempDir, std::path::PathBuf) {
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let engine = Engine::init("test", None, false).await.unwrap();
    engine.task_add("Implement dry-run feature", 5, None).await.unwrap();
    (engine, tmp, original_dir)
}

#[tokio::test]
async fn test_dry_run_returns_correct_task() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine_with_task().await;

    let result = engine.iterate_dry_run().await.unwrap();
    assert_eq!(result.task.description, "Implement dry-run feature");
    assert!(result.dependencies_satisfied);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_no_iteration_records_created() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine_with_task().await;

    // Run dry-run
    let _result = engine.iterate_dry_run().await.unwrap();

    // Verify no iteration records were created
    let conn = dial_core::get_db(Some("test")).unwrap();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM iterations",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0, "Dry run should not create iteration records");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_task_status_unchanged() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine_with_task().await;

    // Run dry-run
    let result = engine.iterate_dry_run().await.unwrap();
    let task_id = result.task.id;

    // Verify task status is still "pending"
    let task = engine.task_get(task_id).await.unwrap();
    assert_eq!(task.status.to_string(), "pending",
        "Dry run should not change task status");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_no_learning_reference_increments() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine_with_task().await;

    // Add a learning
    let learning_id = engine.learn("Test learning for dry run", Some("test")).await.unwrap();

    // Check initial reference count
    let conn = dial_core::get_db(Some("test")).unwrap();
    let initial_refs: i64 = conn.query_row(
        "SELECT times_referenced FROM learnings WHERE id = ?1",
        [learning_id],
        |row| row.get(0),
    ).unwrap();

    // Run dry-run
    let _result = engine.iterate_dry_run().await.unwrap();

    // Verify reference count unchanged
    let after_refs: i64 = conn.query_row(
        "SELECT times_referenced FROM learnings WHERE id = ?1",
        [learning_id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(initial_refs, after_refs,
        "Dry run should not increment learning reference counts");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_has_prompt_preview() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine_with_task().await;

    let result = engine.iterate_dry_run().await.unwrap();

    assert!(!result.prompt_preview.is_empty(), "Prompt preview should not be empty");
    assert!(result.prompt_preview.contains("DIAL Sub-Agent Task"),
        "Prompt preview should contain sub-agent header");
    assert!(result.prompt_preview.contains("Implement dry-run feature"),
        "Prompt preview should contain task description");
    assert!(result.prompt_preview.len() <= 500,
        "Prompt preview should be at most 500 chars");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_includes_context_items() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine_with_task().await;

    let result = engine.iterate_dry_run().await.unwrap();

    // Should always have at least the "Signs" context item
    assert!(!result.context_items_included.is_empty(),
        "Should have at least Signs in included context items");

    let has_signs = result.context_items_included
        .iter()
        .any(|(label, _)| label.contains("Signs"));
    assert!(has_signs, "Should include Signs context item");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_token_budget_default() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine_with_task().await;

    let result = engine.iterate_dry_run().await.unwrap();

    // Default budget is 8000
    assert_eq!(result.token_budget, 8000, "Default token budget should be 8000");
    assert!(result.total_context_tokens <= result.token_budget,
        "Total context tokens should not exceed budget");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_serializes_to_json() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine_with_task().await;

    let result = engine.iterate_dry_run().await.unwrap();

    let json = serde_json::to_string_pretty(&result).unwrap();
    assert!(json.contains("\"task\""), "JSON should contain task field");
    assert!(json.contains("\"context_items_included\""), "JSON should contain context_items_included");
    assert!(json.contains("\"token_budget\""), "JSON should contain token_budget");
    assert!(json.contains("\"dependencies_satisfied\""), "JSON should contain dependencies_satisfied");
    assert!(json.contains("\"prompt_preview\""), "JSON should contain prompt_preview");

    // Verify it round-trips as valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_object(), "Should parse as JSON object");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_fails_with_no_tasks() {
    let _lock = CWD_LOCK.lock().unwrap();
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let engine = Engine::init("test", None, false).await.unwrap();
    // No tasks added

    let result = engine.iterate_dry_run().await;
    assert!(result.is_err(), "Should fail when no tasks are pending");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_respects_task_priority() {
    let _lock = CWD_LOCK.lock().unwrap();
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let engine = Engine::init("test", None, false).await.unwrap();
    engine.task_add("Low priority task", 10, None).await.unwrap();
    engine.task_add("High priority task", 1, None).await.unwrap();

    let result = engine.iterate_dry_run().await.unwrap();
    assert_eq!(result.task.description, "High priority task",
        "Should select highest priority task (lowest number)");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_skips_blocked_dependencies() {
    let _lock = CWD_LOCK.lock().unwrap();
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let engine = Engine::init("test", None, false).await.unwrap();
    let dep_id = engine.task_add("Dependency task", 1, None).await.unwrap();
    let task_id = engine.task_add("Dependent task", 1, None).await.unwrap();
    engine.task_depends(task_id, dep_id).await.unwrap();

    // Dry run should pick the dependency (dep_id), not the dependent task
    let result = engine.iterate_dry_run().await.unwrap();
    assert_eq!(result.task.id, dep_id,
        "Should pick the task without unsatisfied dependencies");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_dry_run_multiple_calls_no_accumulation() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine_with_task().await;

    // Add a learning to test reference counting
    let learning_id = engine.learn("Repeated dry run learning", Some("test")).await.unwrap();

    // Run dry-run multiple times
    let _r1 = engine.iterate_dry_run().await.unwrap();
    let _r2 = engine.iterate_dry_run().await.unwrap();
    let _r3 = engine.iterate_dry_run().await.unwrap();

    // Verify no side effects accumulated
    let conn = dial_core::get_db(Some("test")).unwrap();
    let refs: i64 = conn.query_row(
        "SELECT times_referenced FROM learnings WHERE id = ?1",
        [learning_id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(refs, 0, "Multiple dry runs should not accumulate side effects");

    let iter_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM iterations", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(iter_count, 0, "Multiple dry runs should create no iteration records");

    env::set_current_dir(original_dir).unwrap();
}
