use thiserror::Error;

#[derive(Error, Debug)]
pub enum DialError {
    #[error("DIAL not initialized. Run 'dial init' first.")]
    NotInitialized,

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Phase '{0}' not found")]
    PhaseNotFound(String),

    #[error("Task #{0} not found")]
    TaskNotFound(i64),

    #[error("Spec section #{0} not found")]
    SpecSectionNotFound(i64),

    #[error("Learning #{0} not found")]
    LearningNotFound(i64),

    #[error("No iteration in progress")]
    NoIterationInProgress,

    #[error("No pending tasks")]
    NoPendingTasks,

    #[error("Task #{0} has exceeded max fix attempts")]
    MaxAttemptsExceeded(i64),

    #[error("Not a git repository")]
    NotGitRepo,

    #[error("Git operation failed: {0}")]
    GitError(String),

    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error("Command timed out after {0} seconds")]
    CommandTimeout(u64),

    #[error("Invalid config key: {0}")]
    InvalidConfigKey(String),

    #[error("Specs directory '{0}' not found")]
    SpecsDirNotFound(String),

    #[error("{0}")]
    UserError(String),
}

pub type Result<T> = std::result::Result<T, DialError>;
