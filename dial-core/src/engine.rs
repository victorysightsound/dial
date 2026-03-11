use crate::config;
use crate::db::{self, migrations};
use crate::errors::{DialError, Result};
use crate::failure;
use crate::iteration;
use crate::learning;
use crate::spec;
use crate::task;
use crate::task::models::Task;
use rusqlite::Connection;
use std::path::PathBuf;

/// Configuration for the DIAL engine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Working directory for this engine instance.
    pub work_dir: PathBuf,
    /// Phase name (e.g., "mvp", "v3").
    pub phase: Option<String>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            work_dir: std::env::current_dir().unwrap_or_default(),
            phase: None,
        }
    }
}

/// The DIAL engine. Central API for all operations.
///
/// All public methods are async to support truly async operations like
/// AI provider API calls (added in later phases). Database operations
/// use rusqlite (sync) internally — the async boundary allows future
/// migration to async DB or wrapping with spawn_blocking as needed.
pub struct Engine {
    config: EngineConfig,
}

impl Engine {
    /// Open an existing DIAL project. Runs migrations automatically.
    pub async fn open(config: EngineConfig) -> Result<Self> {
        let dial_dir = config.work_dir.join(".dial");
        if !dial_dir.exists() {
            return Err(DialError::NotInitialized);
        }

        // Verify DB exists and run migrations
        let _conn = db::get_db(config.phase.as_deref())?;

        Ok(Self { config })
    }

    /// Initialize a new DIAL project.
    pub async fn init(
        phase: &str,
        import_solutions_from: Option<&str>,
        setup_agents: bool,
    ) -> Result<Self> {
        db::init_db(phase, import_solutions_from, setup_agents)?;

        Ok(Self {
            config: EngineConfig {
                work_dir: std::env::current_dir().unwrap_or_default(),
                phase: Some(phase.to_string()),
            },
        })
    }

    /// Get the .dial directory path.
    pub fn dial_dir(&self) -> PathBuf {
        self.config.work_dir.join(".dial")
    }

    /// Get the engine configuration.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Get a database connection.
    fn conn(&self) -> Result<Connection> {
        db::get_db(self.config.phase.as_deref())
    }

    /// Get the current schema version.
    pub async fn schema_version(&self) -> Result<i64> {
        let conn = self.conn()?;
        migrations::current_version(&conn)
    }

    // --- Configuration ---

    /// Get a config value.
    pub async fn config_get(&self, key: &str) -> Result<Option<String>> {
        config::config_get(key)
    }

    /// Set a config value.
    pub async fn config_set(&self, key: &str, value: &str) -> Result<()> {
        config::config_set(key, value)
    }

    /// Show all config (prints to stdout).
    pub async fn config_show(&self) -> Result<()> {
        config::config_show()
    }

    // --- Tasks ---

    /// Add a new task. Returns the task ID.
    pub async fn task_add(
        &self,
        description: &str,
        priority: i32,
        spec_section_id: Option<i64>,
    ) -> Result<i64> {
        task::task_add(description, priority, spec_section_id)
    }

    /// List tasks.
    pub async fn task_list(&self, show_all: bool) -> Result<()> {
        task::task_list(show_all)
    }

    /// Get the next pending task.
    pub async fn task_next(&self) -> Result<Option<Task>> {
        task::task_next()
    }

    /// Mark a task as done.
    pub async fn task_done(&self, task_id: i64) -> Result<()> {
        task::task_done(task_id)
    }

    /// Block a task with a reason.
    pub async fn task_block(&self, task_id: i64, reason: &str) -> Result<()> {
        task::task_block(task_id, reason)
    }

    /// Cancel a task.
    pub async fn task_cancel(&self, task_id: i64) -> Result<()> {
        task::task_cancel(task_id)
    }

    /// Search tasks by query.
    pub async fn task_search(&self, query: &str) -> Result<()> {
        task::task_search(query)
    }

    /// Get a task by ID.
    pub async fn task_get(&self, task_id: i64) -> Result<Task> {
        task::get_task_by_id(task_id)
    }

    // --- Iteration ---

    /// Run one iteration (pick next task, set up context).
    pub async fn iterate(&self) -> Result<(bool, String)> {
        iteration::iterate_once()
    }

    /// Validate the current iteration (run build + test).
    pub async fn validate(&self) -> Result<bool> {
        iteration::validate_current()
    }

    /// Run the iteration loop until tasks are exhausted or stopped.
    pub async fn run(&self, max_iterations: Option<u32>) -> Result<()> {
        iteration::run_loop(max_iterations)
    }

    /// Stop the iteration loop gracefully.
    pub async fn stop(&self) -> Result<()> {
        iteration::stop_loop()
    }

    /// Revert to the last successful commit.
    pub async fn revert(&self) -> Result<bool> {
        iteration::revert_to_last_good()
    }

    /// Reset the current in-progress iteration.
    pub async fn reset(&self) -> Result<()> {
        iteration::reset_current()
    }

    /// Show fresh context for current/next task.
    pub async fn show_context(&self) -> Result<()> {
        iteration::show_context()
    }

    /// Generate an orchestrator prompt for sub-agent spawning.
    pub async fn orchestrate(&self) -> Result<()> {
        iteration::orchestrate()
    }

    /// Run automated orchestration with fresh AI subprocesses.
    pub async fn auto_run(&self, max_iterations: Option<u32>, cli: Option<&str>) -> Result<()> {
        iteration::auto_run(max_iterations, cli)
    }

    // --- Failures & Solutions ---

    /// Show failure records.
    pub async fn show_failures(&self, unresolved_only: bool) -> Result<()> {
        failure::show_failures(unresolved_only)
    }

    /// Show solutions.
    pub async fn show_solutions(&self, trusted_only: bool) -> Result<()> {
        failure::show_solutions(trusted_only)
    }

    // --- Learnings ---

    /// Add a learning. Returns the learning ID.
    pub async fn learn(&self, description: &str, category: Option<&str>) -> Result<i64> {
        learning::add_learning(description, category)
    }

    /// List learnings.
    pub async fn learnings_list(&self, category: Option<&str>) -> Result<()> {
        learning::list_learnings(category)
    }

    /// Search learnings.
    pub async fn learnings_search(&self, query: &str) -> Result<Vec<learning::LearningResult>> {
        learning::search_learnings(query)
    }

    /// Delete a learning.
    pub async fn learnings_delete(&self, id: i64) -> Result<()> {
        learning::delete_learning(id)
    }

    // --- Specs ---

    /// Index spec files from a directory. Returns true if specs were found.
    pub async fn index_specs(&self, dir: &str) -> Result<bool> {
        spec::index_specs(dir)
    }

    /// Search specs.
    pub async fn spec_search(&self, query: &str) -> Result<Vec<spec::SpecSearchResult>> {
        spec::spec_search(query)
    }

    /// Show a spec section.
    pub async fn spec_show(&self, id: i64) -> Result<Option<spec::SpecSearchResult>> {
        spec::spec_show(id)
    }

    /// List all spec sections.
    pub async fn spec_list(&self) -> Result<()> {
        spec::spec_list()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_config_default() {
        let config = EngineConfig::default();
        assert!(config.phase.is_none());
        assert!(!config.work_dir.as_os_str().is_empty());
    }

    #[tokio::test]
    async fn test_engine_open_uninitialized_fails() {
        let config = EngineConfig {
            work_dir: PathBuf::from("/tmp/nonexistent-dial-test"),
            phase: None,
        };
        let result = Engine::open(config).await;
        assert!(result.is_err());
    }
}
