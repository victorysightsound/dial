use async_trait::async_trait;
use dial_core::errors::{DialError, Result};
use dial_core::provider::{Provider, ProviderRequest, ProviderResponse, TokenUsage};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Instant;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";
const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Provider for Anthropic's Claude API.
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            model: DEFAULT_MODEL.to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    /// Estimate cost in USD based on model and token counts.
    fn estimate_cost(model: &str, tokens_in: u64, tokens_out: u64) -> f64 {
        let (cost_per_1k_in, cost_per_1k_out) = match model {
            m if m.contains("opus") => (0.015, 0.075),
            m if m.contains("sonnet") => (0.003, 0.015),
            m if m.contains("haiku") => (0.00025, 0.00125),
            _ => (0.003, 0.015), // default to sonnet pricing
        };
        (tokens_in as f64 / 1000.0) * cost_per_1k_in
            + (tokens_out as f64 / 1000.0) * cost_per_1k_out
    }
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
    stream: bool,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ApiResponse {
    content: Vec<ContentBlock>,
    usage: ApiUsage,
    model: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ContentBlock {
    text: Option<String>,
}

#[derive(Deserialize)]
struct ApiUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Deserialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<StreamDelta>,
    message: Option<ApiResponse>,
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct StreamDelta {
    text: Option<String>,
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn execute(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        let start = Instant::now();
        let model = request.model.as_deref().unwrap_or(&self.model);
        let max_tokens = request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);

        let api_request = ApiRequest {
            model: model.to_string(),
            max_tokens,
            messages: vec![Message {
                role: "user".to_string(),
                content: request.prompt,
            }],
            stream: true,
        };

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&api_request)
            .send()
            .await
            .map_err(|e| DialError::CommandFailed(format!("Anthropic API error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(DialError::CommandFailed(format!(
                "Anthropic API {} : {}",
                status, body
            )));
        }

        // Stream the response
        let mut output = String::new();
        let mut tokens_in: u64 = 0;
        let mut tokens_out: u64 = 0;
        let mut response_model = model.to_string();

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|e| DialError::CommandFailed(format!("Stream error: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process SSE events from buffer
            while let Some(event_end) = buffer.find("\n\n") {
                let event_text = buffer[..event_end].to_string();
                buffer = buffer[event_end + 2..].to_string();

                for line in event_text.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            continue;
                        }
                        if let Ok(event) = serde_json::from_str::<StreamEvent>(data) {
                            match event.event_type.as_str() {
                                "message_start" => {
                                    if let Some(msg) = &event.message {
                                        tokens_in = msg.usage.input_tokens;
                                        response_model = msg.model.clone();
                                    }
                                }
                                "content_block_delta" => {
                                    if let Some(delta) = &event.delta {
                                        if let Some(text) = &delta.text {
                                            output.push_str(text);
                                        }
                                    }
                                }
                                "message_delta" => {
                                    if let Some(usage) = &event.usage {
                                        tokens_out = usage.output_tokens;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        let duration = start.elapsed().as_secs_f64();
        let cost = Self::estimate_cost(&response_model, tokens_in, tokens_out);

        Ok(ProviderResponse {
            output,
            success: true,
            exit_code: None,
            usage: Some(TokenUsage {
                tokens_in,
                tokens_out,
                cost_usd: Some(cost),
            }),
            model: Some(response_model),
            duration_secs: Some(duration),
        })
    }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }
}
