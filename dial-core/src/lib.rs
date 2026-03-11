pub mod config;
pub mod db;
pub mod engine;
pub mod errors;
pub mod event;
pub mod failure;
pub mod git;
pub mod iteration;
pub mod learning;
pub mod output;
pub mod provider;
pub mod spec;
pub mod task;
pub mod validation;

// Re-export commonly used items
pub use db::{get_current_phase, get_db, get_dial_dir, init_db, DEFAULT_PHASE};
pub use engine::{Engine, EngineConfig, PipelineStepConfig};
pub use errors::{DialError, Result};
pub use event::{Event, EventHandler};
pub use provider::{Provider, ProviderRequest, ProviderResponse, TokenUsage};

// Constants
pub const VERSION: &str = "3.0.0";
pub const MAX_FIX_ATTEMPTS: u32 = 3;
pub const TRUST_THRESHOLD: f64 = 0.6;
pub const TRUST_INCREMENT: f64 = 0.15;
pub const TRUST_DECREMENT: f64 = 0.20;
pub const INITIAL_CONFIDENCE: f64 = 0.3;
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;
