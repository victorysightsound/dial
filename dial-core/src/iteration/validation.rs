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

pub fn run_validation(conn: &Connection, iteration_id: i64) -> Result<(bool, String)> {
    let build_cmd = config_get("build_cmd")?.unwrap_or_default();
    let test_cmd = config_get("test_cmd")?.unwrap_or_default();
    let build_timeout: u64 = config_get("build_timeout")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    let test_timeout: u64 = config_get("test_timeout")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    // Run build
    if !build_cmd.is_empty() {
        println!("{}", dim(&format!("Running build: {}", build_cmd)));
        let action_id = record_action(conn, iteration_id, "build", &format!("Build: {}", build_cmd), None)?;

        let result = run_command(&build_cmd, Some(build_timeout))?;

        let output_preview = if result.output.len() > 500 {
            &result.output[..500]
        } else {
            &result.output
        };

        let error_preview = if result.output.len() > 1000 {
            &result.output[..1000]
        } else {
            &result.output
        };

        record_outcome(
            conn,
            action_id,
            result.success,
            if result.success { Some(output_preview) } else { None },
            if !result.success { Some(error_preview) } else { None },
            Some(result.duration),
        )?;

        if !result.success {
            println!("{}", red("Build failed."));
            return Ok((false, result.output));
        }
        println!("{}", green("Build passed."));
    }

    // Run tests
    if !test_cmd.is_empty() {
        println!("{}", dim(&format!("Running tests: {}", test_cmd)));
        let action_id = record_action(conn, iteration_id, "test", &format!("Test: {}", test_cmd), None)?;

        let result = run_command(&test_cmd, Some(test_timeout))?;

        let output_preview = if result.output.len() > 500 {
            &result.output[..500]
        } else {
            &result.output
        };

        let error_preview = if result.output.len() > 1000 {
            &result.output[..1000]
        } else {
            &result.output
        };

        record_outcome(
            conn,
            action_id,
            result.success,
            if result.success { Some(output_preview) } else { None },
            if !result.success { Some(error_preview) } else { None },
            Some(result.duration),
        )?;

        if !result.success {
            println!("{}", red("Tests failed."));
            return Ok((false, result.output));
        }
        println!("{}", green("Tests passed."));
    }

    Ok((true, String::new()))
}
