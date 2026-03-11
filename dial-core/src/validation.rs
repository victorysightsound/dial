use crate::errors::{DialError, Result};
use crate::engine::PipelineStepConfig;
use async_trait::async_trait;
use std::time::Duration;

/// Result of a single validation step.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// Name of the step that ran.
    pub step_name: String,
    /// Whether the step passed.
    pub passed: bool,
    /// Output from the step (stdout/stderr combined).
    pub output: String,
    /// Duration in seconds.
    pub duration_secs: f64,
}

/// A single step in the validation pipeline.
#[async_trait]
pub trait ValidationStep: Send + Sync {
    /// Name of this validation step.
    fn name(&self) -> &str;

    /// Execute the validation step. Returns the result.
    async fn run(&self) -> Result<StepResult>;

    /// Whether this step is required (pipeline stops if it fails).
    fn required(&self) -> bool {
        true
    }
}

/// A configurable validation pipeline that runs steps in sequence.
pub struct ValidationPipeline {
    steps: Vec<Box<dyn ValidationStep>>,
}

impl ValidationPipeline {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Add a step to the pipeline.
    pub fn add_step(&mut self, step: Box<dyn ValidationStep>) {
        self.steps.push(step);
    }

    /// Returns true if no steps are configured.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Run all steps in order. Stops on first required failure (fail-fast).
    pub async fn run(&self) -> Result<Vec<StepResult>> {
        let mut results = Vec::new();

        for step in &self.steps {
            let result = step.run().await?;
            let passed = result.passed;
            let required = step.required();
            results.push(result);

            if !passed && required {
                break;
            }
        }

        Ok(results)
    }

    /// Check if all results passed.
    pub fn all_passed(results: &[StepResult]) -> bool {
        results.iter().all(|r| r.passed)
    }

    /// Get combined error output from failed steps.
    pub fn error_output(results: &[StepResult]) -> String {
        results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| format!("[{}] {}", r.step_name, r.output))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl Default for ValidationPipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// A built-in validation step that runs a shell command with optional timeout.
pub struct CommandStep {
    name: String,
    command: String,
    required: bool,
    timeout_secs: Option<u64>,
}

impl CommandStep {
    pub fn new(name: &str, command: &str) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            required: true,
            timeout_secs: None,
        }
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = Some(timeout_secs);
        self
    }
}

#[async_trait]
impl ValidationStep for CommandStep {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self) -> Result<StepResult> {
        let start = std::time::Instant::now();

        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| DialError::CommandFailed(e.to_string()))?;

        let result = if let Some(timeout) = self.timeout_secs {
            match tokio::time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await
            {
                Ok(output_result) => output_result
                    .map_err(|e| DialError::CommandFailed(e.to_string()))?,
                Err(_) => {
                    let duration = start.elapsed().as_secs_f64();
                    return Ok(StepResult {
                        step_name: self.name.clone(),
                        passed: false,
                        output: format!(
                            "Command timed out after {} seconds",
                            timeout
                        ),
                        duration_secs: duration,
                    });
                }
            }
        } else {
            child
                .wait_with_output()
                .await
                .map_err(|e| DialError::CommandFailed(e.to_string()))?
        };

        let duration = start.elapsed().as_secs_f64();
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let combined = format!("{}{}", stdout, stderr);

        Ok(StepResult {
            step_name: self.name.clone(),
            passed: result.status.success(),
            output: combined,
            duration_secs: duration,
        })
    }

    fn required(&self) -> bool {
        self.required
    }
}

/// Build a validation pipeline from DB step configurations.
/// Steps are assumed to already be sorted by sort_order.
pub fn build_pipeline(configs: &[PipelineStepConfig]) -> ValidationPipeline {
    let mut pipeline = ValidationPipeline::new();
    for config in configs {
        let mut step = CommandStep::new(&config.name, &config.command);
        if !config.required {
            step = step.optional();
        }
        if let Some(timeout) = config.timeout_secs {
            step = step.with_timeout(timeout);
        }
        pipeline.add_step(Box::new(step));
    }
    pipeline
}

/// Build a backwards-compatible two-step pipeline from build_cmd/test_cmd config.
pub fn build_legacy_pipeline(build_cmd: &str, test_cmd: &str) -> ValidationPipeline {
    let mut pipeline = ValidationPipeline::new();
    if !build_cmd.is_empty() {
        pipeline.add_step(Box::new(CommandStep::new("build", build_cmd)));
    }
    if !test_cmd.is_empty() {
        pipeline.add_step(Box::new(CommandStep::new("test", test_cmd)));
    }
    pipeline
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_command_step_success() {
        let step = CommandStep::new("echo-test", "echo hello");
        let result = step.run().await.unwrap();
        assert!(result.passed);
        assert!(result.output.contains("hello"));
        assert_eq!(result.step_name, "echo-test");
    }

    #[tokio::test]
    async fn test_command_step_failure() {
        let step = CommandStep::new("fail-test", "exit 1");
        let result = step.run().await.unwrap();
        assert!(!result.passed);
    }

    #[tokio::test]
    async fn test_command_step_timeout() {
        let step = CommandStep::new("slow-test", "sleep 10").with_timeout(1);
        let result = step.run().await.unwrap();
        assert!(!result.passed);
        assert!(result.output.contains("timed out"));
    }

    #[tokio::test]
    async fn test_pipeline_fail_fast_on_required() {
        let mut pipeline = ValidationPipeline::new();
        pipeline.add_step(Box::new(CommandStep::new("fail", "exit 1")));
        pipeline.add_step(Box::new(CommandStep::new("never-runs", "echo should-not-run")));

        let results = pipeline.run().await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].passed);
    }

    #[tokio::test]
    async fn test_pipeline_continues_on_optional_failure() {
        let mut pipeline = ValidationPipeline::new();
        pipeline.add_step(Box::new(CommandStep::new("optional-fail", "exit 1").optional()));
        pipeline.add_step(Box::new(CommandStep::new("should-run", "echo ran")));

        let results = pipeline.run().await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(!results[0].passed);
        assert!(results[1].passed);
    }

    #[tokio::test]
    async fn test_pipeline_all_passed() {
        let results = vec![
            StepResult { step_name: "a".to_string(), passed: true, output: String::new(), duration_secs: 0.0 },
            StepResult { step_name: "b".to_string(), passed: true, output: String::new(), duration_secs: 0.0 },
        ];
        assert!(ValidationPipeline::all_passed(&results));
    }

    #[tokio::test]
    async fn test_pipeline_not_all_passed() {
        let results = vec![
            StepResult { step_name: "a".to_string(), passed: true, output: String::new(), duration_secs: 0.0 },
            StepResult { step_name: "b".to_string(), passed: false, output: "err".to_string(), duration_secs: 0.0 },
        ];
        assert!(!ValidationPipeline::all_passed(&results));
    }

    #[tokio::test]
    async fn test_error_output() {
        let results = vec![
            StepResult { step_name: "build".to_string(), passed: true, output: "ok".to_string(), duration_secs: 0.0 },
            StepResult { step_name: "test".to_string(), passed: false, output: "test failure".to_string(), duration_secs: 0.0 },
        ];
        let output = ValidationPipeline::error_output(&results);
        assert!(output.contains("[test] test failure"));
        assert!(!output.contains("[build]"));
    }

    #[test]
    fn test_build_pipeline_from_configs() {
        let configs = vec![
            PipelineStepConfig {
                id: 1,
                name: "build".to_string(),
                command: "cargo build".to_string(),
                sort_order: 0,
                required: true,
                timeout_secs: Some(300),
            },
            PipelineStepConfig {
                id: 2,
                name: "test".to_string(),
                command: "cargo test".to_string(),
                sort_order: 1,
                required: true,
                timeout_secs: None,
            },
        ];

        let pipeline = build_pipeline(&configs);
        assert!(!pipeline.is_empty());
    }

    #[test]
    fn test_build_legacy_pipeline() {
        let pipeline = build_legacy_pipeline("cargo build", "cargo test");
        assert!(!pipeline.is_empty());

        let empty_pipeline = build_legacy_pipeline("", "");
        assert!(empty_pipeline.is_empty());
    }
}
