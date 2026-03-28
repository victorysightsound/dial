use async_trait::async_trait;
use dial_core::errors::Result;
use dial_core::provider::{Provider, ProviderRequest, ProviderResponse};
use std::fs;
use std::process::Command as StdCommand;
use std::process::Stdio;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::process::Command;

/// Provider that shells out to a CLI AI tool (claude, codex, gemini).
pub struct CliPassthrough {
    /// CLI binary name (e.g., "claude", "codex")
    pub cli: String,
    /// Optional default model for this CLI.
    pub model: Option<String>,
}

impl CliPassthrough {
    pub fn new(cli: &str) -> Self {
        Self {
            cli: cli.to_string(),
            model: None,
        }
    }

    pub fn with_model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    pub fn command_available(cli: &str) -> bool {
        StdCommand::new(cli)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn selected_model<'a>(&'a self, request: &'a ProviderRequest) -> Option<&'a str> {
        request.model.as_deref().or(self.model.as_deref())
    }

    fn build_args(&self, request: &ProviderRequest, output_file: Option<&str>) -> Vec<String> {
        let model = self.selected_model(request);

        match self.cli.as_str() {
            "claude" => vec![
                "-p".to_string(),
                request.prompt.clone(),
                "--output-format".to_string(),
                "text".to_string(),
            ]
            .into_iter()
            .chain(
                model
                    .map(|value| vec!["--model".to_string(), value.to_string()])
                    .unwrap_or_default(),
            )
            .collect(),
            "codex" => {
                let mut args = vec![
                    "exec".to_string(),
                    "--skip-git-repo-check".to_string(),
                    "--ephemeral".to_string(),
                    "--color".to_string(),
                    "never".to_string(),
                    "-c".to_string(),
                    "web_search=\"disabled\"".to_string(),
                    "-c".to_string(),
                    "model_reasoning_effort=\"low\"".to_string(),
                    "-c".to_string(),
                    "model_verbosity=\"low\"".to_string(),
                ];
                if let Some(file) = output_file {
                    args.push("-o".to_string());
                    args.push(file.to_string());
                }
                if let Some(value) = model {
                    args.push("--model".to_string());
                    args.push(value.to_string());
                }
                args.push(request.prompt.clone());
                args
            }
            "gemini" => {
                let mut args = vec![
                    "-p".to_string(),
                    request.prompt.clone(),
                    "-o".to_string(),
                    "text".to_string(),
                ];
                if let Some(value) = model {
                    args.push("-m".to_string());
                    args.push(value.to_string());
                }
                args
            }
            "copilot" => {
                let mut args = vec![
                    "-p".to_string(),
                    request.prompt.clone(),
                    "-s".to_string(),
                    "--reasoning-effort".to_string(),
                    "low".to_string(),
                    "--stream".to_string(),
                    "off".to_string(),
                    "--disable-builtin-mcps".to_string(),
                    "--no-ask-user".to_string(),
                    "--no-custom-instructions".to_string(),
                    "--allow-all-tools".to_string(),
                    "--allow-all-paths".to_string(),
                    "--allow-all-urls".to_string(),
                ];
                if let Some(value) = model {
                    args.push("--model".to_string());
                    args.push(value.to_string());
                }
                args
            }
            _ => vec![request.prompt.clone()],
        }
    }

    fn temp_output_file(&self) -> Option<String> {
        if self.cli != "codex" {
            return None;
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let filename = format!("dial-codex-{}-{}.txt", std::process::id(), timestamp);
        Some(
            std::env::temp_dir()
                .join(filename)
                .to_string_lossy()
                .to_string(),
        )
    }
}

#[async_trait]
impl Provider for CliPassthrough {
    fn name(&self) -> &str {
        &self.cli
    }

    async fn execute(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        let start = Instant::now();
        let output_file = self.temp_output_file();
        let args = self.build_args(&request, output_file.as_deref());

        let timeout = request.timeout_secs.unwrap_or(600);

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            Command::new(&self.cli)
                .args(&args)
                .current_dir(&request.work_dir)
                .output(),
        )
        .await
        .map_err(|_| dial_core::errors::DialError::CommandTimeout(timeout))?
        .map_err(|e| dial_core::errors::DialError::CommandFailed(e.to_string()))?;

        let duration = start.elapsed().as_secs_f64();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let combined = if stderr.trim().is_empty() {
            stdout
        } else {
            format!("{}\n{}", stdout, stderr)
        };
        let file_output = output_file
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok());
        if let Some(path) = output_file {
            let _ = fs::remove_file(path);
        }
        let final_output = file_output
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(combined);

        Ok(ProviderResponse {
            output: final_output,
            success: output.status.success(),
            exit_code: output.status.code(),
            usage: None, // CLI tools don't report token usage
            model: self.selected_model(&request).map(str::to_string),
            duration_secs: Some(duration),
        })
    }

    async fn is_available(&self) -> bool {
        Self::command_available(&self.cli)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ProviderRequest {
        ProviderRequest {
            prompt: "Return JSON".to_string(),
            work_dir: ".".to_string(),
            max_tokens: None,
            model: None,
            timeout_secs: Some(30),
        }
    }

    #[test]
    fn codex_uses_exec_and_output_file() {
        let provider = CliPassthrough::new("codex");
        let args = provider.build_args(&request(), Some("/tmp/out.txt"));
        assert_eq!(args[0], "exec");
        assert!(args.contains(&"--skip-git-repo-check".to_string()));
        assert!(args.contains(&"web_search=\"disabled\"".to_string()));
        assert!(args.contains(&"model_reasoning_effort=\"low\"".to_string()));
        assert!(args.contains(&"model_verbosity=\"low\"".to_string()));
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"/tmp/out.txt".to_string()));
    }

    #[test]
    fn claude_uses_print_mode() {
        let provider = CliPassthrough::new("claude");
        let args = provider.build_args(&request(), None);
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"--output-format".to_string()));
        assert!(args.contains(&"text".to_string()));
    }

    #[test]
    fn gemini_uses_headless_text_output() {
        let provider = CliPassthrough::new("gemini");
        let args = provider.build_args(&request(), None);
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"text".to_string()));
    }

    #[test]
    fn copilot_uses_noninteractive_silent_mode() {
        let provider = CliPassthrough::new("copilot");
        let args = provider.build_args(&request(), None);
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"-s".to_string()));
        assert!(args.contains(&"--reasoning-effort".to_string()));
        assert!(args.contains(&"low".to_string()));
        assert!(args.contains(&"--stream".to_string()));
        assert!(args.contains(&"off".to_string()));
        assert!(args.contains(&"--disable-builtin-mcps".to_string()));
        assert!(args.contains(&"--no-ask-user".to_string()));
        assert!(args.contains(&"--no-custom-instructions".to_string()));
        assert!(args.contains(&"--allow-all-tools".to_string()));
    }
}
