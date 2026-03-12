use crate::budget;
use crate::config;
use crate::db::{self, migrations};
use crate::errors::{DialError, Result};
use crate::event::{Event, EventHandler};
use crate::failure;
use crate::iteration;
use crate::learning;
use crate::prd;
use crate::provider::{Provider, ProviderResponse};
use crate::spec;
use crate::task;
use crate::task::models::Task;
use rusqlite::Connection;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

/// Result of a dry-run/preview iteration. Contains all the information
/// that would be used in a real iteration, without creating DB records
/// or spawning subagents.
#[derive(Debug, Clone, Serialize)]
pub struct DryRunResult {
    /// The task that would be executed.
    pub task: Task,
    /// Context items that fit within the token budget: (label, token_count).
    pub context_items_included: Vec<(String, usize)>,
    /// Context items excluded due to budget overflow: (label, token_count).
    pub context_items_excluded: Vec<(String, usize)>,
    /// Total tokens across all included context items.
    pub total_context_tokens: usize,
    /// The token budget that was applied.
    pub token_budget: usize,
    /// First 500 characters of the prompt that would be sent.
    pub prompt_preview: String,
    /// Descriptions of trusted solutions that would be suggested.
    pub suggested_solutions: Vec<String>,
    /// Whether all task dependencies are satisfied.
    pub dependencies_satisfied: bool,
}

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

/// Approval mode for iterations.
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalMode {
    /// Auto-commit on validation pass (default, existing behavior).
    Auto,
    /// Emit ApprovalRequired event with diff summary, pause iteration for review.
    Review,
    /// Always require manual approval before committing.
    Manual,
}

impl ApprovalMode {
    /// Parse an approval mode from a string ("auto", "review", "manual").
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "review" => Some(Self::Review),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

impl std::fmt::Display for ApprovalMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Review => write!(f, "review"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// Configuration for the DIAL engine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Working directory for this engine instance.
    pub work_dir: PathBuf,
    /// Phase name (e.g., "mvp", "v3").
    pub phase: Option<String>,
    /// Approval mode for iterations.
    pub approval_mode: ApprovalMode,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            work_dir: std::env::current_dir().unwrap_or_default(),
            phase: None,
            approval_mode: ApprovalMode::Auto,
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
                approval_mode: ApprovalMode::Auto,
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

    /// Return tasks where total_failures >= threshold (chronic failures).
    pub async fn chronic_failures(&self, threshold: i64) -> Result<Vec<task::ChronicFailureInfo>> {
        let conn = self.conn()?;
        let results = task::get_chronic_failures_with_conn(&conn, threshold)?;
        if !results.is_empty() {
            for r in &results {
                self.emit(Event::ChronicFailureDetected {
                    task_id: r.task_id,
                    total_failures: r.total_failures,
                    total_attempts: r.total_attempts,
                });
            }
        }
        Ok(results)
    }

    // --- Iteration ---

    /// Run one iteration (pick next task, set up context).
    pub async fn iterate(&self) -> Result<(bool, String)> {
        iteration::iterate_once()
    }

    /// Dry-run / preview mode: selects the next task, assembles context,
    /// generates the prompt, and returns a DryRunResult WITHOUT creating
    /// iteration records, updating task status, or spawning subagents.
    pub async fn iterate_dry_run(&self) -> Result<DryRunResult> {
        let conn = self.conn()?;

        // Get next pending task (same query as iterate_once / auto_run)
        let mut stmt = conn.prepare(
            "SELECT id, description, status, priority, blocked_by, spec_section_id, created_at, started_at, completed_at
             FROM tasks WHERE status = 'pending'
             AND id NOT IN (
                 SELECT td.task_id FROM task_dependencies td
                 INNER JOIN tasks dep ON dep.id = td.depends_on_id
                 WHERE dep.status != 'completed'
             )
             ORDER BY priority, id LIMIT 1",
        )?;

        let task: Option<Task> = stmt.query_row([], |row| Task::from_row(row)).ok();

        let task = match task {
            Some(t) => t,
            None => {
                return Err(DialError::UserError("No pending tasks available for dry run.".to_string()));
            }
        };

        // Check dependency satisfaction
        let unsatisfied: i64 = conn.query_row(
            "SELECT COUNT(*) FROM task_dependencies td
             INNER JOIN tasks t ON t.id = td.depends_on_id
             WHERE td.task_id = ?1 AND t.status != 'completed'",
            [task.id],
            |row| row.get(0),
        )?;
        let dependencies_satisfied = unsatisfied == 0;

        // Gather context items WITHOUT side effects (no reference counting)
        let items = iteration::gather_context_items_pure(&conn, &task)?;

        // Get token budget from config (default 8000)
        let token_budget: usize = crate::config::config_get("token_budget")?
            .and_then(|s| s.parse().ok())
            .unwrap_or(8000);

        // Assemble within budget
        let (included, excluded) = budget::assemble_context(&items, token_budget);

        let context_items_included: Vec<(String, usize)> = included
            .iter()
            .map(|item| (item.label.clone(), item.tokens))
            .collect();

        let context_items_excluded: Vec<(String, usize)> = excluded
            .iter()
            .map(|item| (item.label.clone(), item.tokens))
            .collect();

        let total_context_tokens: usize = included.iter().map(|item| item.tokens).sum();

        // Generate the full prompt (side-effect-free — uses gather_context which
        // does increment references, but we use gather_context_without_signs
        // and build the prompt manually to avoid that)
        let context_text = budget::format_context(&included);
        let full_prompt = format!(
            "# DIAL Sub-Agent Task\n\nYou are a fresh AI agent spawned by DIAL to complete ONE task.\n\n## Your Task\n**Task #{id}:** {desc}\n\n{context}",
            id = task.id,
            desc = task.description,
            context = context_text,
        );

        let prompt_preview = if full_prompt.len() > 500 {
            full_prompt[..500].to_string()
        } else {
            full_prompt.clone()
        };

        // Find suggested solutions for recent unresolved failures
        let mut sol_stmt = conn.prepare(
            "SELECT DISTINCT s.description
             FROM failures f
             INNER JOIN failure_patterns fp ON f.pattern_id = fp.id
             INNER JOIN solutions s ON s.pattern_id = fp.id
             WHERE f.resolved = 0 AND s.confidence >= ?1
             ORDER BY s.confidence DESC LIMIT 10",
        )?;

        let suggested_solutions: Vec<String> = sol_stmt
            .query_map([crate::TRUST_THRESHOLD], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(DryRunResult {
            task,
            context_items_included,
            context_items_excluded,
            total_context_tokens,
            token_budget,
            prompt_preview,
            suggested_solutions,
            dependencies_satisfied,
        })
    }

    /// Validate the current iteration (run build + test).
    /// Emits per-step events (StepPassed, StepFailed, StepSkipped) to registered handlers.
    /// On failure: emits SolutionSuggested if trusted solutions exist for the failure pattern.
    /// On success: increments confidence for any previously suggested solutions.
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

        // Emit SolutionSuggested events for failures with auto-suggested solutions
        if !result.suggested_solutions.is_empty() {
            // Group by failure_id
            let mut by_failure: std::collections::HashMap<i64, Vec<(i64, String, f64)>> =
                std::collections::HashMap::new();
            for (failure_id, sol_id, desc, conf) in &result.suggested_solutions {
                by_failure
                    .entry(*failure_id)
                    .or_default()
                    .push((*sol_id, desc.clone(), *conf));
            }
            for (failure_id, solutions) in by_failure {
                self.emit(Event::SolutionSuggested { failure_id, solutions });
            }
        }

        // After successful validation, increment confidence for previously suggested solutions
        if result.success {
            if let Some(task_id) = result.task_id {
                let conn = self.conn()?;
                let boosted = failure::mark_solution_applications_success(&conn, task_id)?;
                if !boosted.is_empty() {
                    self.emit(Event::Info(format!(
                        "Boosted confidence for {} solution(s) after successful validation",
                        boosted.len()
                    )));
                }
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

    // --- Approval ---

    /// Get the current approval mode.
    pub fn approval_mode(&self) -> &ApprovalMode {
        &self.config.approval_mode
    }

    /// Set the approval mode.
    pub fn set_approval_mode(&mut self, mode: ApprovalMode) {
        self.config.approval_mode = mode;
    }

    /// Generate a diff summary from git diff --stat.
    pub fn diff_summary(&self) -> Result<String> {
        if !crate::git::git_is_repo() {
            return Ok("(not a git repo)".to_string());
        }

        let output = std::process::Command::new("git")
            .args(["diff", "--stat"])
            .output()
            .map_err(|e| DialError::CommandFailed(e.to_string()))?;

        let diff_stat = String::from_utf8_lossy(&output.stdout).to_string();

        if diff_stat.is_empty() {
            // Try staged changes
            let output = std::process::Command::new("git")
                .args(["diff", "--cached", "--stat"])
                .output()
                .map_err(|e| DialError::CommandFailed(e.to_string()))?;

            let cached = String::from_utf8_lossy(&output.stdout).to_string();
            if cached.is_empty() {
                return Ok("(no changes)".to_string());
            }
            return Ok(cached);
        }

        Ok(diff_stat)
    }

    /// Approve a paused iteration (in Review/Manual mode).
    pub async fn approve(&self) -> Result<()> {
        let conn = self.conn()?;

        // Find the paused iteration
        let iteration: Option<(i64, i64)> = conn
            .query_row(
                "SELECT id, task_id FROM iterations WHERE status = 'awaiting_approval' ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        let (iteration_id, _task_id) = match iteration {
            Some(i) => i,
            None => return Err(DialError::UserError("No iteration awaiting approval".to_string())),
        };

        // Resume by setting back to in_progress, then committing
        conn.execute(
            "UPDATE iterations SET status = 'in_progress' WHERE id = ?1",
            [iteration_id],
        )?;

        self.emit(Event::Approved { iteration_id });

        // Now validate and commit normally
        self.validate().await?;

        Ok(())
    }

    /// Reject a paused iteration (in Review/Manual mode).
    pub async fn reject(&self, reason: &str) -> Result<()> {
        let conn = self.conn()?;

        let iteration: Option<(i64, i64)> = conn
            .query_row(
                "SELECT id, task_id FROM iterations WHERE status = 'awaiting_approval' ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        let (iteration_id, task_id) = match iteration {
            Some(i) => i,
            None => return Err(DialError::UserError("No iteration awaiting approval".to_string())),
        };

        let now = chrono::Local::now().to_rfc3339();

        // Mark iteration as rejected
        conn.execute(
            "UPDATE iterations SET status = 'rejected', ended_at = ?1, notes = ?2 WHERE id = ?3",
            rusqlite::params![now, reason, iteration_id],
        )?;

        // Reset task to pending
        conn.execute(
            "UPDATE tasks SET status = 'pending' WHERE id = ?1",
            [task_id],
        )?;

        self.emit(Event::Rejected { iteration_id, reason: reason.to_string() });

        // Revert changes if in git repo
        if crate::git::git_is_repo() && crate::git::git_has_changes() {
            let _ = std::process::Command::new("git")
                .args(["checkout", "."])
                .output();
        }

        Ok(())
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

    /// Compute aggregated metrics per failure pattern.
    pub async fn pattern_metrics(&self) -> Result<Vec<crate::failure::PatternMetrics>> {
        let conn = self.conn()?;
        crate::failure::compute_pattern_metrics(&conn)
    }

    // --- Solutions ---

    /// Apply confidence decay to stale solutions.
    /// Decays by 0.05 per 30 days without validation.
    pub async fn solutions_decay(&self) -> Result<usize> {
        let conn = self.conn()?;
        let count = crate::failure::apply_confidence_decay(&conn, 0.05, 30)?;
        if count > 0 {
            self.emit(Event::Info(format!("Decayed confidence on {} stale solutions", count)));
        }
        Ok(count)
    }

    /// Refresh/validate a solution (resets its decay clock).
    pub async fn solutions_refresh(&self, solution_id: i64) -> Result<()> {
        let conn = self.conn()?;
        crate::failure::validate_solution(&conn, solution_id)?;
        self.emit(Event::Info(format!("Solution #{} re-validated", solution_id)));
        Ok(())
    }

    /// Get history for a solution.
    pub async fn solutions_history(&self, solution_id: i64) -> Result<Vec<crate::failure::SolutionEvent>> {
        let conn = self.conn()?;
        crate::failure::get_solution_history(&conn, solution_id)
    }

    // --- Learnings ---

    /// Add a learning. Returns the learning ID.
    pub async fn learn(&self, description: &str, category: Option<&str>) -> Result<i64> {
        self.learn_linked(description, category, None, None).await
    }

    /// Add a learning with optional pattern and iteration linking. Returns the learning ID.
    pub async fn learn_linked(
        &self,
        description: &str,
        category: Option<&str>,
        pattern_id: Option<i64>,
        iteration_id: Option<i64>,
    ) -> Result<i64> {
        let result = learning::add_learning_linked(description, category, pattern_id, iteration_id);
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

    /// List learnings for a specific pattern.
    pub async fn learnings_for_pattern(&self, pattern_id: i64) -> Result<Vec<learning::LearningResult>> {
        let conn = self.conn()?;
        learning::learnings_for_pattern(&conn, pattern_id)
    }

    /// Display learnings for a specific pattern (CLI output).
    pub async fn learnings_list_for_pattern(&self, pattern_id: i64) -> Result<()> {
        learning::list_learnings_for_pattern(pattern_id)
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

    // --- PRD (Structured Spec Database) ---

    /// Get a connection to the PRD database (creates if needed).
    fn prd_conn(&self) -> Result<Connection> {
        prd::get_or_init_prd_db()
    }

    /// Import spec files from a directory into prd.db.
    pub async fn prd_import(&self, specs_dir: &str) -> Result<()> {
        let result = prd::import::prd_import(specs_dir)?;
        self.emit(Event::PrdImported { files: result.files, sections: result.sections });
        Ok(())
    }

    /// Search PRD sections by query (FTS5).
    pub async fn prd_search(&self, query: &str) -> Result<Vec<prd::PrdSection>> {
        let conn = self.prd_conn()?;
        prd::prd_search_sections(&conn, query)
    }

    /// Show a single PRD section by its dotted section_id.
    pub async fn prd_show(&self, section_id: &str) -> Result<Option<prd::PrdSection>> {
        let conn = self.prd_conn()?;
        prd::prd_get_section(&conn, section_id)
    }

    /// List all PRD sections.
    pub async fn prd_list(&self) -> Result<Vec<prd::PrdSection>> {
        let conn = self.prd_conn()?;
        prd::prd_list_sections(&conn)
    }

    /// Add a terminology entry to the PRD.
    pub async fn prd_term_add(
        &self,
        canonical: &str,
        variants_json: &str,
        definition: &str,
        category: &str,
        first_used_in: Option<&str>,
    ) -> Result<i64> {
        let conn = self.prd_conn()?;
        let id = prd::prd_add_term(&conn, canonical, variants_json, definition, category, first_used_in)?;
        self.emit(Event::TermAdded { canonical: canonical.to_string(), category: category.to_string() });
        Ok(id)
    }

    /// List terminology entries, optionally filtered by category.
    pub async fn prd_term_list(&self, category: Option<&str>) -> Result<Vec<prd::PrdTerm>> {
        let conn = self.prd_conn()?;
        prd::prd_list_terms(&conn, category)
    }

    /// Search terminology by query (FTS5).
    pub async fn prd_term_search(&self, query: &str) -> Result<Vec<prd::PrdTerm>> {
        let conn = self.prd_conn()?;
        prd::prd_search_terms(&conn, query)
    }

    /// Run the PRD wizard (interactive spec generation).
    pub async fn prd_wizard(
        &self,
        template: &str,
        from_doc: Option<&str>,
        resume: bool,
    ) -> Result<()> {
        let provider = self.provider.as_ref()
            .ok_or(DialError::ProviderRequired)?;

        let conn = self.prd_conn()?;

        if resume {
            self.emit(Event::WizardResumed { phase: 0 });
        }

        let result = prd::wizard::run_wizard(
            provider.as_ref(),
            &conn,
            template,
            from_doc,
            resume,
            false,
        ).await?;

        self.emit(Event::WizardCompleted {
            sections_generated: result.sections_generated,
            tasks_generated: result.tasks_generated,
        });

        Ok(())
    }

    /// Run the full new-project wizard (phases 1-9).
    ///
    /// Used by `dial new` to create a project from scratch, including
    /// spec generation, task review, build/test config, iteration mode,
    /// and launch summary.
    pub async fn new_project(
        &self,
        template: &str,
        from_doc: Option<&str>,
        resume: bool,
    ) -> Result<()> {
        let provider = self.provider.as_ref()
            .ok_or(DialError::ProviderRequired)?;

        let conn = self.prd_conn()?;

        if resume {
            self.emit(Event::WizardResumed { phase: 0 });
        }

        let result = prd::wizard::run_wizard(
            provider.as_ref(),
            &conn,
            template,
            from_doc,
            resume,
            true,
        ).await?;

        self.emit(Event::WizardCompleted {
            sections_generated: result.sections_generated,
            tasks_generated: result.tasks_generated,
        });

        Ok(())
    }

    /// Migrate existing spec_sections from the phase DB into prd.db.
    pub async fn prd_migrate(&self) -> Result<usize> {
        let count = prd::import::migrate_spec_sections_to_prd()?;
        self.emit(Event::PrdImported { files: 0, sections: count });
        Ok(count)
    }

    // --- Crash Recovery ---

    /// Detect and recover from dangling in_progress iterations.
    /// Returns the number of iterations that were reset.
    pub async fn recover(&self) -> Result<u64> {
        let conn = self.conn()?;

        let mut stmt = conn.prepare(
            "SELECT id, task_id FROM iterations WHERE status = 'in_progress'",
        )?;
        let dangling: Vec<(i64, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        if dangling.is_empty() {
            self.emit(Event::Info("No dangling iterations found.".to_string()));
            return Ok(0);
        }

        let count = dangling.len() as u64;
        let now = chrono::Local::now().to_rfc3339();

        for (iter_id, task_id) in &dangling {
            conn.execute(
                "UPDATE iterations SET status = 'failed', ended_at = ?1, notes = 'Recovered from crash' WHERE id = ?2",
                rusqlite::params![now, iter_id],
            )?;
            conn.execute(
                "UPDATE tasks SET status = 'pending' WHERE id = ?1 AND status = 'in_progress'",
                [task_id],
            )?;
            self.emit(Event::Warning(format!(
                "Recovered iteration #{} (task #{}): marked as failed, task reset to pending",
                iter_id, task_id
            )));
        }

        self.emit(Event::Info(format!("Recovered {} dangling iteration(s).", count)));
        Ok(count)
    }

    // --- Migration ---

    /// Migrate data from a v2 DIAL database (best-effort).
    pub async fn migrate_v2(&self, v2_path: &str) -> Result<()> {
        use std::path::Path;

        let path = Path::new(v2_path);
        if !path.exists() {
            return Err(DialError::UserError(format!("V2 database not found: {}", v2_path)));
        }

        let v2_conn = rusqlite::Connection::open(path)
            .map_err(DialError::Database)?;
        let conn = self.conn()?;

        let mut migrated = 0;

        // Migrate tasks if the table exists
        let has_tasks: bool = v2_conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='tasks'",
            [], |row| row.get(0),
        ).unwrap_or(false);

        if has_tasks {
            let mut stmt = v2_conn.prepare(
                "SELECT description, priority, status FROM tasks",
            ).map_err(DialError::Database)?;

            let tasks = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1).unwrap_or(5),
                    row.get::<_, String>(2).unwrap_or_else(|_| "pending".to_string()),
                ))
            }).map_err(DialError::Database)?;

            for task in tasks {
                if let Ok((desc, priority, status)) = task {
                    let result = conn.execute(
                        "INSERT OR IGNORE INTO tasks (description, priority, status) VALUES (?1, ?2, ?3)",
                        rusqlite::params![desc, priority, status],
                    );
                    if result.is_ok() {
                        migrated += 1;
                    }
                }
            }
        }

        // Migrate learnings if the table exists
        let has_learnings: bool = v2_conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='learnings'",
            [], |row| row.get(0),
        ).unwrap_or(false);

        if has_learnings {
            let mut stmt = v2_conn.prepare(
                "SELECT description, category FROM learnings",
            ).map_err(DialError::Database)?;

            let learnings = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            }).map_err(DialError::Database)?;

            for learning in learnings {
                if let Ok((desc, cat)) = learning {
                    let result = conn.execute(
                        "INSERT OR IGNORE INTO learnings (description, category) VALUES (?1, ?2)",
                        rusqlite::params![desc, cat],
                    );
                    if result.is_ok() {
                        migrated += 1;
                    }
                }
            }
        }

        // Migrate config if the table exists
        let has_config: bool = v2_conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='config'",
            [], |row| row.get(0),
        ).unwrap_or(false);

        if has_config {
            let mut stmt = v2_conn.prepare(
                "SELECT key, value FROM config",
            ).map_err(DialError::Database)?;

            let configs = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }).map_err(DialError::Database)?;

            for config in configs {
                if let Ok((key, value)) = config {
                    let _ = conn.execute(
                        "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
                        rusqlite::params![key, value],
                    );
                }
            }
        }

        self.emit(Event::Info(format!("Migrated {} records from v2 database: {}", migrated, v2_path)));
        Ok(())
    }

    // --- Health ---

    /// Compute the project health score.
    pub async fn health(&self) -> Result<crate::health::HealthScore> {
        let conn = self.conn()?;
        crate::health::compute_health(&conn)
    }

    // --- Metrics ---

    /// Compute a structured metrics report.
    pub async fn stats(&self) -> Result<crate::metrics::MetricsReport> {
        let conn = self.conn()?;
        crate::metrics::compute_metrics(&conn)
    }

    /// Compute daily trends over the last N days.
    pub async fn trends(&self, days: i64) -> Result<Vec<crate::metrics::TrendPoint>> {
        let conn = self.conn()?;
        crate::metrics::compute_trends(&conn, days)
    }

    /// Record a metric snapshot for a completed iteration.
    pub fn record_metric(
        &self,
        iteration_id: i64,
        task_id: i64,
        success: bool,
        duration_secs: f64,
        tokens_in: i64,
        tokens_out: i64,
        cost_usd: f64,
    ) -> Result<()> {
        let conn = self.conn()?;
        crate::metrics::record_iteration_metric(
            &conn, iteration_id, task_id, success, duration_secs, tokens_in, tokens_out, cost_usd,
        )
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
            approval_mode: ApprovalMode::Auto,
        };
        let result = Engine::open(config).await;
        assert!(result.is_err());
    }
}
