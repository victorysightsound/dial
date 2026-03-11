use async_trait::async_trait;
use dial_core::provider::{Provider, ProviderRequest, ProviderResponse};
use dial_core::errors::Result;
use std::time::Instant;
use tokio::process::Command;

/// Provider that shells out to a CLI AI tool (claude, codex, gemini).
pub struct CliPassthrough {
    /// CLI binary name (e.g., "claude", "codex")
    pub cli: String,
}

impl CliPassthrough {
    pub fn new(cli: &str) -> Self {
        Self { cli: cli.to_string() }
    }

    fn build_args(&self, request: &ProviderRequest) -> Vec<String> {
        match self.cli.as_str() {
            "claude" => vec![
                "-p".to_string(),
                request.prompt.clone(),
                "--output-format".to_string(),
                "text".to_string(),
            ],
            "codex" => vec![
                "--task".to_string(),
                request.prompt.clone(),
            ],
            _ => vec![request.prompt.clone()],
        }
    }
}

#[async_trait]
impl Provider for CliPassthrough {
    fn name(&self) -> &str {
        &self.cli
    }

    async fn execute(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        let start = Instant::now();
        let args = self.build_args(&request);

        let timeout = request.timeout_secs.unwrap_or(600);

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            Command::new(&self.cli)
                .args(&args)
                .current_dir(&request.work_dir)
                .output()
        )
        .await
        .map_err(|_| dial_core::errors::DialError::CommandTimeout(timeout))?
        .map_err(|e| dial_core::errors::DialError::CommandFailed(e.to_string()))?;

        let duration = start.elapsed().as_secs_f64();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let combined = if stderr.is_empty() {
            stdout
        } else {
            format!("{}\n{}", stdout, stderr)
        };

        Ok(ProviderResponse {
            output: combined,
            success: output.status.success(),
            exit_code: output.status.code(),
            usage: None, // CLI tools don't report token usage
            model: None,
            duration_secs: Some(duration),
        })
    }

    async fn is_available(&self) -> bool {
        Command::new("which")
            .arg(&self.cli)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
