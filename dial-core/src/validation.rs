use crate::errors::Result;
use async_trait::async_trait;

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

    /// Run all steps in order. Stops on first required failure.
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

/// A built-in validation step that runs a shell command.
pub struct CommandStep {
    name: String,
    command: String,
    required: bool,
}

impl CommandStep {
    pub fn new(name: &str, command: &str) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            required: true,
        }
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
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

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .output()
            .await
            .map_err(|e| crate::errors::DialError::CommandFailed(e.to_string()))?;

        let duration = start.elapsed().as_secs_f64();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        Ok(StepResult {
            step_name: self.name.clone(),
            passed: output.status.success(),
            output: combined,
            duration_secs: duration,
        })
    }

    fn required(&self) -> bool {
        self.required
    }
}
