//! DIAL (Deterministic Iterative Agent Loop) core library.
//!
//! Provides the engine, event system, provider abstraction, and persistence
//! layer for autonomous AI development with persistent memory.

/// Token budget estimation and context assembly.
pub mod budget;
/// Key-value configuration storage.
pub mod config;
/// SQLite database initialization, migrations, and access.
pub mod db;
/// Central engine coordinating tasks, iterations, and validation.
pub mod engine;
/// Error types used throughout the crate.
pub mod errors;
/// Event and handler types for engine lifecycle notifications.
pub mod event;
/// Failure pattern detection and solution tracking.
pub mod failure;
/// Git repository helpers (status, commit, revert, checkpoints).
pub mod git;
/// Project health score computation.
pub mod health;
/// Iteration lifecycle: context gathering, validation, commit.
pub mod iteration;
/// Learnings storage and retrieval.
pub mod learning;
/// Metrics computation, trend analysis, and reporting.
pub mod metrics;
/// Structured output formatting.
pub mod output;
/// PRD database, enhanced parser, templates, and wizard.
pub mod prd;
/// AI provider trait and request/response types.
pub mod provider;
/// Spec file indexing and full-text search.
pub mod spec;
/// Task management (add, list, block, dependencies).
pub mod task;
/// Build and test validation pipeline.
pub mod validation;

// Re-export commonly used items
pub use db::{get_current_phase, get_db, get_dial_dir, init_db, with_transaction, DEFAULT_PHASE};
pub use engine::{ApprovalMode, DryRunResult, Engine, EngineConfig, PatternInfo, PipelineStepConfig};
pub use health::{HealthScore, Trend};
pub use metrics::{MetricsReport, TrendPoint};
pub use errors::{DialError, Result};
pub use event::{Event, EventHandler};
pub use provider::{Provider, ProviderRequest, ProviderResponse, TokenUsage};

/// Current DIAL version.
pub const VERSION: &str = "4.1.0";
/// Maximum number of fix attempts before a task is abandoned.
pub const MAX_FIX_ATTEMPTS: u32 = 3;
/// Minimum confidence threshold for a solution to be considered trusted.
pub const TRUST_THRESHOLD: f64 = 0.6;
/// Confidence increase applied when a solution succeeds.
pub const TRUST_INCREMENT: f64 = 0.15;
/// Confidence decrease applied when a solution fails.
pub const TRUST_DECREMENT: f64 = 0.20;
/// Starting confidence score for newly recorded solutions.
pub const INITIAL_CONFIDENCE: f64 = 0.3;
/// Default command timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;
