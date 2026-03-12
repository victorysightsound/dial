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
async fn test_health_empty_project() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let health = _engine.health().await.unwrap();
    assert_eq!(health.score, 50);
    assert_eq!(health.trend, dial_core::Trend::Stable);
    assert_eq!(health.factors.len(), 6);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_health_full_cycle() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_engine, tmp, original_dir) = setup_engine().await;

    let config = EngineConfig {
        work_dir: tmp.path().to_path_buf(),
        phase: Some("test".to_string()),
        ..Default::default()
    };
    let engine = Engine::open(config).await.unwrap();

    // Seed a task
    engine.task_add("health test task", 1, None).await.unwrap();

    let conn = rusqlite::Connection::open(tmp.path().join(".dial/test.db")).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;").unwrap();

    // 15 completed + 5 failed iterations = 75% success rate
    for i in 1..=15 {
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, status) VALUES (1, ?1, 'completed')",
            [i],
        )
        .unwrap();
    }
    for i in 16..=20 {
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, status) VALUES (1, ?1, 'failed')",
            [i],
        )
        .unwrap();
    }

    // A failure pattern with solution
    conn.execute(
        "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('HealthTestErr', 'Health test error', 'test')",
        [],
    )
    .unwrap();
    let pattern_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'fix it', 0.8)",
        [pattern_id],
    )
    .unwrap();

    // One resolved, one unresolved failure
    conn.execute(
        "INSERT INTO failures (iteration_id, pattern_id, error_text, resolved) VALUES (1, ?1, 'err1', 1)",
        [pattern_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO failures (iteration_id, pattern_id, error_text, resolved) VALUES (2, ?1, 'err2', 0)",
        [pattern_id],
    )
    .unwrap();

    // A referenced learning
    conn.execute(
        "INSERT INTO learnings (category, description, times_referenced) VALUES ('pattern', 'learned something', 3)",
        [],
    )
    .unwrap();
    // An unreferenced learning
    conn.execute(
        "INSERT INTO learnings (category, description, times_referenced) VALUES ('build', 'also learned', 0)",
        [],
    )
    .unwrap();

    drop(conn);

    let health = engine.health().await.unwrap();

    // Verify score is reasonable for a mixed project
    assert!(health.score > 30, "Score should be > 30 for a mixed project, got {}", health.score);
    assert!(health.score < 90, "Score should be < 90 for a mixed project, got {}", health.score);
    assert_eq!(health.factors.len(), 6);

    // Verify individual factor names exist
    let factor_names: Vec<&str> = health.factors.iter().map(|f| f.name.as_str()).collect();
    assert!(factor_names.contains(&"success_rate"));
    assert!(factor_names.contains(&"success_trend"));
    assert!(factor_names.contains(&"solution_confidence"));
    assert!(factor_names.contains(&"blocked_task_ratio"));
    assert!(factor_names.contains(&"learning_utilization"));
    assert!(factor_names.contains(&"pattern_resolution_rate"));

    // success_rate should be 75
    let sr = health.factors.iter().find(|f| f.name == "success_rate").unwrap();
    assert_eq!(sr.score, 75);

    // pattern_resolution_rate: 1/2 = 50
    let pr = health.factors.iter().find(|f| f.name == "pattern_resolution_rate").unwrap();
    assert_eq!(pr.score, 50);

    // learning_utilization: 1/2 = 50
    let lu = health.factors.iter().find(|f| f.name == "learning_utilization").unwrap();
    assert_eq!(lu.score, 50);

    // solution_confidence: 0.8 -> 80
    let sc = health.factors.iter().find(|f| f.name == "solution_confidence").unwrap();
    assert_eq!(sc.score, 80);

    // Verify JSON serialization works
    let json = serde_json::to_string_pretty(&health).unwrap();
    assert!(json.contains("\"score\""));
    assert!(json.contains("\"trend\""));
    assert!(json.contains("\"factors\""));

    env::set_current_dir(original_dir).unwrap();
}
