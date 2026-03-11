use dial_core::{Engine, EngineConfig};
use dial_core::task::models::TaskStatus;
use std::env;
use std::sync::Mutex;
use tempfile::TempDir;

// Mutex to serialize tests that change the global current directory.
// Rust tests run in parallel by default, but set_current_dir is process-global.
static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Helper: create an Engine in a temp directory.
/// Returns the Engine, TempDir, and the original working directory to restore.
async fn setup_engine() -> (Engine, TempDir, std::path::PathBuf) {
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let engine = Engine::init("test", None, false).await.unwrap();
    (engine, tmp, original_dir)
}

#[tokio::test]
async fn test_engine_init_creates_dial_dir() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, tmp, original_dir) = setup_engine().await;

    assert!(tmp.path().join(".dial").exists());
    assert!(tmp.path().join(".dial/test.db").exists());
    assert!(engine.dial_dir().exists());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_engine_open_after_init() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (_engine, tmp, original_dir) = setup_engine().await;

    let config = EngineConfig {
        work_dir: tmp.path().to_path_buf(),
        phase: Some("test".to_string()),
    };
    let engine2 = Engine::open(config).await;
    assert!(engine2.is_ok());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_engine_open_fails_without_init() {
    let _lock = CWD_LOCK.lock().unwrap();
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();

    let config = EngineConfig {
        work_dir: tmp.path().to_path_buf(),
        phase: None,
    };
    let result = Engine::open(config).await;
    assert!(result.is_err());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_schema_version() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let version = engine.schema_version().await.unwrap();
    assert!(version > 0, "Schema version should be positive after migrations");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_config_get_set() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    engine.config_set("test_key", "test_value").await.unwrap();
    let value = engine.config_get("test_key").await.unwrap();
    assert_eq!(value, Some("test_value".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_config_get_missing_key() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let value = engine.config_get("nonexistent_key").await.unwrap();
    assert_eq!(value, None);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_config_set_overwrites() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    engine.config_set("key", "value1").await.unwrap();
    engine.config_set("key", "value2").await.unwrap();
    let value = engine.config_get("key").await.unwrap();
    assert_eq!(value, Some("value2".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_add_returns_id() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let id1 = engine.task_add("First task", 5, None).await.unwrap();
    let id2 = engine.task_add("Second task", 3, None).await.unwrap();
    assert!(id1 > 0);
    assert!(id2 > id1);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_get_by_id() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let id = engine.task_add("Test task", 5, None).await.unwrap();
    let task = engine.task_get(id).await.unwrap();
    assert_eq!(task.id, id);
    assert_eq!(task.description, "Test task");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_get_nonexistent_fails() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let result = engine.task_get(99999).await;
    assert!(result.is_err());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_done() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let id = engine.task_add("Task to complete", 5, None).await.unwrap();
    engine.task_done(id).await.unwrap();

    let task = engine.task_get(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Completed);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_block() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let id = engine.task_add("Task to block", 5, None).await.unwrap();
    engine.task_block(id, "waiting on dependency").await.unwrap();

    let task = engine.task_get(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Blocked);
    assert_eq!(task.blocked_by, Some("waiting on dependency".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_cancel() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let id = engine.task_add("Task to cancel", 5, None).await.unwrap();
    engine.task_cancel(id).await.unwrap();

    let task = engine.task_get(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Cancelled);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_next_returns_highest_priority() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    engine.task_add("Low priority", 10, None).await.unwrap();
    engine.task_add("High priority", 1, None).await.unwrap();
    engine.task_add("Medium priority", 5, None).await.unwrap();

    let next = engine.task_next().await.unwrap();
    assert!(next.is_some());
    assert_eq!(next.unwrap().description, "High priority");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_next_empty_queue() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let next = engine.task_next().await.unwrap();
    assert!(next.is_none());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_learn_and_search() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let id = engine.learn("Always validate inputs", Some("pattern")).await.unwrap();
    assert!(id > 0);

    let results = engine.learnings_search("validate inputs").await.unwrap();
    assert!(!results.is_empty());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_learn_delete() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let id = engine.learn("Temporary learning", Some("test")).await.unwrap();
    engine.learnings_delete(id).await.unwrap();

    let results = engine.learnings_search("temporary").await.unwrap();
    assert!(results.is_empty());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_migration_version_matches_latest() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let version = engine.schema_version().await.unwrap();
    let latest = dial_core::db::migrations::latest_version();
    assert_eq!(version, latest);

    env::set_current_dir(original_dir).unwrap();
}
