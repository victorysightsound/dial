use dial_core::Engine;
use std::env;
use std::sync::Mutex;
use tempfile::TempDir;

static CWD_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

async fn setup_engine() -> (Engine, TempDir, std::path::PathBuf) {
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let engine = Engine::init("test", None, false).await.unwrap();
    (engine, tmp, original_dir)
}

#[tokio::test]
async fn test_similar_completed_tasks_integration() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let conn = dial_core::get_db(Some("test")).unwrap();

    // Create a completed task — shares "user authentication" with the search query
    conn.execute(
        "INSERT INTO tasks (id, description, priority, status, completed_at)
         VALUES (1, 'implement user authentication', 5, 'completed', '2025-01-01T00:00:00Z')",
        [],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO iterations (task_id, status, notes, commit_hash, ended_at)
         VALUES (1, 'completed', 'Added JWT middleware and login endpoint', 'abc123def', '2025-01-01T00:00:00Z')",
        [],
    ).unwrap();

    // Create a pending task with same keywords (should NOT appear)
    conn.execute(
        "INSERT INTO tasks (id, description, priority, status)
         VALUES (2, 'implement user authentication roles', 5, 'pending')",
        [],
    )
    .unwrap();

    // Search for similar tasks — FTS5 implicit AND requires all terms present
    let results =
        dial_core::task::find_similar_completed_tasks(&conn, "user authentication", 3).unwrap();

    assert_eq!(results.len(), 1, "Should find exactly one completed task");
    assert_eq!(results[0].0.id, 1);
    assert!(results[0].1.contains("Added JWT middleware"));
    assert!(results[0].1.contains("abc123def"));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_similar_tasks_context_assembly() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let conn = dial_core::get_db(Some("test")).unwrap();

    // Completed task with known terms
    conn.execute(
        "INSERT INTO tasks (id, description, priority, status, completed_at)
         VALUES (1, 'build REST API endpoint', 5, 'completed', '2025-01-01T00:00:00Z')",
        [],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO iterations (task_id, status, notes, commit_hash, ended_at)
         VALUES (1, 'completed', 'Used actix-web with JSON serialization', 'commit789', '2025-01-01T00:00:00Z')",
        [],
    ).unwrap();

    // New task whose description (after stop-word stripping) matches via FTS AND
    // "build REST API endpoint" matches "build REST API endpoint" exactly
    let task = dial_core::task::models::Task {
        id: 99,
        description: "build REST API endpoint".to_string(),
        status: dial_core::task::models::TaskStatus::Pending,
        priority: 5,
        blocked_by: None,
        spec_section_id: None,
        prd_section_id: None,
        created_at: "2025-01-03T00:00:00Z".to_string(),
        started_at: None,
        completed_at: None,
        total_attempts: 0,
        total_failures: 0,
        last_failure_at: None,
        acceptance_criteria: Vec::new(),
        requires_browser_verification: false,
    };

    // Gather context — should include similar completed task
    let context =
        dial_core::iteration::context::gather_context_without_signs(&conn, &task).unwrap();

    assert!(
        context.contains("Similar Completed Tasks"),
        "Context should include similar completed tasks section. Got:\n{}",
        context
    );
    assert!(
        context.contains("SIMILAR COMPLETED TASK:"),
        "Context should contain formatted similar task entries"
    );
    assert!(
        context.contains("commit789"),
        "Context should contain commit hash from iteration"
    );

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_similar_tasks_budgeted_context() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let conn = dial_core::get_db(Some("test")).unwrap();

    // Completed task
    conn.execute(
        "INSERT INTO tasks (id, description, priority, status, completed_at)
         VALUES (1, 'setup CI pipeline', 5, 'completed', '2025-01-01T00:00:00Z')",
        [],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO iterations (task_id, status, notes, commit_hash, ended_at)
         VALUES (1, 'completed', 'Configured GitHub Actions with caching', 'ci_hash', '2025-01-01T00:00:00Z')",
        [],
    ).unwrap();

    // New task matching the same terms
    let task = dial_core::task::models::Task {
        id: 99,
        description: "setup CI pipeline".to_string(),
        status: dial_core::task::models::TaskStatus::Pending,
        priority: 5,
        blocked_by: None,
        spec_section_id: None,
        prd_section_id: None,
        created_at: "2025-01-03T00:00:00Z".to_string(),
        started_at: None,
        completed_at: None,
        total_attempts: 0,
        total_failures: 0,
        last_failure_at: None,
        acceptance_criteria: Vec::new(),
        requires_browser_verification: false,
    };

    let items = dial_core::iteration::context::gather_context_items(&conn, &task).unwrap();
    let has_similar = items
        .iter()
        .any(|item| item.label == "Similar Completed Tasks");
    assert!(
        has_similar,
        "Context items should include Similar Completed Tasks"
    );

    // Verify priority is 25
    let similar_item = items
        .iter()
        .find(|item| item.label == "Similar Completed Tasks")
        .unwrap();
    assert_eq!(
        similar_item.priority, 25,
        "Similar tasks should have priority 25"
    );

    env::set_current_dir(original_dir).unwrap();
}
