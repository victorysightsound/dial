use crate::config;
use crate::db::{self, migrations};
use crate::errors::{DialError, Result};
use crate::event::{Event, EventHandler};
use crate::failure;
use crate::iteration;
use crate::learning;
use crate::provider::{Provider, ProviderResponse};
use crate::spec;
use crate::task;
use crate::task::models::Task;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;

/// Configuration for a single validation pipeline step (from DB).
#[derive(Debug, Clone)]
pub struct PipelineStepConfig {
    pub id: i64,
    pub name: String,
    pub command: String,
    pub sort_order: i32,
    pub required: bool,
    pub timeout_secs: Option<u64>,
}

/// Info about a failure pattern from the DB.
#[derive(Debug, Clone)]
pub struct PatternInfo {
    pub id: i64,
    pub pattern_key: String,
    pub description: String,
    pub category: Option<String>,
    pub regex_pattern: Option<String>,
    pub status: String,
    pub occurrence_count: i64,
}

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
    handlers: Vec<Arc<dyn EventHandler>>,
    provider: Option<Arc<dyn Provider>>,
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

        Ok(Self { config, handlers: Vec::new(), provider: None })
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
            handlers: Vec::new(),
            provider: None,
        })
    }

    /// Register an event handler.
    pub fn on_event(&mut self, handler: Arc<dyn EventHandler>) {
        self.handlers.push(handler);
    }

    /// Set the AI provider for automated operations.
    pub fn set_provider(&mut self, provider: Arc<dyn Provider>) {
        self.provider = Some(provider);
    }

    /// Get the configured provider, if any.
    pub fn provider(&self) -> Option<&Arc<dyn Provider>> {
        self.provider.as_ref()
    }

    /// Emit an event to all registered handlers.
    pub fn emit(&self, event: Event) {
        for handler in &self.handlers {
            handler.handle(&event);
        }
    }

    /// Record provider usage for an iteration.
    pub fn record_usage(
        &self,
        iteration_id: Option<i64>,
        response: &ProviderResponse,
        provider_name: &str,
    ) -> Result<()> {
        let conn = self.conn()?;
        let (tokens_in, tokens_out, cost_usd) = match &response.usage {
            Some(usage) => (
                usage.tokens_in as i64,
                usage.tokens_out as i64,
                usage.cost_usd.unwrap_or(0.0),
            ),
            None => (0, 0, 0.0),
        };

        conn.execute(
            "INSERT INTO provider_usage (iteration_id, provider, model, tokens_in, tokens_out, cost_usd, duration_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                iteration_id,
                provider_name,
                response.model,
                tokens_in,
                tokens_out,
                cost_usd,
                response.duration_secs,
            ],
        )?;
        Ok(())
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
        let result = config::config_set(key, value);
        if result.is_ok() {
            self.emit(Event::ConfigSet { key: key.to_string(), value: value.to_string() });
        }
        result
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
        let result = task::task_add(description, priority, spec_section_id);
        if let Ok(id) = &result {
            self.emit(Event::TaskAdded {
                id: *id,
                description: description.to_string(),
                priority,
            });
        }
        result
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
        let result = task::task_done(task_id);
        if result.is_ok() {
            self.emit(Event::TaskCompleted { id: task_id });
        }
        result
    }

    /// Block a task with a reason.
    pub async fn task_block(&self, task_id: i64, reason: &str) -> Result<()> {
        let result = task::task_block(task_id, reason);
        if result.is_ok() {
            self.emit(Event::TaskBlocked { id: task_id, reason: reason.to_string() });
        }
        result
    }

    /// Cancel a task.
    pub async fn task_cancel(&self, task_id: i64) -> Result<()> {
        let result = task::task_cancel(task_id);
        if result.is_ok() {
            self.emit(Event::TaskCancelled { id: task_id });
        }
        result
    }

    /// Search tasks by query.
    pub async fn task_search(&self, query: &str) -> Result<()> {
        task::task_search(query)
    }

    /// Get a task by ID.
    pub async fn task_get(&self, task_id: i64) -> Result<Task> {
        task::get_task_by_id(task_id)
    }

    /// Add a dependency: task_id depends on depends_on_id.
    pub async fn task_depends(&self, task_id: i64, depends_on_id: i64) -> Result<()> {
        let result = task::task_depends(task_id, depends_on_id);
        if result.is_ok() {
            self.emit(Event::TaskDependencyAdded { task_id, depends_on_id });
        }
        result
    }

    /// Remove a dependency.
    pub async fn task_undepend(&self, task_id: i64, depends_on_id: i64) -> Result<()> {
        let result = task::task_undepend(task_id, depends_on_id);
        if result.is_ok() {
            self.emit(Event::TaskDependencyRemoved { task_id, depends_on_id });
        }
        result
    }

    /// Get all tasks that task_id depends on.
    pub async fn task_get_dependencies(&self, task_id: i64) -> Result<Vec<i64>> {
        task::task_get_dependencies(task_id)
    }

    /// Get all tasks that depend on task_id.
    pub async fn task_get_dependents(&self, task_id: i64) -> Result<Vec<i64>> {
        task::task_get_dependents(task_id)
    }

    /// Check if all dependencies of a task are satisfied.
    pub async fn task_deps_satisfied(&self, task_id: i64) -> Result<bool> {
        task::task_deps_satisfied(task_id)
    }

    /// Show dependency info for a task.
    pub async fn task_show_deps(&self, task_id: i64) -> Result<()> {
        task::task_show_deps(task_id)
    }

    // --- Iteration ---

    /// Run one iteration (pick next task, set up context).
    pub async fn iterate(&self) -> Result<(bool, String)> {
        iteration::iterate_once()
    }

    /// Validate the current iteration (run build + test).
    /// Emits per-step events (StepPassed, StepFailed, StepSkipped) to registered handlers.
    pub async fn validate(&self) -> Result<bool> {
        let result = iteration::validate_current_with_details()?;

        // Emit per-step events
        for step in &result.step_results {
            if step.skipped {
                self.emit(Event::StepSkipped {
                    name: step.name.clone(),
                    reason: "prior required step failed".to_string(),
                });
            } else if step.passed {
                self.emit(Event::StepPassed {
                    name: step.name.clone(),
                    duration_secs: step.duration_secs,
                });
            } else {
                self.emit(Event::StepFailed {
                    name: step.name.clone(),
                    required: step.required,
                    output: step.output.clone(),
                    duration_secs: step.duration_secs,
                });
            }
        }

        Ok(result.success)
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

    /// Gather context items for a task with a token budget.
    /// Returns the formatted context and any excluded item labels.
    pub async fn gather_context_budgeted(
        &self,
        task: &crate::task::models::Task,
        token_budget: usize,
    ) -> Result<(String, Vec<String>)> {
        let conn = self.conn()?;
        let (context, excluded) = iteration::gather_context_budgeted(&conn, task, token_budget)?;

        // Emit warnings for excluded items
        for label in &excluded {
            self.emit(Event::Warning(format!("Context truncated: '{}' excluded (budget exceeded)", label)));
        }

        Ok((context, excluded))
    }

    /// Run automated orchestration with fresh AI subprocesses.
    pub async fn auto_run(&self, max_iterations: Option<u32>, cli: Option<&str>) -> Result<()> {
        iteration::auto_run(max_iterations, cli)
    }

    // --- Validation Pipeline ---

    /// Add a step to the validation pipeline.
    pub async fn pipeline_add(
        &self,
        name: &str,
        command: &str,
        sort_order: i32,
        required: bool,
        timeout_secs: Option<u64>,
    ) -> Result<i64> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO validation_steps (name, command, sort_order, required, timeout_secs)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                name,
                command,
                sort_order,
                if required { 1 } else { 0 },
                timeout_secs.map(|t| t as i64),
            ],
        )?;
        let id = conn.last_insert_rowid();
        self.emit(Event::Info(format!("Added pipeline step '{}': {}", name, command)));
        Ok(id)
    }

    /// Remove a step from the validation pipeline by ID.
    pub async fn pipeline_remove(&self, step_id: i64) -> Result<()> {
        let conn = self.conn()?;
        let affected = conn.execute(
            "DELETE FROM validation_steps WHERE id = ?1",
            [step_id],
        )?;
        if affected == 0 {
            return Err(DialError::UserError(format!("Pipeline step #{} not found", step_id)));
        }
        self.emit(Event::Info(format!("Removed pipeline step #{}", step_id)));
        Ok(())
    }

    /// List all configured validation pipeline steps.
    pub async fn pipeline_list(&self) -> Result<Vec<PipelineStepConfig>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, command, sort_order, required, timeout_secs
             FROM validation_steps ORDER BY sort_order, id",
        )?;

        let steps: Vec<PipelineStepConfig> = stmt
            .query_map([], |row| {
                Ok(PipelineStepConfig {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    command: row.get(2)?,
                    sort_order: row.get(3)?,
                    required: row.get::<_, i64>(4)? != 0,
                    timeout_secs: row.get::<_, Option<i64>>(5)?.map(|t| t as u64),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(steps)
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

    // --- Patterns ---

    /// Suggest new patterns by clustering unknown errors.
    pub async fn patterns_suggest(&self) -> Result<Vec<crate::failure::SuggestedPattern>> {
        let conn = self.conn()?;
        Ok(crate::failure::suggest_patterns_from_clustering(&conn))
    }

    /// List all patterns from the DB.
    pub async fn patterns_list(&self) -> Result<Vec<PatternInfo>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, pattern_key, description, category, regex_pattern, status, occurrence_count
             FROM failure_patterns ORDER BY occurrence_count DESC",
        )?;

        let patterns = stmt
            .query_map([], |row| {
                Ok(PatternInfo {
                    id: row.get(0)?,
                    pattern_key: row.get(1)?,
                    description: row.get(2)?,
                    category: row.get(3)?,
                    regex_pattern: row.get(4)?,
                    status: row.get(5)?,
                    occurrence_count: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(patterns)
    }

    /// Add a new pattern to the DB.
    pub async fn patterns_add(
        &self,
        pattern_key: &str,
        description: &str,
        category: &str,
        regex_pattern: &str,
        status: &str,
    ) -> Result<i64> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category, regex_pattern, status)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![pattern_key, description, category, regex_pattern, status],
        )?;
        let id = conn.last_insert_rowid();
        self.emit(Event::Info(format!("Added pattern '{}' [{}]", pattern_key, status)));
        Ok(id)
    }

    /// Promote a pattern's status: suggested -> confirmed -> trusted.
    pub async fn patterns_promote(&self, pattern_id: i64) -> Result<String> {
        let conn = self.conn()?;
        let current_status: String = conn.query_row(
            "SELECT status FROM failure_patterns WHERE id = ?1",
            [pattern_id],
            |row| row.get(0),
        ).map_err(|_| DialError::UserError(format!("Pattern #{} not found", pattern_id)))?;

        let new_status = match current_status.as_str() {
            "suggested" => "confirmed",
            "confirmed" => "trusted",
            "trusted" => {
                return Err(DialError::UserError("Pattern is already trusted".to_string()));
            }
            other => {
                return Err(DialError::UserError(format!("Unknown status: {}", other)));
            }
        };

        conn.execute(
            "UPDATE failure_patterns SET status = ?1 WHERE id = ?2",
            rusqlite::params![new_status, pattern_id],
        )?;

        self.emit(Event::Info(format!("Pattern #{} promoted: {} -> {}", pattern_id, current_status, new_status)));
        Ok(new_status.to_string())
    }

    // --- Learnings ---

    /// Add a learning. Returns the learning ID.
    pub async fn learn(&self, description: &str, category: Option<&str>) -> Result<i64> {
        let result = learning::add_learning(description, category);
        if let Ok(id) = &result {
            self.emit(Event::LearningAdded {
                id: *id,
                description: description.to_string(),
                category: category.map(|c| c.to_string()),
            });
        }
        result
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
        let result = learning::delete_learning(id);
        if result.is_ok() {
            self.emit(Event::LearningDeleted { id });
        }
        result
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
