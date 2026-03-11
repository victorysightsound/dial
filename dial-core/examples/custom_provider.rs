//! Example of implementing a custom AI provider.

use async_trait::async_trait;
use dial_core::provider::{Provider, ProviderRequest, ProviderResponse, TokenUsage};
use dial_core::Engine;
use std::sync::Arc;

/// A mock provider that echoes back a predefined response.
struct EchoProvider {
    model: String,
}

#[async_trait]
impl Provider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    async fn execute(&self, request: ProviderRequest) -> dial_core::Result<ProviderResponse> {
        let prompt_len = request.prompt.len();
        let response_text = format!(
            "Echo provider received {} chars. Model: {}.",
            prompt_len, self.model,
        );

        Ok(ProviderResponse {
            output: response_text,
            success: true,
            exit_code: Some(0),
            usage: Some(TokenUsage {
                tokens_in: prompt_len as u64 / 4,
                tokens_out: 50,
                cost_usd: Some(0.001),
            }),
            model: Some(self.model.clone()),
            duration_secs: Some(0.5),
        })
    }

    async fn is_available(&self) -> bool {
        true
    }
}

#[tokio::main]
async fn main() -> dial_core::Result<()> {
    // Initialize engine
    let mut engine = Engine::init("provider-demo", None, false).await?;

    // Register custom provider
    let provider = Arc::new(EchoProvider {
        model: "echo-1.0".to_string(),
    });
    engine.set_provider(provider.clone());

    // Verify provider is registered
    let p = engine.provider().expect("provider should be set");
    println!("Registered provider: {}", p.name());

    // Execute a request through the provider
    let request = ProviderRequest {
        prompt: "Implement a fibonacci function in Rust".to_string(),
        work_dir: ".".to_string(),
        max_tokens: Some(4096),
        model: None,
        timeout_secs: None,
    };

    let response = p.execute(request).await?;
    println!("Response: {}", response.output);

    if let Some(usage) = &response.usage {
        println!("Tokens: {} in, {} out", usage.tokens_in, usage.tokens_out);
        if let Some(cost) = usage.cost_usd {
            println!("Cost: ${:.4}", cost);
        }
    }

    Ok(())
}
