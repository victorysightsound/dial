use crate::errors::Result;
use async_trait::async_trait;

/// Request to send to an AI provider.
#[derive(Debug, Clone)]
pub struct ProviderRequest {
    /// The prompt/instructions to send.
    pub prompt: String,
    /// Working directory for the provider to operate in.
    pub work_dir: String,
    /// Maximum tokens for the response (if applicable).
    pub max_tokens: Option<u32>,
    /// Model identifier (provider-specific).
    pub model: Option<String>,
    /// Timeout in seconds.
    pub timeout_secs: Option<u64>,
}

/// Response from an AI provider.
#[derive(Debug, Clone)]
pub struct ProviderResponse {
    /// The generated output/response text.
    pub output: String,
    /// Whether the provider completed successfully.
    pub success: bool,
    /// Exit code (for CLI-based providers).
    pub exit_code: Option<i32>,
    /// Token usage information.
    pub usage: Option<TokenUsage>,
    /// Model used.
    pub model: Option<String>,
    /// Duration in seconds.
    pub duration_secs: Option<f64>,
}

/// Token usage information from a provider call.
#[derive(Debug, Clone)]
pub struct TokenUsage {
    /// Input/prompt tokens.
    pub tokens_in: u64,
    /// Output/completion tokens.
    pub tokens_out: u64,
    /// Cost in USD (if known).
    pub cost_usd: Option<f64>,
}

/// Trait for AI providers that can execute tasks.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider name (e.g., "anthropic", "cli-passthrough").
    fn name(&self) -> &str;

    /// Execute a task with the given prompt.
    async fn execute(&self, request: ProviderRequest) -> Result<ProviderResponse>;

    /// Check if the provider is available/configured.
    async fn is_available(&self) -> bool;
}
