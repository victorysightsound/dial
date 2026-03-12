use crate::task::models::Task;

/// Events emitted by the DIAL engine during operations.
#[derive(Debug, Clone)]
pub enum Event {
    // --- Task Events ---
    /// A new task was added to the backlog.
    TaskAdded { id: i64, description: String, priority: i32 },
    /// A task was marked as done.
    TaskCompleted { id: i64 },
    /// A task was blocked with a reason.
    TaskBlocked { id: i64, reason: String },
    /// A task was cancelled.
    TaskCancelled { id: i64 },
    /// A previously blocked task was unblocked.
    TaskUnblocked { id: i64 },
    /// A dependency link was created between two tasks.
    TaskDependencyAdded { task_id: i64, depends_on_id: i64 },
    /// A dependency link was removed between two tasks.
    TaskDependencyRemoved { task_id: i64, depends_on_id: i64 },

    // --- Iteration Events ---
    /// An iteration started for a task.
    IterationStarted { iteration_id: i64, task: Task, attempt: i32, max_attempts: u32 },
    /// An iteration completed successfully, optionally with a git commit hash.
    IterationCompleted { iteration_id: i64, task_id: i64, commit_hash: Option<String> },
    /// An iteration failed with an error message.
    IterationFailed { iteration_id: i64, task_id: i64, error: String },

    // --- Validation Events ---
    /// Validation pipeline started for an iteration.
    ValidationStarted { iteration_id: i64 },
    /// All validation steps passed.
    ValidationPassed,
    /// Validation failed with captured error output.
    ValidationFailed { error_output: String },
    /// Build step started with the given command.
    BuildStarted { command: String },
    /// Build step passed.
    BuildPassed,
    /// Build step failed with captured output.
    BuildFailed { output: String },
    /// Test step started with the given command.
    TestStarted { command: String },
    /// Test step passed.
    TestPassed,
    /// Test step failed with captured output.
    TestFailed { output: String },

    // --- Pipeline Step Events ---
    /// A named pipeline step started.
    StepStarted { name: String, command: String, required: bool },
    /// A named pipeline step passed.
    StepPassed { name: String, duration_secs: f64 },
    /// A named pipeline step failed.
    StepFailed { name: String, required: bool, output: String, duration_secs: f64 },
    /// A named pipeline step was skipped (e.g., after a required step failed).
    StepSkipped { name: String, reason: String },

    // --- Learning Events ---
    /// A new learning was recorded.
    LearningAdded { id: i64, description: String, category: Option<String> },
    /// A learning was deleted.
    LearningDeleted { id: i64 },

    // --- Failure/Solution Events ---
    /// A failure was recorded and matched to a pattern.
    FailureRecorded { failure_id: i64, pattern_id: i64 },
    /// A trusted solution was found for a failure.
    SolutionFound { description: String, confidence: f64 },
    /// Solutions were auto-suggested for a recorded failure pattern.
    SolutionSuggested { failure_id: i64, solutions: Vec<(i64, String, f64)> },

    // --- Config Events ---
    /// A configuration key was set or updated.
    ConfigSet { key: String, value: String },

    // --- Approval Events ---
    /// An iteration is awaiting manual approval (Review/Manual mode).
    ApprovalRequired { iteration_id: i64, task_id: i64, diff_summary: String },
    /// A paused iteration was approved.
    Approved { iteration_id: i64 },
    /// A paused iteration was rejected with a reason.
    Rejected { iteration_id: i64, reason: String },

    // --- PRD Events ---
    /// Spec files were imported into prd.db.
    PrdImported { files: usize, sections: usize },
    /// A wizard phase started.
    WizardPhaseStarted { phase: u8, name: String },
    /// A wizard phase completed.
    WizardPhaseCompleted { phase: u8, name: String },
    /// The wizard finished, generating sections and tasks.
    WizardCompleted { sections_generated: usize, tasks_generated: usize },
    /// The wizard was paused (state saved for resume).
    WizardPaused { phase: u8 },
    /// The wizard was resumed from a saved state.
    WizardResumed { phase: u8 },
    /// A terminology entry was added.
    TermAdded { canonical: String, category: String },
    /// Task review phase completed with summary of changes.
    TaskReviewCompleted { tasks_kept: usize, tasks_added: usize, tasks_removed: usize },
    /// Build and test commands were configured.
    BuildTestConfigured { build_cmd: String, test_cmd: String, pipeline_steps: usize },
    /// Iteration mode was selected.
    IterationModeSet { mode: String },
    /// Project is ready for launch.
    LaunchReady { project_name: String, task_count: usize },

    // --- Checkpoint Events ---
    /// A checkpoint was created before task execution.
    CheckpointCreated { iteration_id: i64, checkpoint_id: String },
    /// A checkpoint was restored after validation failure.
    CheckpointRestored { iteration_id: i64 },
    /// A checkpoint was dropped after successful validation.
    CheckpointDropped { iteration_id: i64 },

    // --- General Events ---
    /// Informational message.
    Info(String),
    /// Warning message.
    Warning(String),
    /// Error message.
    Error(String),
}

/// Trait for handling events emitted by the engine.
pub trait EventHandler: Send + Sync {
    /// Called for each event emitted during engine operations.
    fn handle(&self, event: &Event);
}
