use async_trait::async_trait;
use dial_core::errors::Result;
use dial_core::provider::{Provider, ProviderRequest, ProviderResponse};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::process::Stdio;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Provider that shells out to a CLI AI tool (claude, codex, gemini).
pub struct CliPassthrough {
    /// CLI binary name (e.g., "claude", "codex")
    pub cli: String,
    /// Optional default model for this CLI.
    pub model: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedCliCommand {
    program: OsString,
    prefix_args: Vec<OsString>,
}

impl ResolvedCliCommand {
    fn direct(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            prefix_args: Vec::new(),
        }
    }

    fn from_path(path: PathBuf) -> Self {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());

        match extension.as_deref() {
            Some("cmd") | Some("bat") => Self {
                program: OsString::from("cmd.exe"),
                prefix_args: vec![OsString::from("/C"), path.into_os_string()],
            },
            Some("ps1") => Self {
                program: OsString::from("powershell.exe"),
                prefix_args: vec![
                    OsString::from("-NoProfile"),
                    OsString::from("-NonInteractive"),
                    OsString::from("-ExecutionPolicy"),
                    OsString::from("Bypass"),
                    OsString::from("-File"),
                    path.into_os_string(),
                ],
            },
            _ => Self::direct(path.into_os_string()),
        }
    }

    fn std_command(&self) -> StdCommand {
        let mut command = StdCommand::new(&self.program);
        command.args(&self.prefix_args);
        command
    }

    fn tokio_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.prefix_args);
        command
    }
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
        let Some(command) = Self::resolve_command(cli) else {
            return false;
        };

        command
            .std_command()
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn resolve_command(cli: &str) -> Option<ResolvedCliCommand> {
        if cfg!(windows) {
            let search_paths = std::env::var_os("PATH")
                .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
                .unwrap_or_default();
            let path_exts = Self::windows_path_exts(std::env::var_os("PATHEXT"));
            Self::resolve_windows_command(cli, &search_paths, &path_exts)
        } else {
            Some(ResolvedCliCommand::direct(cli))
        }
    }

    fn windows_path_exts(path_ext: Option<OsString>) -> Vec<OsString> {
        let extensions = path_ext
            .as_deref()
            .map(std::env::split_paths)
            .into_iter()
            .flatten()
            .filter_map(|value| value.into_os_string().into_string().ok())
            .flat_map(|value| value.split(';').map(str::to_string).collect::<Vec<_>>())
            .filter_map(|value| {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_ascii_lowercase())
                }
            })
            .map(OsString::from)
            .collect::<Vec<_>>();

        Self::prefer_windows_extensions(&extensions)
    }

    fn prefer_windows_extensions(path_exts: &[OsString]) -> Vec<OsString> {
        let mut ordered = Vec::new();

        for preferred in [".exe", ".ps1", ".cmd", ".bat", ".com"] {
            if path_exts
                .iter()
                .any(|value| value.to_string_lossy().eq_ignore_ascii_case(preferred))
            {
                ordered.push(OsString::from(preferred));
            }
        }

        for extension in path_exts {
            if !ordered.iter().any(|value| {
                value
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&extension.to_string_lossy())
            }) {
                ordered.push(extension.clone());
            }
        }

        if ordered.is_empty() {
            vec![
                OsString::from(".exe"),
                OsString::from(".ps1"),
                OsString::from(".cmd"),
                OsString::from(".bat"),
                OsString::from(".com"),
            ]
        } else {
            ordered
        }
    }

    fn resolve_windows_command(
        cli: &str,
        search_paths: &[PathBuf],
        path_exts: &[OsString],
    ) -> Option<ResolvedCliCommand> {
        let cli_path = Path::new(cli);
        let has_explicit_path = cli_path.components().count() > 1 || cli_path.is_absolute();

        if has_explicit_path {
            return Self::resolve_windows_candidate(cli_path, path_exts);
        }

        for directory in search_paths {
            let candidate = directory.join(cli);
            if let Some(command) = Self::resolve_windows_candidate(&candidate, path_exts) {
                return Some(command);
            }
        }

        None
    }

    fn resolve_windows_candidate(
        path: &Path,
        path_exts: &[OsString],
    ) -> Option<ResolvedCliCommand> {
        if path.extension().is_some() {
            return path
                .is_file()
                .then(|| ResolvedCliCommand::from_path(path.to_path_buf()));
        }

        for extension in path_exts {
            let ext = extension.to_string_lossy();
            let candidate = if ext.starts_with('.') {
                path.with_extension(ext.trim_start_matches('.'))
            } else {
                path.with_extension(ext.as_ref())
            };
            if candidate.is_file() {
                return Some(ResolvedCliCommand::from_path(candidate));
            }
        }

        path.is_file()
            .then(|| ResolvedCliCommand::from_path(path.to_path_buf()))
    }

    fn selected_model<'a>(&'a self, request: &'a ProviderRequest) -> Option<&'a str> {
        request.model.as_deref().or(self.model.as_deref())
    }

    fn build_args(
        &self,
        request: &ProviderRequest,
        output_file: Option<&str>,
        schema_file: Option<&str>,
    ) -> Vec<String> {
        let model = self.selected_model(request);

        match self.cli.as_str() {
            "claude" => {
                let mut args = vec![
                    "-p".to_string(),
                    request.prompt.clone(),
                    "--output-format".to_string(),
                    "text".to_string(),
                ];
                if let Some(schema) = request.output_schema.as_deref() {
                    args.push("--json-schema".to_string());
                    args.push(schema.to_string());
                }
                if let Some(value) = model {
                    args.push("--model".to_string());
                    args.push(value.to_string());
                }
                args
            }
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
                if let Some(file) = schema_file {
                    args.push("--output-schema".to_string());
                    args.push(file.to_string());
                }
                if let Some(value) = model {
                    args.push("--model".to_string());
                    args.push(value.to_string());
                }
                if self.uses_stdin_prompt() {
                    args.push("-".to_string());
                } else {
                    args.push(request.prompt.clone());
                }
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

    fn temp_schema_file(&self, schema: Option<&str>) -> Option<String> {
        if self.cli != "codex" {
            return None;
        }

        let schema = schema?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let filename = format!(
            "dial-codex-schema-{}-{}.json",
            std::process::id(),
            timestamp
        );
        let path = std::env::temp_dir().join(filename);
        fs::write(&path, schema).ok()?;
        Some(path.to_string_lossy().to_string())
    }

    fn uses_stdin_prompt(&self) -> bool {
        matches!(self.cli.as_str(), "codex")
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
        let schema_file = self.temp_schema_file(request.output_schema.as_deref());
        let args = self.build_args(&request, output_file.as_deref(), schema_file.as_deref());
        let command = Self::resolve_command(&self.cli).ok_or_else(|| {
            dial_core::errors::DialError::CommandFailed(format!(
                "Wizard backend '{}' is not installed or not on PATH.",
                self.cli
            ))
        })?;
        let uses_stdin_prompt = self.uses_stdin_prompt();
        let stdin_prompt = uses_stdin_prompt.then(|| request.prompt.clone());

        let timeout = request.timeout_secs.unwrap_or(600);

        let output = tokio::time::timeout(std::time::Duration::from_secs(timeout), {
            let mut process = command.tokio_command();
            process.args(&args).current_dir(&request.work_dir);
            if uses_stdin_prompt {
                process.stdin(Stdio::piped());
            }
            async move {
                if uses_stdin_prompt {
                    let mut child = process.spawn()?;
                    if let Some(mut stdin) = child.stdin.take() {
                        if let Some(prompt) = stdin_prompt.as_deref() {
                            stdin.write_all(prompt.as_bytes()).await?;
                        }
                    }
                    child.wait_with_output().await
                } else {
                    process.output().await
                }
            }
        })
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
        if let Some(path) = schema_file {
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
            output_schema: None,
            max_tokens: None,
            model: None,
            timeout_secs: Some(30),
        }
    }

    #[test]
    fn codex_uses_exec_and_output_file() {
        let provider = CliPassthrough::new("codex");
        let args = provider.build_args(&request(), Some("/tmp/out.txt"), None);
        assert_eq!(args[0], "exec");
        assert!(args.contains(&"--skip-git-repo-check".to_string()));
        assert!(args.contains(&"web_search=\"disabled\"".to_string()));
        assert!(args.contains(&"model_reasoning_effort=\"low\"".to_string()));
        assert!(args.contains(&"model_verbosity=\"low\"".to_string()));
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"/tmp/out.txt".to_string()));
        assert!(args.contains(&"-".to_string()));
    }

    #[test]
    fn claude_uses_print_mode() {
        let provider = CliPassthrough::new("claude");
        let args = provider.build_args(&request(), None, None);
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"--output-format".to_string()));
        assert!(args.contains(&"text".to_string()));
    }

    #[test]
    fn gemini_uses_headless_text_output() {
        let provider = CliPassthrough::new("gemini");
        let args = provider.build_args(&request(), None, None);
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"text".to_string()));
    }

    #[test]
    fn copilot_uses_noninteractive_silent_mode() {
        let provider = CliPassthrough::new("copilot");
        let args = provider.build_args(&request(), None, None);
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

    #[test]
    fn claude_passes_inline_json_schema() {
        let provider = CliPassthrough::new("claude");
        let mut request = request();
        request.output_schema = Some(r#"{"type":"object"}"#.to_string());

        let args = provider.build_args(&request, None, None);
        assert!(args.contains(&"--json-schema".to_string()));
        assert!(args.contains(&r#"{"type":"object"}"#.to_string()));
    }

    #[test]
    fn codex_passes_output_schema_file() {
        let provider = CliPassthrough::new("codex");
        let args = provider.build_args(&request(), Some("/tmp/out.txt"), Some("/tmp/schema.json"));
        assert!(args.contains(&"--output-schema".to_string()));
        assert!(args.contains(&"/tmp/schema.json".to_string()));
    }

    #[test]
    fn windows_resolution_wraps_cmd_shims() {
        let temp = tempfile::tempdir().unwrap();
        let shim = temp.path().join("codex.cmd");
        fs::write(&shim, "@echo off\r\n").unwrap();

        let resolved = CliPassthrough::resolve_windows_command(
            "codex",
            &[temp.path().to_path_buf()],
            &[OsString::from(".cmd"), OsString::from(".exe")],
        )
        .unwrap();

        assert_eq!(resolved.program, OsString::from("cmd.exe"));
        assert_eq!(
            resolved.prefix_args,
            vec![OsString::from("/C"), shim.into_os_string()]
        );
    }

    #[test]
    fn windows_resolution_prefers_powershell_shim_over_extensionless_file() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("codex"), "").unwrap();
        let ps1 = temp.path().join("codex.ps1");
        fs::write(&ps1, "Write-Output 'ok'\r\n").unwrap();
        let cmd = temp.path().join("codex.cmd");
        fs::write(&cmd, "@echo off\r\n").unwrap();

        let resolved = CliPassthrough::resolve_windows_command(
            "codex",
            &[temp.path().to_path_buf()],
            &[OsString::from(".ps1"), OsString::from(".cmd")],
        )
        .unwrap();

        assert_eq!(resolved.program, OsString::from("powershell.exe"));
        assert_eq!(
            resolved.prefix_args,
            vec![
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-ExecutionPolicy"),
                OsString::from("Bypass"),
                OsString::from("-File"),
                ps1.into_os_string(),
            ]
        );
    }

    #[test]
    fn windows_resolution_prefers_real_executables_before_scripts() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("claude.exe");
        let cmd = temp.path().join("claude.cmd");
        fs::write(&exe, "").unwrap();
        fs::write(&cmd, "@echo off\r\n").unwrap();

        let resolved = CliPassthrough::resolve_windows_command(
            "claude",
            &[temp.path().to_path_buf()],
            &[OsString::from(".exe"), OsString::from(".cmd")],
        )
        .unwrap();

        assert_eq!(resolved.program, exe.into_os_string());
        assert!(resolved.prefix_args.is_empty());
    }

    #[test]
    fn windows_extension_preference_promotes_ps1_before_cmd() {
        let ordered = CliPassthrough::prefer_windows_extensions(&[
            OsString::from(".com"),
            OsString::from(".cmd"),
            OsString::from(".ps1"),
            OsString::from(".exe"),
        ]);

        assert_eq!(
            ordered,
            vec![
                OsString::from(".exe"),
                OsString::from(".ps1"),
                OsString::from(".cmd"),
                OsString::from(".com"),
            ]
        );
    }
}
