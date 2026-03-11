use crate::config::config_get;
use crate::errors::Result;
use crate::output::{dim, green, red};
use crate::DEFAULT_TIMEOUT_SECS;
use rusqlite::Connection;
use std::process::Command;
use std::time::Instant;

pub struct CommandResult {
    pub success: bool,
    pub output: String,
    pub duration: f64,
}

pub fn run_command(cmd: &str, timeout_secs: Option<u64>) -> Result<CommandResult> {
    if cmd.is_empty() {
        return Ok(CommandResult {
            success: true,
            output: String::new(),
            duration: 0.0,
        });
    }

    let _timeout_secs = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    let start = Instant::now();

    let result = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output();

    let duration = start.elapsed().as_secs_f64();

    match result {
        Ok(output) => {
            if output.status.success() {
                Ok(CommandResult {
                    success: true,
                    output: String::from_utf8_lossy(&output.stdout).to_string(),
                    duration,
                })
            } else {
                let error_output = if output.stderr.is_empty() {
                    String::from_utf8_lossy(&output.stdout).to_string()
                } else {
                    String::from_utf8_lossy(&output.stderr).to_string()
                };
                Ok(CommandResult {
                    success: false,
                    output: error_output,
                    duration,
                })
            }
        }
        Err(e) => Ok(CommandResult {
            success: false,
            output: e.to_string(),
            duration,
        }),
    }
}

pub fn record_action(
    conn: &Connection,
    iteration_id: i64,
    action_type: &str,
    description: &str,
    file_path: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO actions (iteration_id, action_type, description, file_path)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![iteration_id, action_type, description, file_path],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn record_outcome(
    conn: &Connection,
    action_id: i64,
    success: bool,
    output_summary: Option<&str>,
    error_message: Option<&str>,
    duration: Option<f64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO outcomes (action_id, success, output_summary, error_message, duration_seconds)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            action_id,
            if success { 1 } else { 0 },
            output_summary,
            error_message,
            duration
        ],
    )?;
    Ok(())
}

/// Result of a single pipeline step execution.
#[derive(Debug, Clone)]
pub struct PipelineStepResult {
    pub name: String,
    pub command: String,
    pub passed: bool,
    pub required: bool,
    pub output: String,
    pub duration_secs: f64,
    /// true if step was skipped (e.g. due to fail-fast from prior required failure)
    pub skipped: bool,
}

/// Validation result including per-step details.
pub struct ValidationResult {
    pub success: bool,
    pub error_output: String,
    pub step_results: Vec<PipelineStepResult>,
}

/// Run validation using the configurable pipeline if steps are configured,
/// otherwise fall back to build_cmd/test_cmd from config.
pub fn run_validation(conn: &Connection, iteration_id: i64) -> Result<(bool, String)> {
    let result = run_validation_with_details(conn, iteration_id)?;
    Ok((result.success, result.error_output))
}

/// Run validation and return detailed per-step results.
pub fn run_validation_with_details(conn: &Connection, iteration_id: i64) -> Result<ValidationResult> {
    // Check for configured pipeline steps
    let pipeline_steps = load_pipeline_steps(conn)?;

    if !pipeline_steps.is_empty() {
        return run_pipeline_validation(conn, iteration_id, &pipeline_steps);
    }

    // Fallback: legacy build_cmd/test_cmd behavior
    run_legacy_validation(conn, iteration_id)
}

/// Load ordered pipeline steps from the validation_steps table.
/// Returns empty vec if the table doesn't exist (pre-migration DB).
fn load_pipeline_steps(conn: &Connection) -> Result<Vec<PipelineStep>> {
    // Check if table exists (may not exist on pre-migration DBs)
    let table_exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='validation_steps'",
        [],
        |row| row.get(0),
    )?;

    if !table_exists {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT id, name, command, sort_order, required, timeout_secs
         FROM validation_steps ORDER BY sort_order, id",
    )?;

    let steps: Vec<PipelineStep> = stmt
        .query_map([], |row| {
            Ok(PipelineStep {
                _id: row.get(0)?,
                name: row.get(1)?,
                command: row.get(2)?,
                _sort_order: row.get(3)?,
                required: row.get::<_, i64>(4)? != 0,
                timeout_secs: row.get::<_, Option<i64>>(5)?.map(|t| t as u64),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(steps)
}

struct PipelineStep {
    _id: i64,
    name: String,
    command: String,
    _sort_order: i32,
    required: bool,
    timeout_secs: Option<u64>,
}

/// Run validation using the configurable pipeline. Ordered steps with per-step
/// timeout, required/optional, and fail-fast on required step failure.
fn run_pipeline_validation(
    conn: &Connection,
    iteration_id: i64,
    steps: &[PipelineStep],
) -> Result<ValidationResult> {
    let mut all_passed = true;
    let mut error_outputs = Vec::new();
    let mut step_results: Vec<PipelineStepResult> = Vec::new();
    let mut failed_fast = false;

    println!(
        "{}",
        dim(&format!("Running validation pipeline ({} steps)...", steps.len()))
    );

    for step in steps {
        // If a required step already failed, mark remaining steps as skipped
        if failed_fast {
            step_results.push(PipelineStepResult {
                name: step.name.clone(),
                command: step.command.clone(),
                passed: false,
                required: step.required,
                output: String::new(),
                duration_secs: 0.0,
                skipped: true,
            });
            continue;
        }

        let timeout = step
            .timeout_secs
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        println!(
            "{}",
            dim(&format!(
                "  Step '{}'{}: {}",
                step.name,
                if step.required { "" } else { " (optional)" },
                step.command
            ))
        );

        let action_id = record_action(
            conn,
            iteration_id,
            &step.name,
            &format!("Pipeline step '{}': {}", step.name, step.command),
            None,
        )?;

        let result = run_command(&step.command, Some(timeout))?;

        let output_preview = truncate_str(&result.output, 500);
        let error_preview = truncate_str(&result.output, 1000);

        record_outcome(
            conn,
            action_id,
            result.success,
            if result.success {
                Some(output_preview)
            } else {
                None
            },
            if !result.success {
                Some(error_preview)
            } else {
                None
            },
            Some(result.duration),
        )?;

        let step_result = PipelineStepResult {
            name: step.name.clone(),
            command: step.command.clone(),
            passed: result.success,
            required: step.required,
            output: result.output.clone(),
            duration_secs: result.duration,
            skipped: false,
        };

        if result.success {
            println!(
                "    {} ({}s)",
                green(&format!("{} passed", step.name)),
                format!("{:.1}", result.duration)
            );
        } else if step.required {
            println!("    {}", red(&format!("{} FAILED (required)", step.name)));
            all_passed = false;
            error_outputs.push(format!("[{}] {}", step.name, result.output));
            failed_fast = true;
        } else {
            println!(
                "    {}",
                dim(&format!("{} failed (optional, continuing)", step.name))
            );
        }

        step_results.push(step_result);
    }

    if all_passed {
        println!("{}", green("Pipeline passed."));
    }

    let combined_errors = error_outputs.join("\n\n");
    Ok(ValidationResult {
        success: all_passed,
        error_output: combined_errors,
        step_results,
    })
}

/// Legacy validation using build_cmd/test_cmd config values.
fn run_legacy_validation(conn: &Connection, iteration_id: i64) -> Result<ValidationResult> {
    let build_cmd = config_get("build_cmd")?.unwrap_or_default();
    let test_cmd = config_get("test_cmd")?.unwrap_or_default();
    let build_timeout: u64 = config_get("build_timeout")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    let test_timeout: u64 = config_get("test_timeout")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    let mut step_results = Vec::new();

    // Run build
    if !build_cmd.is_empty() {
        println!("{}", dim(&format!("Running build: {}", build_cmd)));
        let action_id = record_action(conn, iteration_id, "build", &format!("Build: {}", build_cmd), None)?;

        let result = run_command(&build_cmd, Some(build_timeout))?;
        let output_preview = truncate_str(&result.output, 500);
        let error_preview = truncate_str(&result.output, 1000);

        record_outcome(
            conn,
            action_id,
            result.success,
            if result.success { Some(output_preview) } else { None },
            if !result.success { Some(error_preview) } else { None },
            Some(result.duration),
        )?;

        step_results.push(PipelineStepResult {
            name: "build".to_string(),
            command: build_cmd.clone(),
            passed: result.success,
            required: true,
            output: result.output.clone(),
            duration_secs: result.duration,
            skipped: false,
        });

        if !result.success {
            println!("{}", red("Build failed."));
            // Mark test as skipped if build failed
            if !test_cmd.is_empty() {
                step_results.push(PipelineStepResult {
                    name: "test".to_string(),
                    command: test_cmd.clone(),
                    passed: false,
                    required: true,
                    output: String::new(),
                    duration_secs: 0.0,
                    skipped: true,
                });
            }
            return Ok(ValidationResult {
                success: false,
                error_output: result.output,
                step_results,
            });
        }
        println!("{}", green("Build passed."));
    }

    // Run tests
    if !test_cmd.is_empty() {
        println!("{}", dim(&format!("Running tests: {}", test_cmd)));
        let action_id = record_action(conn, iteration_id, "test", &format!("Test: {}", test_cmd), None)?;

        let result = run_command(&test_cmd, Some(test_timeout))?;
        let output_preview = truncate_str(&result.output, 500);
        let error_preview = truncate_str(&result.output, 1000);

        record_outcome(
            conn,
            action_id,
            result.success,
            if result.success { Some(output_preview) } else { None },
            if !result.success { Some(error_preview) } else { None },
            Some(result.duration),
        )?;

        step_results.push(PipelineStepResult {
            name: "test".to_string(),
            command: test_cmd.clone(),
            passed: result.success,
            required: true,
            output: result.output.clone(),
            duration_secs: result.duration,
            skipped: false,
        });

        if !result.success {
            println!("{}", red("Tests failed."));
            return Ok(ValidationResult {
                success: false,
                error_output: result.output,
                step_results,
            });
        }
        println!("{}", green("Tests passed."));
    }

    Ok(ValidationResult {
        success: true,
        error_output: String::new(),
        step_results,
    })
}

/// Truncate a string to max_len, returning a &str slice.
fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() > max_len {
        &s[..max_len]
    } else {
        s
    }
}
