use dial_core::config::config_get;
use dial_core::errors::{DialError, Result};
use dial_core::Provider;
use dial_providers::{CliPassthrough, OpenAiCompatibleProvider};
use std::env;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardBackend {
    Codex,
    Claude,
    Copilot,
    Gemini,
    OpenAiCompatible,
}

impl WizardBackend {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "codex" => Some(Self::Codex),
            "claude" | "claude-code" | "claude_code" => Some(Self::Claude),
            "copilot" | "github-copilot" | "github-copilot-cli" => Some(Self::Copilot),
            "gemini" => Some(Self::Gemini),
            "openai" | "openai-compatible" | "api:openai-compatible" => {
                Some(Self::OpenAiCompatible)
            }
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Copilot => "copilot",
            Self::Gemini => "gemini",
            Self::OpenAiCompatible => "openai-compatible",
        }
    }

    pub fn cli_name(&self) -> Option<&'static str> {
        match self {
            Self::Codex => Some("codex"),
            Self::Claude => Some("claude"),
            Self::Copilot => Some("copilot"),
            Self::Gemini => Some("gemini"),
            Self::OpenAiCompatible => None,
        }
    }

    pub fn supported_values() -> &'static str {
        "codex, claude, copilot, gemini, openai-compatible"
    }
}

pub struct ResolvedWizardProvider {
    pub backend: WizardBackend,
    pub provider: Arc<dyn Provider>,
}

pub fn resolve_wizard_provider(
    explicit_backend: Option<&str>,
    explicit_model: Option<&str>,
) -> Result<ResolvedWizardProvider> {
    let backend = select_backend_from_environment(explicit_backend)?;
    let configured_model = non_empty(project_config_get("wizard_model")?);
    let model = explicit_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or(configured_model);

    let provider: Arc<dyn Provider> = match backend {
        WizardBackend::Codex
        | WizardBackend::Claude
        | WizardBackend::Copilot
        | WizardBackend::Gemini => {
            let cli = backend.cli_name().unwrap();
            if !CliPassthrough::command_available(cli) {
                return Err(DialError::UserError(format!(
                    "Wizard backend '{}' is not installed or not on PATH.",
                    backend.as_str()
                )));
            }
            let provider = if let Some(model) = model.as_deref() {
                CliPassthrough::new(cli).with_model(model)
            } else {
                CliPassthrough::new(cli)
            };
            Arc::new(provider)
        }
        WizardBackend::OpenAiCompatible => {
            let api_key = env::var("OPENAI_API_KEY").map_err(|_| {
                DialError::UserError(
                    "OpenAI-compatible wizard backend requires OPENAI_API_KEY.".to_string(),
                )
            })?;
            let base_url = non_empty(project_config_get("wizard_api_base_url")?).or_else(|| {
                env::var("OPENAI_BASE_URL")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            });
            let model = model
                .or_else(|| env::var("OPENAI_MODEL").ok().filter(|value| !value.trim().is_empty()))
                .ok_or_else(|| {
                    DialError::UserError(
                        "OpenAI-compatible wizard backend requires wizard_model, --wizard-model, or OPENAI_MODEL."
                            .to_string(),
                    )
                })?;
            Arc::new(OpenAiCompatibleProvider::new(api_key, base_url).with_model(&model))
        }
    };

    Ok(ResolvedWizardProvider { backend, provider })
}

fn select_backend_from_environment(explicit_backend: Option<&str>) -> Result<WizardBackend> {
    let explicit = explicit_backend.map(parse_backend).transpose()?;
    let configured = non_empty(project_config_get("wizard_backend")?)
        .map(|value| parse_backend(&value))
        .transpose()?;
    let configured_ai_cli =
        non_empty(project_config_get("ai_cli")?).and_then(|value| WizardBackend::parse(&value));
    let session_hint = detect_current_session_hint();
    let installed = installed_cli_backends();

    select_backend(
        explicit,
        configured,
        configured_ai_cli,
        session_hint,
        &installed,
    )
    .map_err(DialError::UserError)
}

fn project_config_get(key: &str) -> Result<Option<String>> {
    match config_get(key) {
        Ok(value) => Ok(value),
        Err(DialError::NotInitialized) => Ok(None),
        Err(error) => Err(error),
    }
}

fn parse_backend(value: &str) -> Result<WizardBackend> {
    WizardBackend::parse(value).ok_or_else(|| {
        DialError::InvalidConfig(format!(
            "Unknown wizard backend '{}'. Supported values: {}",
            value,
            WizardBackend::supported_values()
        ))
    })
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn detect_current_session_hint() -> Option<WizardBackend> {
    if env::var_os("CODEX_THREAD_ID").is_some() {
        return Some(WizardBackend::Codex);
    }
    if env::var_os("CLAUDECODE").is_some() {
        return Some(WizardBackend::Claude);
    }
    None
}

fn installed_cli_backends() -> Vec<WizardBackend> {
    [
        WizardBackend::Codex,
        WizardBackend::Claude,
        WizardBackend::Copilot,
        WizardBackend::Gemini,
    ]
    .into_iter()
    .filter(|backend| {
        backend
            .cli_name()
            .map(CliPassthrough::command_available)
            .unwrap_or(false)
    })
    .collect()
}

fn select_backend(
    explicit: Option<WizardBackend>,
    configured: Option<WizardBackend>,
    configured_ai_cli: Option<WizardBackend>,
    session_hint: Option<WizardBackend>,
    installed: &[WizardBackend],
) -> std::result::Result<WizardBackend, String> {
    if let Some(backend) = explicit {
        return Ok(backend);
    }
    if let Some(backend) = configured {
        return Ok(backend);
    }
    if let Some(backend) = configured_ai_cli {
        return Ok(backend);
    }
    if let Some(backend) = session_hint {
        return Ok(backend);
    }

    match installed {
        [backend] => Ok(*backend),
        [] => Err(format!(
            "No wizard backend is configured. Pass --wizard-backend, set wizard_backend, or install one of: {}.",
            WizardBackend::supported_values()
        )),
        _ => {
            let names = installed
                .iter()
                .map(WizardBackend::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "Multiple wizard backends are available ({}). Pass --wizard-backend or set wizard_backend explicitly.",
                names
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    fn cwd_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn explicit_backend_wins() {
        let resolved = select_backend(
            Some(WizardBackend::Copilot),
            Some(WizardBackend::Claude),
            Some(WizardBackend::Gemini),
            Some(WizardBackend::Codex),
            &[WizardBackend::Codex, WizardBackend::Claude],
        )
        .unwrap();
        assert_eq!(resolved, WizardBackend::Copilot);
    }

    #[test]
    fn configured_backend_wins_over_session_hint() {
        let resolved = select_backend(
            None,
            Some(WizardBackend::Claude),
            None,
            Some(WizardBackend::Codex),
            &[WizardBackend::Codex, WizardBackend::Claude],
        )
        .unwrap();
        assert_eq!(resolved, WizardBackend::Claude);
    }

    #[test]
    fn ai_cli_fallback_is_used_when_wizard_backend_missing() {
        let resolved = select_backend(
            None,
            None,
            Some(WizardBackend::Gemini),
            Some(WizardBackend::Codex),
            &[WizardBackend::Codex, WizardBackend::Gemini],
        )
        .unwrap();
        assert_eq!(resolved, WizardBackend::Gemini);
    }

    #[test]
    fn session_hint_breaks_cli_ambiguity() {
        let resolved = select_backend(
            None,
            None,
            None,
            Some(WizardBackend::Codex),
            &[
                WizardBackend::Codex,
                WizardBackend::Claude,
                WizardBackend::Copilot,
            ],
        )
        .unwrap();
        assert_eq!(resolved, WizardBackend::Codex);
    }

    #[test]
    fn single_installed_cli_is_selected() {
        let resolved = select_backend(None, None, None, None, &[WizardBackend::Copilot]).unwrap();
        assert_eq!(resolved, WizardBackend::Copilot);
    }

    #[test]
    fn ambiguity_requires_explicit_choice() {
        let error = select_backend(
            None,
            None,
            None,
            None,
            &[WizardBackend::Codex, WizardBackend::Claude],
        )
        .unwrap_err();
        assert!(error.contains("Multiple wizard backends are available"));
    }

    #[test]
    fn project_config_get_returns_none_before_init() {
        let _guard = cwd_lock().lock().unwrap();
        let original_dir = env::current_dir().unwrap();
        let temp = tempdir().unwrap();
        env::set_current_dir(temp.path()).unwrap();

        let value = project_config_get("wizard_backend").unwrap();
        assert_eq!(value, None);

        env::set_current_dir(original_dir).unwrap();
    }
}
