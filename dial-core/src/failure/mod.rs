pub mod patterns;
pub mod solutions;

use crate::db::get_db;
use crate::errors::Result;
use crate::output::{bold, dim, green, red, yellow};
use crate::TRUST_THRESHOLD;
use chrono::Local;
use rusqlite::Connection;

pub use patterns::{detect_failure_pattern, detect_failure_pattern_from_db, suggest_patterns_from_clustering, SuggestedPattern};
pub use solutions::{
    apply_confidence_decay, apply_solution_failure, apply_solution_success,
    find_trusted_solutions, get_solution_history, record_solution,
    record_solution_with_source, validate_solution, Solution, SolutionEvent,
};

pub fn get_or_create_failure_pattern(conn: &Connection, pattern_key: &str, category: &str) -> Result<i64> {
    let mut stmt = conn.prepare("SELECT id FROM failure_patterns WHERE pattern_key = ?1")?;
    let result: Option<i64> = stmt.query_row([pattern_key], |row| row.get(0)).ok();

    match result {
        Some(id) => {
            let now = Local::now().to_rfc3339();
            conn.execute(
                "UPDATE failure_patterns
                 SET occurrence_count = occurrence_count + 1, last_seen_at = ?1
                 WHERE id = ?2",
                rusqlite::params![now, id],
            )?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO failure_patterns (pattern_key, description, category)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![pattern_key, format!("Auto-detected {}", pattern_key), category],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }
}

pub fn record_failure(
    conn: &Connection,
    iteration_id: i64,
    error_text: &str,
    file_path: Option<&str>,
    line_number: Option<i64>,
) -> Result<(i64, i64)> {
    let (pattern_key, category) = detect_failure_pattern_from_db(conn, error_text);
    let pattern_id = get_or_create_failure_pattern(conn, &pattern_key, &category)?;

    conn.execute(
        "INSERT INTO failures (iteration_id, pattern_id, error_text, file_path, line_number)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![iteration_id, pattern_id, error_text, file_path, line_number],
    )?;

    let failure_id = conn.last_insert_rowid();
    Ok((failure_id, pattern_id))
}

pub fn show_failures(unresolved_only: bool) -> Result<()> {
    let conn = get_db(None)?;

    let sql = if unresolved_only {
        "SELECT f.id, f.iteration_id, f.error_text, fp.pattern_key, f.file_path, f.line_number, f.resolved
         FROM failures f
         LEFT JOIN failure_patterns fp ON f.pattern_id = fp.id
         WHERE f.resolved = 0
         ORDER BY f.created_at DESC"
    } else {
        "SELECT f.id, f.iteration_id, f.error_text, fp.pattern_key, f.file_path, f.line_number, f.resolved
         FROM failures f
         LEFT JOIN failure_patterns fp ON f.pattern_id = fp.id
         ORDER BY f.created_at DESC LIMIT 50"
    };

    let mut stmt = conn.prepare(sql)?;
    let rows: Vec<(i64, i64, String, Option<String>, Option<String>, Option<i64>, bool)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get::<_, i64>(6)? != 0,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        println!("{}", dim("No failures found."));
        return Ok(());
    }

    let title = if unresolved_only {
        "Failures (unresolved)"
    } else {
        "Failures"
    };
    println!("{}", bold(title));
    println!("{}", "=".repeat(60));

    for (id, iteration_id, error_text, pattern_key, file_path, line_number, resolved) in rows {
        let pattern = pattern_key.unwrap_or_else(|| "unknown".to_string());
        let resolved_str = if resolved {
            green("resolved")
        } else {
            red("unresolved")
        };

        println!("\n#{} [{}] {}", id, pattern, resolved_str);
        println!("  Iteration: #{}", iteration_id);

        if let Some(fp) = file_path {
            let line = line_number.map(|l| l.to_string()).unwrap_or_else(|| "?".to_string());
            println!("  File: {}:{}", fp, line);
        }

        let preview = if error_text.len() > 200 {
            format!("{}...", &error_text[..200])
        } else {
            error_text
        };
        println!("  {}", preview);
    }

    Ok(())
}

pub fn show_solutions(trusted_only: bool) -> Result<()> {
    let conn = get_db(None)?;

    let rows: Vec<(i64, String, String, Option<String>, f64, i64, i64)> = if trusted_only {
        let mut stmt = conn.prepare(
            "SELECT s.id, fp.pattern_key, s.description, s.code_example, s.confidence, s.success_count, s.failure_count
             FROM solutions s
             INNER JOIN failure_patterns fp ON s.pattern_id = fp.id
             WHERE s.confidence >= ?1
             ORDER BY s.confidence DESC",
        )?;
        let result = stmt.query_map([TRUST_THRESHOLD], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        result
    } else {
        let mut stmt = conn.prepare(
            "SELECT s.id, fp.pattern_key, s.description, s.code_example, s.confidence, s.success_count, s.failure_count
             FROM solutions s
             INNER JOIN failure_patterns fp ON s.pattern_id = fp.id
             ORDER BY s.confidence DESC",
        )?;
        let result = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        result
    };

    if rows.is_empty() {
        println!("{}", dim("No solutions recorded."));
        return Ok(());
    }

    println!("{}", bold("Solutions"));
    println!("{}", "=".repeat(60));

    for (id, pattern_key, description, code_example, confidence, success_count, failure_count) in rows {
        let trusted = confidence >= TRUST_THRESHOLD;
        let trust_indicator = if trusted {
            green("TRUSTED")
        } else {
            yellow("untrusted")
        };

        println!("\n  #{} [{}] {}", id, trust_indicator, pattern_key);
        println!(
            "     Confidence: {:.2} ({} success, {} fail)",
            confidence, success_count, failure_count
        );
        println!("     {}", description);

        if let Some(example) = code_example {
            let preview = if example.len() > 100 {
                format!("{}...", &example[..100])
            } else {
                example
            };
            println!("{}", dim(&format!("     Example: {}", preview)));
        }
    }

    Ok(())
}
