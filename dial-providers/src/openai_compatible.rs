use async_trait::async_trait;
use dial_core::errors::{DialError, Result};
use dial_core::provider::{Provider, ProviderRequest, ProviderResponse, TokenUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::time::Instant;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Provider for OpenAI-compatible chat completion APIs.
pub struct OpenAiCompatibleProvider {
    api_key: String,
    base_url: String,
    model: Option<String>,
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            api_key,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            model: None,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{}/chat/completions", base)
        }
    }

    fn extract_content(content: &JsonValue) -> Option<String> {
        match content {
            JsonValue::String(text) => Some(text.clone()),
            JsonValue::Array(parts) => {
                let text = parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
                    .collect::<Vec<_>>()
                    .join("");
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
            _ => None,
        }
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessageRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct ChatMessageRequest {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    model: Option<String>,
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Deserialize)]
struct ChatMessageResponse {
    content: JsonValue,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    async fn execute(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        let model = request
            .model
            .clone()
            .or_else(|| self.model.clone())
            .ok_or_else(|| {
                DialError::InvalidConfig(
                    "OpenAI-compatible wizard backend requires a model. Set wizard_model, pass --wizard-model, or set OPENAI_MODEL.".to_string(),
                )
            })?;
        let start = Instant::now();

        let payload = ChatCompletionRequest {
            model: model.clone(),
            messages: vec![ChatMessageRequest {
                role: "user".to_string(),
                content: request.prompt,
            }],
            max_tokens: request.max_tokens,
        };

        let response = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|err| {
                DialError::CommandFailed(format!("OpenAI-compatible API error: {}", err))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(DialError::CommandFailed(format!(
                "OpenAI-compatible API {}: {}",
                status, body
            )));
        }

        let data: ChatCompletionResponse = response.json().await.map_err(|err| {
            DialError::CommandFailed(format!("Invalid OpenAI-compatible response: {}", err))
        })?;

        let output = data
            .choices
            .first()
            .and_then(|choice| Self::extract_content(&choice.message.content))
            .ok_or_else(|| {
                DialError::CommandFailed(
                    "OpenAI-compatible API returned no text content in the first choice."
                        .to_string(),
                )
            })?;

        Ok(ProviderResponse {
            output,
            success: true,
            exit_code: None,
            usage: data.usage.map(|usage| TokenUsage {
                tokens_in: usage.prompt_tokens,
                tokens_out: usage.completion_tokens,
                cost_usd: None,
            }),
            model: data.model.or(Some(model)),
            duration_secs: Some(start.elapsed().as_secs_f64()),
        })
    }

    async fn is_available(&self) -> bool {
        !self.api_key.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_chat_completions_path_to_base_url() {
        let provider = OpenAiCompatibleProvider::new(
            "token".to_string(),
            Some("https://example.com/v1".to_string()),
        );
        assert_eq!(
            provider.endpoint(),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn preserves_full_chat_completion_endpoint() {
        let provider = OpenAiCompatibleProvider::new(
            "token".to_string(),
            Some("https://example.com/v1/chat/completions".to_string()),
        );
        assert_eq!(
            provider.endpoint(),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn extracts_string_content() {
        let content = JsonValue::String("hello".to_string());
        assert_eq!(
            OpenAiCompatibleProvider::extract_content(&content),
            Some("hello".to_string())
        );
    }

    #[test]
    fn extracts_text_from_array_content() {
        let content = serde_json::json!([
            {"type": "output_text", "text": "hello"},
            {"type": "output_text", "text": " world"}
        ]);
        assert_eq!(
            OpenAiCompatibleProvider::extract_content(&content),
            Some("hello world".to_string())
        );
    }
}
