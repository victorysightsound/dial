use crate::task::models::Task;

/// Events emitted by the DIAL engine during operations.
#[derive(Debug, Clone)]
pub enum Event {
    // --- Task Events ---
    TaskAdded { id: i64, description: String, priority: i32 },
    TaskCompleted { id: i64 },
    TaskBlocked { id: i64, reason: String },
    TaskCancelled { id: i64 },
    TaskUnblocked { id: i64 },
    TaskDependencyAdded { task_id: i64, depends_on_id: i64 },
    TaskDependencyRemoved { task_id: i64, depends_on_id: i64 },

    // --- Iteration Events ---
    IterationStarted { iteration_id: i64, task: Task, attempt: i32, max_attempts: u32 },
    IterationCompleted { iteration_id: i64, task_id: i64, commit_hash: Option<String> },
    IterationFailed { iteration_id: i64, task_id: i64, error: String },

    // --- Validation Events ---
    ValidationStarted { iteration_id: i64 },
    ValidationPassed,
    ValidationFailed { error_output: String },
    BuildStarted { command: String },
    BuildPassed,
    BuildFailed { output: String },
    TestStarted { command: String },
    TestPassed,
    TestFailed { output: String },

    // --- Pipeline Step Events ---
    StepStarted { name: String, command: String, required: bool },
    StepPassed { name: String, duration_secs: f64 },
    StepFailed { name: String, required: bool, output: String, duration_secs: f64 },
    StepSkipped { name: String, reason: String },

    // --- Learning Events ---
    LearningAdded { id: i64, description: String, category: Option<String> },
    LearningDeleted { id: i64 },

    // --- Failure/Solution Events ---
    FailureRecorded { failure_id: i64, pattern_id: i64 },
    SolutionFound { description: String, confidence: f64 },

    // --- Config Events ---
    ConfigSet { key: String, value: String },

    // --- General Events ---
    Info(String),
    Warning(String),
    Error(String),
}

/// Trait for handling events emitted by the engine.
pub trait EventHandler: Send + Sync {
    fn handle(&self, event: &Event);
}
