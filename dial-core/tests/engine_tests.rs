use dial_core::{Engine, EngineConfig, Event, EventHandler};
use dial_core::provider::{Provider, ProviderRequest, ProviderResponse, TokenUsage};
use dial_core::task::models::TaskStatus;
use async_trait::async_trait;
use std::env;
use std::sync::{Arc, Mutex};
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

// --- Dependency Graph Tests ---

#[tokio::test]
async fn test_task_dependency_basic() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let b = engine.task_add("Task B", 5, None).await.unwrap();

    engine.task_depends(b, a).await.unwrap();

    let deps = engine.task_get_dependencies(b).await.unwrap();
    assert_eq!(deps, vec![a]);

    let dependents = engine.task_get_dependents(a).await.unwrap();
    assert_eq!(dependents, vec![b]);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_self_dependency_rejected() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let result = engine.task_depends(a, a).await;
    assert!(result.is_err());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_cycle_rejected() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let b = engine.task_add("Task B", 5, None).await.unwrap();
    let c = engine.task_add("Task C", 5, None).await.unwrap();

    // A -> B -> C, then try C -> A (cycle)
    engine.task_depends(b, a).await.unwrap();
    engine.task_depends(c, b).await.unwrap();
    let result = engine.task_depends(a, c).await;
    assert!(result.is_err());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_next_respects_dependencies() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    // B (priority 1) depends on A (priority 5)
    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let b = engine.task_add("Task B", 1, None).await.unwrap();
    engine.task_depends(b, a).await.unwrap();

    // Even though B has higher priority, A should come first (B's deps not met)
    let next = engine.task_next().await.unwrap();
    assert!(next.is_some());
    assert_eq!(next.unwrap().id, a);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_next_after_dependency_completed() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let b = engine.task_add("Task B", 5, None).await.unwrap();
    engine.task_depends(b, a).await.unwrap();

    // Complete A
    engine.task_done(a).await.unwrap();

    // Now B should be available
    let next = engine.task_next().await.unwrap();
    assert!(next.is_some());
    assert_eq!(next.unwrap().id, b);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_task_deps_satisfied() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let b = engine.task_add("Task B", 5, None).await.unwrap();
    engine.task_depends(b, a).await.unwrap();

    assert!(!engine.task_deps_satisfied(b).await.unwrap());

    engine.task_done(a).await.unwrap();
    assert!(engine.task_deps_satisfied(b).await.unwrap());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_auto_unblock_on_dependency_completion() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let b = engine.task_add("Task B", 5, None).await.unwrap();
    engine.task_depends(b, a).await.unwrap();

    // Block B manually (simulating what happens when deps aren't met)
    engine.task_block(b, "waiting on Task A").await.unwrap();
    let task_b = engine.task_get(b).await.unwrap();
    assert_eq!(task_b.status, TaskStatus::Blocked);

    // Complete A — should auto-unblock B
    engine.task_done(a).await.unwrap();
    let task_b = engine.task_get(b).await.unwrap();
    assert_eq!(task_b.status, TaskStatus::Pending);
    assert_eq!(task_b.blocked_by, None);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_auto_unblock_partial_deps() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let b = engine.task_add("Task B", 5, None).await.unwrap();
    let c = engine.task_add("Task C", 5, None).await.unwrap();

    // C depends on both A and B
    engine.task_depends(c, a).await.unwrap();
    engine.task_depends(c, b).await.unwrap();
    engine.task_block(c, "waiting on A and B").await.unwrap();

    // Complete only A — C should stay blocked
    engine.task_done(a).await.unwrap();
    let task_c = engine.task_get(c).await.unwrap();
    assert_eq!(task_c.status, TaskStatus::Blocked);

    // Complete B — now C should be unblocked
    engine.task_done(b).await.unwrap();
    let task_c = engine.task_get(c).await.unwrap();
    assert_eq!(task_c.status, TaskStatus::Pending);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_undepend_removes_dependency() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let b = engine.task_add("Task B", 1, None).await.unwrap();
    engine.task_depends(b, a).await.unwrap();

    // B should not be next (blocked by dep)
    let next = engine.task_next().await.unwrap();
    assert_eq!(next.unwrap().id, a);

    // Remove the dependency
    engine.task_undepend(b, a).await.unwrap();

    // Now B should be next (higher priority)
    let next = engine.task_next().await.unwrap();
    assert_eq!(next.unwrap().id, b);

    env::set_current_dir(original_dir).unwrap();
}

// --- Event System Tests ---

/// Test handler that records events
struct RecordingHandler {
    events: Mutex<Vec<String>>,
}

impl RecordingHandler {
    fn new() -> Self {
        Self { events: Mutex::new(Vec::new()) }
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

impl EventHandler for RecordingHandler {
    fn handle(&self, event: &Event) {
        let label = match event {
            Event::TaskAdded { id, .. } => format!("task_added:{}", id),
            Event::TaskCompleted { id } => format!("task_completed:{}", id),
            Event::TaskBlocked { id, .. } => format!("task_blocked:{}", id),
            Event::TaskCancelled { id } => format!("task_cancelled:{}", id),
            Event::TaskDependencyAdded { task_id, depends_on_id } => {
                format!("dep_added:{}:{}", task_id, depends_on_id)
            }
            Event::TaskDependencyRemoved { task_id, depends_on_id } => {
                format!("dep_removed:{}:{}", task_id, depends_on_id)
            }
            Event::ConfigSet { key, .. } => format!("config_set:{}", key),
            Event::LearningAdded { id, .. } => format!("learning_added:{}", id),
            Event::LearningDeleted { id } => format!("learning_deleted:{}", id),
            _ => format!("{:?}", event),
        };
        self.events.lock().unwrap().push(label);
    }
}

#[tokio::test]
async fn test_event_handler_receives_task_events() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (mut engine, _tmp, original_dir) = setup_engine().await;

    let handler = Arc::new(RecordingHandler::new());
    engine.on_event(handler.clone());

    let id = engine.task_add("Event test task", 5, None).await.unwrap();
    engine.task_done(id).await.unwrap();

    let events = handler.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], format!("task_added:{}", id));
    assert_eq!(events[1], format!("task_completed:{}", id));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_event_handler_receives_config_events() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (mut engine, _tmp, original_dir) = setup_engine().await;

    let handler = Arc::new(RecordingHandler::new());
    engine.on_event(handler.clone());

    engine.config_set("foo", "bar").await.unwrap();

    let events = handler.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], "config_set:foo");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_multiple_event_handlers() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (mut engine, _tmp, original_dir) = setup_engine().await;

    let handler1 = Arc::new(RecordingHandler::new());
    let handler2 = Arc::new(RecordingHandler::new());
    engine.on_event(handler1.clone());
    engine.on_event(handler2.clone());

    engine.task_add("Multi handler test", 5, None).await.unwrap();

    // Both handlers should receive the event
    assert_eq!(handler1.events().len(), 1);
    assert_eq!(handler2.events().len(), 1);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_event_ordering() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (mut engine, _tmp, original_dir) = setup_engine().await;

    let handler = Arc::new(RecordingHandler::new());
    engine.on_event(handler.clone());

    let a = engine.task_add("Task A", 5, None).await.unwrap();
    let b = engine.task_add("Task B", 5, None).await.unwrap();
    engine.task_depends(b, a).await.unwrap();
    engine.task_block(b, "waiting").await.unwrap();
    engine.task_cancel(a).await.unwrap();

    let events = handler.events();
    assert_eq!(events.len(), 5);
    assert_eq!(events[0], format!("task_added:{}", a));
    assert_eq!(events[1], format!("task_added:{}", b));
    assert_eq!(events[2], format!("dep_added:{}:{}", b, a));
    assert_eq!(events[3], format!("task_blocked:{}", b));
    assert_eq!(events[4], format!("task_cancelled:{}", a));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_event_learning_lifecycle() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (mut engine, _tmp, original_dir) = setup_engine().await;

    let handler = Arc::new(RecordingHandler::new());
    engine.on_event(handler.clone());

    let id = engine.learn("Test learning", Some("pattern")).await.unwrap();
    engine.learnings_delete(id).await.unwrap();

    let events = handler.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], format!("learning_added:{}", id));
    assert_eq!(events[1], format!("learning_deleted:{}", id));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_no_events_without_handler() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    // No handler registered — operations should still succeed
    engine.task_add("No handler test", 5, None).await.unwrap();
    engine.config_set("key", "val").await.unwrap();

    env::set_current_dir(original_dir).unwrap();
}

// --- Provider System Tests ---

struct MockProvider {
    response: ProviderResponse,
}

impl MockProvider {
    fn new(output: &str, success: bool) -> Self {
        Self {
            response: ProviderResponse {
                output: output.to_string(),
                success,
                exit_code: if success { Some(0) } else { Some(1) },
                usage: Some(TokenUsage {
                    tokens_in: 100,
                    tokens_out: 200,
                    cost_usd: Some(0.003),
                }),
                model: Some("mock-model".to_string()),
                duration_secs: Some(1.5),
            },
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn execute(&self, _request: ProviderRequest) -> dial_core::Result<ProviderResponse> {
        Ok(self.response.clone())
    }

    async fn is_available(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn test_mock_provider_execute() {
    let provider = MockProvider::new("Hello from mock", true);
    let request = ProviderRequest {
        prompt: "test prompt".to_string(),
        work_dir: "/tmp".to_string(),
        max_tokens: None,
        model: None,
        timeout_secs: None,
    };

    let response = provider.execute(request).await.unwrap();
    assert!(response.success);
    assert_eq!(response.output, "Hello from mock");
    assert!(response.usage.is_some());
    let usage = response.usage.unwrap();
    assert_eq!(usage.tokens_in, 100);
    assert_eq!(usage.tokens_out, 200);
}

#[tokio::test]
async fn test_engine_provider_registration() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (mut engine, _tmp, original_dir) = setup_engine().await;

    assert!(engine.provider().is_none());

    let mock = Arc::new(MockProvider::new("test", true));
    engine.set_provider(mock);

    assert!(engine.provider().is_some());
    assert_eq!(engine.provider().unwrap().name(), "mock");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_record_usage() {
    let _lock = CWD_LOCK.lock().unwrap();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let response = ProviderResponse {
        output: "test output".to_string(),
        success: true,
        exit_code: Some(0),
        usage: Some(TokenUsage {
            tokens_in: 500,
            tokens_out: 1000,
            cost_usd: Some(0.015),
        }),
        model: Some("test-model".to_string()),
        duration_secs: Some(2.5),
    };

    engine.record_usage(None, &response, "mock").unwrap();

    // Verify it was stored
    let conn = dial_core::get_db(None).unwrap();
    let (provider, tokens_in, tokens_out, cost): (String, i64, i64, f64) = conn.query_row(
        "SELECT provider, tokens_in, tokens_out, cost_usd FROM provider_usage ORDER BY id DESC LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap();

    assert_eq!(provider, "mock");
    assert_eq!(tokens_in, 500);
    assert_eq!(tokens_out, 1000);
    assert!((cost - 0.015).abs() < 0.001);

    env::set_current_dir(original_dir).unwrap();
}
