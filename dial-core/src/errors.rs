use thiserror::Error;

/// Errors that can occur during DIAL operations.
#[derive(Error, Debug)]
pub enum DialError {
    /// No `.dial` directory found in the working directory.
    #[error("DIAL not initialized. Run 'dial init' first.")]
    NotInitialized,

    /// SQLite database error.
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// Filesystem I/O error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// The requested phase does not exist in the database.
    #[error("Phase '{0}' not found")]
    PhaseNotFound(String),

    /// No task exists with the given ID.
    #[error("Task #{0} not found")]
    TaskNotFound(i64),

    /// No spec section exists with the given ID.
    #[error("Spec section #{0} not found")]
    SpecSectionNotFound(i64),

    /// No learning exists with the given ID.
    #[error("Learning #{0} not found")]
    LearningNotFound(i64),

    /// An operation required an active iteration but none is in progress.
    #[error("No iteration in progress")]
    NoIterationInProgress,

    /// No pending tasks remain in the backlog.
    #[error("No pending tasks")]
    NoPendingTasks,

    /// A task has exceeded the maximum number of fix attempts.
    #[error("Task #{0} has exceeded max fix attempts")]
    MaxAttemptsExceeded(i64),

    /// The working directory is not a git repository.
    #[error("Not a git repository")]
    NotGitRepo,

    /// A git operation (commit, revert, etc.) failed.
    #[error("Git operation failed: {0}")]
    GitError(String),

    /// An external command failed to execute.
    #[error("Command failed: {0}")]
    CommandFailed(String),

    /// An external command exceeded its timeout.
    #[error("Command timed out after {0} seconds")]
    CommandTimeout(u64),

    /// An unrecognized configuration key was provided.
    #[error("Invalid config key: {0}")]
    InvalidConfigKey(String),

    /// A configuration value is invalid or malformed.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// The specified specs directory does not exist.
    #[error("Specs directory '{0}' not found")]
    SpecsDirNotFound(String),

    /// A general user-facing error with a custom message.
    #[error("{0}")]
    UserError(String),

    /// A PRD section with the given section_id was not found.
    #[error("PRD section '{0}' not found")]
    PrdSectionNotFound(String),

    /// An error occurred during the spec wizard process.
    #[error("Wizard error: {0}")]
    WizardError(String),

    /// The requested template does not exist.
    #[error("Template '{0}' not found. Available: spec, architecture, api, mvp")]
    TemplateNotFound(String),

    /// A provider is required but none is configured.
    #[error("Provider required for wizard. Set ANTHROPIC_API_KEY or use --cli flag.")]
    ProviderRequired,

    /// Adding a dependency would create a cycle in the task graph.
    #[error("Cyclic dependency detected: task #{0} would create a cycle")]
    CyclicDependency(i64),

    /// A task cannot depend on itself.
    #[error("Self-dependency: task #{0} cannot depend on itself")]
    SelfDependency(i64),
}

/// Convenience alias for `std::result::Result<T, DialError>`.
pub type Result<T> = std::result::Result<T, DialError>;
