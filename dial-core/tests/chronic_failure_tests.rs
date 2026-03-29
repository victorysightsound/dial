use dial_core::{Engine, Event, EventHandler};
use std::env;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// Mutex to serialize tests that change the global current directory.
static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Helper: create an Engine in a temp directory.
async fn setup_engine() -> (Engine, TempDir, std::path::PathBuf) {
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let engine = Engine::init("test", None, false).await.unwrap();
    (engine, tmp, original_dir)
}

/// Collects ChronicFailureDetected events
struct ChronicEventCollector {
    events: Mutex<Vec<(i64, i64, i64)>>, // (task_id, total_failures, total_attempts)
}

impl ChronicEventCollector {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl EventHandler for ChronicEventCollector {
    fn handle(&self, event: &Event) {
        if let Event::ChronicFailureDetected {
            task_id,
            total_failures,
            total_attempts,
        } = event
        {
            self.events
                .lock()
                .unwrap()
                .push((*task_id, *total_failures, *total_attempts));
        }
    }
}

#[tokio::test]
async fn test_chronic_failures_returns_empty_for_healthy_tasks() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    engine.task_add("healthy task", 5, None).await.unwrap();

    let results = engine.chronic_failures(10).await.unwrap();
    assert!(results.is_empty());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_chronic_failures_detects_high_failure_tasks() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let task_id = engine.task_add("chronic task", 5, None).await.unwrap();

    // Manually set high failure count via DB
    let conn = dial_core::get_db(Some("test")).unwrap();
    conn.execute(
        "UPDATE tasks SET total_failures = 15, total_attempts = 20 WHERE id = ?1",
        [task_id],
    )
    .unwrap();

    let results = engine.chronic_failures(10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].task_id, task_id);
    assert_eq!(results[0].total_failures, 15);
    assert_eq!(results[0].total_attempts, 20);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_chronic_failures_emits_events() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (mut engine, _tmp, original_dir) = setup_engine().await;

    let collector = Arc::new(ChronicEventCollector::new());
    engine.on_event(collector.clone());

    let task_id = engine.task_add("failing task", 5, None).await.unwrap();

    // Set failures above threshold
    let conn = dial_core::get_db(Some("test")).unwrap();
    conn.execute(
        "UPDATE tasks SET total_failures = 12, total_attempts = 18 WHERE id = ?1",
        [task_id],
    )
    .unwrap();

    let results = engine.chronic_failures(10).await.unwrap();
    assert_eq!(results.len(), 1);

    let events = collector.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, task_id);
    assert_eq!(events[0].1, 12);
    assert_eq!(events[0].2, 18);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_create_iteration_increments_total_attempts() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let task_id = engine
        .task_add("attempt counter task", 5, None)
        .await
        .unwrap();

    let conn = dial_core::get_db(Some("test")).unwrap();

    // Initial total_attempts should be 0
    let initial: i64 = conn
        .query_row(
            "SELECT COALESCE(total_attempts, 0) FROM tasks WHERE id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(initial, 0);

    // Create an iteration
    dial_core::iteration::create_iteration(&conn, task_id, 1).unwrap();

    let after: i64 = conn
        .query_row(
            "SELECT total_attempts FROM tasks WHERE id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, 1);

    // Create another iteration
    dial_core::iteration::create_iteration(&conn, task_id, 2).unwrap();

    let after2: i64 = conn
        .query_row(
            "SELECT total_attempts FROM tasks WHERE id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after2, 2);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_complete_iteration_failed_increments_total_failures() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let task_id = engine
        .task_add("failure counter task", 5, None)
        .await
        .unwrap();

    let conn = dial_core::get_db(Some("test")).unwrap();

    // Create an iteration
    let iteration_id = dial_core::iteration::create_iteration(&conn, task_id, 1).unwrap();

    // Complete it as failed
    dial_core::iteration::complete_iteration(
        &conn,
        iteration_id,
        "failed",
        None,
        Some("test failure"),
    )
    .unwrap();

    let failures: i64 = conn
        .query_row(
            "SELECT total_failures FROM tasks WHERE id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(failures, 1);

    let last_failure: Option<String> = conn
        .query_row(
            "SELECT last_failure_at FROM tasks WHERE id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(last_failure.is_some());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_complete_iteration_success_does_not_increment_failures() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let task_id = engine.task_add("success task", 5, None).await.unwrap();

    let conn = dial_core::get_db(Some("test")).unwrap();

    // Create and complete an iteration successfully
    let iteration_id = dial_core::iteration::create_iteration(&conn, task_id, 1).unwrap();
    dial_core::iteration::complete_iteration(&conn, iteration_id, "completed", None, None).unwrap();

    let failures: i64 = conn
        .query_row(
            "SELECT COALESCE(total_failures, 0) FROM tasks WHERE id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(failures, 0);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_auto_block_chronic_failure_integration() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let task_id = engine
        .task_add("chronically failing task", 5, None)
        .await
        .unwrap();

    let conn = dial_core::get_db(Some("test")).unwrap();

    // Simulate many failures over multiple iterations
    for attempt in 1..=12 {
        let iter_id = dial_core::iteration::create_iteration(&conn, task_id, attempt).unwrap();
        dial_core::iteration::complete_iteration(
            &conn,
            iter_id,
            "failed",
            None,
            Some("test error"),
        )
        .unwrap();
        // Reset task to pending so it can be picked up again
        conn.execute(
            "UPDATE tasks SET status = 'pending' WHERE id = ?1",
            [task_id],
        )
        .unwrap();
    }

    // Verify counters
    let (total_attempts, total_failures): (i64, i64) = conn
        .query_row(
            "SELECT total_attempts, total_failures FROM tasks WHERE id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(total_attempts, 12);
    assert_eq!(total_failures, 12);

    // Set config threshold to 10
    engine.config_set("max_total_failures", "10").await.unwrap();

    // The chronic_failures method should detect this task
    let chronic = engine.chronic_failures(10).await.unwrap();
    assert_eq!(chronic.len(), 1);
    assert_eq!(chronic[0].task_id, task_id);
    assert_eq!(chronic[0].total_failures, 12);

    env::set_current_dir(original_dir).unwrap();
}
