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

#[tokio::test]
async fn test_pattern_metrics_empty() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let metrics = _engine.pattern_metrics().await.unwrap();
    assert!(metrics.is_empty());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_pattern_metrics_full_cycle() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_engine, tmp, original_dir) = setup_engine().await;

    // Open a direct DB connection for seeding test data
    let config = EngineConfig {
        work_dir: tmp.path().to_path_buf(),
        phase: Some("test".to_string()),
        ..Default::default()
    };
    let engine = Engine::open(config).await.unwrap();

    // Seed: add a task
    engine
        .task_add("integration test task", 1, None)
        .await
        .unwrap();

    // Use the DB directly to seed iteration, pattern, failure, and provider_usage
    let conn = rusqlite::Connection::open(tmp.path().join(".dial/test.db")).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .unwrap();

    // Create iteration
    conn.execute(
        "INSERT INTO iterations (task_id, attempt_number, duration_seconds, status) VALUES (1, 1, 90.0, 'failed')",
        [],
    )
    .unwrap();
    let iter_id = conn.last_insert_rowid();

    // Create failure pattern (unique key to avoid collision with seeded patterns)
    conn.execute(
        "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('IntegTestError', 'Integration test error', 'build')",
        [],
    )
    .unwrap();
    let pattern_id = conn.last_insert_rowid();

    // Create failure (unresolved)
    conn.execute(
        "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'error[E0308]: mismatched types')",
        rusqlite::params![iter_id, pattern_id],
    )
    .unwrap();

    // Record provider usage
    conn.execute(
        "INSERT INTO provider_usage (iteration_id, provider, model, tokens_in, tokens_out, cost_usd) VALUES (?1, 'anthropic', 'claude-opus-4-6', 2000, 800, 0.12)",
        [iter_id],
    )
    .unwrap();

    // Create a second iteration with the same pattern, auto-resolved
    conn.execute(
        "INSERT INTO iterations (task_id, attempt_number, duration_seconds, status) VALUES (1, 2, 45.0, 'completed')",
        [],
    )
    .unwrap();
    let iter_id2 = conn.last_insert_rowid();

    // Create a solution
    conn.execute(
        "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'Fix type mismatch', 0.8)",
        [pattern_id],
    )
    .unwrap();
    let solution_id = conn.last_insert_rowid();

    // Auto-resolved failure
    conn.execute(
        "INSERT INTO failures (iteration_id, pattern_id, error_text, resolved, resolved_by_solution_id) VALUES (?1, ?2, 'error[E0308]: wrong type', 1, ?3)",
        rusqlite::params![iter_id2, pattern_id, solution_id],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO provider_usage (iteration_id, provider, model, tokens_in, tokens_out, cost_usd) VALUES (?1, 'anthropic', 'claude-opus-4-6', 1500, 600, 0.08)",
        [iter_id2],
    )
    .unwrap();

    drop(conn);

    // Now test the engine method
    let metrics = engine.pattern_metrics().await.unwrap();
    assert_eq!(metrics.len(), 1);

    let m = &metrics[0];
    assert_eq!(m.pattern_key, "IntegTestError");
    assert_eq!(m.category, "build");
    assert_eq!(m.total_occurrences, 2);
    assert!((m.total_resolution_time_secs - 135.0).abs() < 0.1);
    assert!((m.avg_resolution_time_secs - 67.5).abs() < 0.1);
    assert_eq!(m.total_tokens_consumed, 4900); // (2000+800) + (1500+600)
    assert!((m.total_cost_usd - 0.20).abs() < 0.01);
    assert_eq!(m.auto_resolved_count, 1);
    assert_eq!(m.manual_resolved_count, 0);
    assert_eq!(m.unresolved_count, 1);

    env::set_current_dir(original_dir).unwrap();
}
