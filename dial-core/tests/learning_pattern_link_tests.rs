use dial_core::{Engine, EngineConfig};
use std::env;
use std::sync::Mutex;
use tempfile::TempDir;

static CWD_LOCK: Mutex<()> = Mutex::new(());

async fn setup_engine() -> (Engine, TempDir, std::path::PathBuf) {
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let engine = Engine::init("test", None, false).await.unwrap();
    (engine, tmp, original_dir)
}

/// Full integration test: create task, record failure, learn linked to pattern,
/// query learnings for pattern, and verify context includes pattern-linked learnings.
#[tokio::test]
async fn test_learning_pattern_link_full_cycle() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_engine, tmp, original_dir) = setup_engine().await;

    // Open a fresh engine to use the DB
    let config = EngineConfig {
        work_dir: tmp.path().to_path_buf(),
        phase: Some("test".to_string()),
        ..Default::default()
    };
    let engine = Engine::open(config).await.unwrap();

    // 1. Add a task
    let task_id = engine.task_add("integration test task", 1, None).await.unwrap();

    // 2. Seed iteration, pattern, and failure via direct DB access
    let conn = rusqlite::Connection::open(tmp.path().join(".dial/test.db")).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;").unwrap();

    conn.execute(
        "INSERT INTO iterations (task_id, attempt_number, duration_seconds, status) VALUES (?1, 1, 30.0, 'failed')",
        [task_id],
    )
    .unwrap();
    let iteration_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('IntegLinkErr', 'Integration link error', 'build')",
        [],
    )
    .unwrap();
    let pattern_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'error: unresolved import')",
        rusqlite::params![iteration_id, pattern_id],
    )
    .unwrap();

    drop(conn);

    // 3. Add a learning linked to the pattern and iteration
    let learning_id = engine
        .learn_linked(
            "Always check import paths after refactoring",
            Some("gotcha"),
            Some(pattern_id),
            Some(iteration_id),
        )
        .await
        .unwrap();
    assert!(learning_id > 0);

    // 4. Query learnings for this pattern
    let learnings = engine.learnings_for_pattern(pattern_id).await.unwrap();
    assert_eq!(learnings.len(), 1);
    assert_eq!(learnings[0].description, "Always check import paths after refactoring");
    assert_eq!(learnings[0].category.as_deref(), Some("gotcha"));

    // 5. Verify the learning is stored with correct links in the DB
    let conn = rusqlite::Connection::open(tmp.path().join(".dial/test.db")).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;").unwrap();

    let (stored_pid, stored_iid): (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT pattern_id, iteration_id FROM learnings WHERE id = ?1",
            [learning_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_pid, Some(pattern_id));
    assert_eq!(stored_iid, Some(iteration_id));

    // 6. Verify context assembly includes pattern-linked learnings
    let task = engine.task_get(task_id).await.unwrap();
    let (context, _excluded) = engine.gather_context_budgeted(&task, 50000).await.unwrap();

    assert!(
        context.contains("LEARNING (from pattern: IntegLinkErr): Always check import paths after refactoring"),
        "Budgeted context should contain pattern-linked learning. Got:\n{}",
        context
    );

    // 7. Verify that adding an unlinked learning does NOT appear in pattern queries
    let _unlinked_id = engine
        .learn("General unlinked learning", Some("other"))
        .await
        .unwrap();
    let learnings_after = engine.learnings_for_pattern(pattern_id).await.unwrap();
    assert_eq!(learnings_after.len(), 1, "Unlinked learning should not appear in pattern query");

    drop(conn);
    env::set_current_dir(original_dir).unwrap();
}

/// Test that learn() without links still works (backward compatibility).
#[tokio::test]
async fn test_learn_without_links_backward_compat() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_engine, tmp, original_dir) = setup_engine().await;

    let config = EngineConfig {
        work_dir: tmp.path().to_path_buf(),
        phase: Some("test".to_string()),
        ..Default::default()
    };
    let engine = Engine::open(config).await.unwrap();

    // Use the original learn() method — should still work
    let id = engine.learn("simple learning", Some("build")).await.unwrap();
    assert!(id > 0);

    // Verify no pattern/iteration links
    let conn = rusqlite::Connection::open(tmp.path().join(".dial/test.db")).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;").unwrap();

    let (pid, iid): (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT pattern_id, iteration_id FROM learnings WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(pid.is_none());
    assert!(iid.is_none());

    drop(conn);
    env::set_current_dir(original_dir).unwrap();
}
