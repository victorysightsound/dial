pub mod patterns;
pub mod solutions;

use crate::db::{get_db, with_transaction};
use crate::errors::Result;
use crate::output::{bold, dim, green, red, yellow};
use crate::TRUST_THRESHOLD;
use chrono::Local;
use rusqlite::Connection;

/// Aggregated metrics for a single failure pattern.
#[derive(Debug, Clone)]
pub struct PatternMetrics {
    pub pattern_key: String,
    pub category: String,
    pub total_occurrences: i64,
    pub total_resolution_time_secs: f64,
    pub avg_resolution_time_secs: f64,
    pub total_tokens_consumed: i64,
    pub total_cost_usd: f64,
    pub auto_resolved_count: i64,
    pub manual_resolved_count: i64,
    pub unresolved_count: i64,
    pub first_seen: String,
    pub last_seen: String,
}

impl PatternMetrics {
    /// Format as a JSON string.
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"pattern_key":"{}","category":"{}","total_occurrences":{},"total_resolution_time_secs":{:.1},"avg_resolution_time_secs":{:.1},"total_tokens_consumed":{},"total_cost_usd":{:.4},"auto_resolved_count":{},"manual_resolved_count":{},"unresolved_count":{},"first_seen":"{}","last_seen":"{}"}}"#,
            self.pattern_key, self.category, self.total_occurrences,
            self.total_resolution_time_secs, self.avg_resolution_time_secs,
            self.total_tokens_consumed, self.total_cost_usd,
            self.auto_resolved_count, self.manual_resolved_count, self.unresolved_count,
            self.first_seen, self.last_seen,
        )
    }
}

/// Compute aggregated metrics per failure pattern.
///
/// Joins failures, iterations, provider_usage, and solutions tables to compute:
/// - total occurrences per pattern
/// - resolution times (from iteration duration_seconds)
/// - tokens and cost (from provider_usage linked via iteration_id)
/// - resolution breakdown (auto-resolved via solution, manually resolved, unresolved)
/// - first/last seen timestamps
pub fn compute_pattern_metrics(conn: &Connection) -> Result<Vec<PatternMetrics>> {
    let mut stmt = conn.prepare(
        "SELECT
            fp.pattern_key,
            COALESCE(fp.category, 'unknown') as category,
            COUNT(f.id) as total_occurrences,
            COALESCE(SUM(i.duration_seconds), 0.0) as total_resolution_time_secs,
            COALESCE(SUM(pu_tokens.tokens_in + pu_tokens.tokens_out), 0) as total_tokens_consumed,
            COALESCE(SUM(pu_tokens.cost_usd), 0.0) as total_cost_usd,
            SUM(CASE WHEN f.resolved = 1 AND f.resolved_by_solution_id IS NOT NULL THEN 1 ELSE 0 END) as auto_resolved_count,
            SUM(CASE WHEN f.resolved = 1 AND f.resolved_by_solution_id IS NULL THEN 1 ELSE 0 END) as manual_resolved_count,
            SUM(CASE WHEN f.resolved = 0 THEN 1 ELSE 0 END) as unresolved_count,
            MIN(f.created_at) as first_seen,
            MAX(f.created_at) as last_seen
         FROM failure_patterns fp
         INNER JOIN failures f ON f.pattern_id = fp.id
         INNER JOIN iterations i ON f.iteration_id = i.id
         LEFT JOIN (
             SELECT iteration_id,
                    SUM(tokens_in + tokens_out) as tokens_in_out_sum,
                    SUM(tokens_in) as tokens_in,
                    SUM(tokens_out) as tokens_out,
                    SUM(cost_usd) as cost_usd
             FROM provider_usage
             GROUP BY iteration_id
         ) pu_tokens ON pu_tokens.iteration_id = i.id
         GROUP BY fp.id
         ORDER BY total_occurrences DESC",
    )?;

    let metrics = stmt
        .query_map([], |row| {
            let total_occurrences: i64 = row.get(2)?;
            let total_resolution_time_secs: f64 = row.get(3)?;
            let avg_resolution_time_secs = if total_occurrences > 0 {
                total_resolution_time_secs / total_occurrences as f64
            } else {
                0.0
            };

            Ok(PatternMetrics {
                pattern_key: row.get(0)?,
                category: row.get(1)?,
                total_occurrences,
                total_resolution_time_secs,
                avg_resolution_time_secs,
                total_tokens_consumed: row.get(4)?,
                total_cost_usd: row.get(5)?,
                auto_resolved_count: row.get(6)?,
                manual_resolved_count: row.get(7)?,
                unresolved_count: row.get(8)?,
                first_seen: row.get::<_, String>(9).unwrap_or_default(),
                last_seen: row.get::<_, String>(10).unwrap_or_default(),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(metrics)
}

pub use patterns::{detect_failure_pattern, detect_failure_pattern_from_db, suggest_patterns_from_clustering, SuggestedPattern};
pub use solutions::{
    apply_confidence_decay, apply_solution_failure, apply_solution_success,
    find_solutions_for_pattern, find_trusted_solutions, get_pending_solution_applications,
    get_solution_history, mark_solution_applications_success, record_solution,
    record_solution_application, record_solution_with_source, validate_solution,
    Solution, SolutionEvent,
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
) -> Result<(i64, i64, Vec<(i64, String, f64)>)> {
    with_transaction(conn, |conn| {
        let (pattern_key, category) = detect_failure_pattern_from_db(conn, error_text);
        let pattern_id = get_or_create_failure_pattern(conn, &pattern_key, &category)?;

        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text, file_path, line_number)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![iteration_id, pattern_id, error_text, file_path, line_number],
        )?;

        let failure_id = conn.last_insert_rowid();

        // Auto-suggest: find trusted solutions for the matched pattern
        let suggested_solutions = solutions::find_solutions_for_pattern(conn, pattern_id)?;

        // Record each suggestion as a solution application (success=NULL until resolved)
        for &(solution_id, _, _) in &suggested_solutions {
            solutions::record_solution_application(conn, solution_id, failure_id, iteration_id)?;
        }

        Ok((failure_id, pattern_id, suggested_solutions))
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    /// Set up an in-memory DB with base schema + migration tables needed for pattern metrics.
    fn setup_metrics_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .unwrap();
        conn.execute_batch(schema::SCHEMA).unwrap();
        // Add provider_usage table (migration 3)
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS provider_usage (
                id INTEGER PRIMARY KEY,
                iteration_id INTEGER,
                provider TEXT NOT NULL,
                model TEXT,
                tokens_in INTEGER DEFAULT 0,
                tokens_out INTEGER DEFAULT 0,
                cost_usd REAL DEFAULT 0.0,
                duration_secs REAL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (iteration_id) REFERENCES iterations(id)
            );
            "#,
        )
        .unwrap();
        // Add solution provenance columns (migration 7)
        conn.execute_batch(
            r#"
            ALTER TABLE solutions ADD COLUMN source TEXT NOT NULL DEFAULT 'auto-learned';
            ALTER TABLE solutions ADD COLUMN last_validated_at TEXT;
            ALTER TABLE solutions ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
            "#,
        )
        .unwrap();
        // Add pattern columns (migration 6)
        conn.execute_batch(
            r#"
            ALTER TABLE failure_patterns ADD COLUMN regex_pattern TEXT;
            ALTER TABLE failure_patterns ADD COLUMN status TEXT NOT NULL DEFAULT 'suggested';
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_compute_pattern_metrics_empty() {
        let conn = setup_metrics_test_db();
        let metrics = compute_pattern_metrics(&conn).unwrap();
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_compute_pattern_metrics_single_pattern() {
        let conn = setup_metrics_test_db();

        // Create task + iteration
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('test task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, duration_seconds) VALUES (?1, 1, 120.5)",
            [task_id],
        )
        .unwrap();
        let iteration_id = conn.last_insert_rowid();

        // Create pattern
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('RustCompileError', 'Rust compile error', 'build')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        // Create failure (unresolved)
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'error[E0308]')",
            rusqlite::params![iteration_id, pattern_id],
        )
        .unwrap();

        // Add provider_usage for this iteration
        conn.execute(
            "INSERT INTO provider_usage (iteration_id, provider, model, tokens_in, tokens_out, cost_usd) VALUES (?1, 'anthropic', 'claude', 1000, 500, 0.05)",
            [iteration_id],
        )
        .unwrap();

        let metrics = compute_pattern_metrics(&conn).unwrap();
        assert_eq!(metrics.len(), 1);

        let m = &metrics[0];
        assert_eq!(m.pattern_key, "RustCompileError");
        assert_eq!(m.category, "build");
        assert_eq!(m.total_occurrences, 1);
        assert!((m.total_resolution_time_secs - 120.5).abs() < 0.1);
        assert!((m.avg_resolution_time_secs - 120.5).abs() < 0.1);
        assert_eq!(m.total_tokens_consumed, 1500);
        assert!((m.total_cost_usd - 0.05).abs() < 0.001);
        assert_eq!(m.auto_resolved_count, 0);
        assert_eq!(m.manual_resolved_count, 0);
        assert_eq!(m.unresolved_count, 1);
    }

    #[test]
    fn test_compute_pattern_metrics_resolution_types() {
        let conn = setup_metrics_test_db();

        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Create pattern
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('TestFailure', 'Test failure', 'test')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        // Create a solution
        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'fix it', 0.8)",
            [pattern_id],
        )
        .unwrap();
        let solution_id = conn.last_insert_rowid();

        // Iteration 1: failure auto-resolved by solution
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, duration_seconds) VALUES (?1, 1, 60.0)",
            [task_id],
        )
        .unwrap();
        let iter1 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text, resolved, resolved_by_solution_id) VALUES (?1, ?2, 'fail 1', 1, ?3)",
            rusqlite::params![iter1, pattern_id, solution_id],
        )
        .unwrap();

        // Iteration 2: failure manually resolved (no solution)
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, duration_seconds) VALUES (?1, 2, 30.0)",
            [task_id],
        )
        .unwrap();
        let iter2 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text, resolved) VALUES (?1, ?2, 'fail 2', 1)",
            rusqlite::params![iter2, pattern_id],
        )
        .unwrap();

        // Iteration 3: unresolved failure
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, duration_seconds) VALUES (?1, 3, 45.0)",
            [task_id],
        )
        .unwrap();
        let iter3 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'fail 3')",
            rusqlite::params![iter3, pattern_id],
        )
        .unwrap();

        let metrics = compute_pattern_metrics(&conn).unwrap();
        assert_eq!(metrics.len(), 1);

        let m = &metrics[0];
        assert_eq!(m.total_occurrences, 3);
        assert_eq!(m.auto_resolved_count, 1);
        assert_eq!(m.manual_resolved_count, 1);
        assert_eq!(m.unresolved_count, 1);
        assert!((m.total_resolution_time_secs - 135.0).abs() < 0.1);
        assert!((m.avg_resolution_time_secs - 45.0).abs() < 0.1);
    }

    #[test]
    fn test_compute_pattern_metrics_multiple_patterns() {
        let conn = setup_metrics_test_db();

        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Pattern A: 3 occurrences
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('ErrorA', 'Error A', 'build')",
            [],
        )
        .unwrap();
        let pattern_a = conn.last_insert_rowid();

        // Pattern B: 1 occurrence
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('ErrorB', 'Error B', 'test')",
            [],
        )
        .unwrap();
        let pattern_b = conn.last_insert_rowid();

        for i in 0..3 {
            conn.execute(
                "INSERT INTO iterations (task_id, attempt_number, duration_seconds) VALUES (?1, ?2, 10.0)",
                rusqlite::params![task_id, i + 1],
            )
            .unwrap();
            let iter_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'err a')",
                rusqlite::params![iter_id, pattern_a],
            )
            .unwrap();
        }

        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, duration_seconds) VALUES (?1, 4, 20.0)",
            [task_id],
        )
        .unwrap();
        let iter_b = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'err b')",
            rusqlite::params![iter_b, pattern_b],
        )
        .unwrap();

        let metrics = compute_pattern_metrics(&conn).unwrap();
        assert_eq!(metrics.len(), 2);

        // Default sort is by occurrences DESC
        assert_eq!(metrics[0].pattern_key, "ErrorA");
        assert_eq!(metrics[0].total_occurrences, 3);
        assert_eq!(metrics[1].pattern_key, "ErrorB");
        assert_eq!(metrics[1].total_occurrences, 1);
    }

    #[test]
    fn test_compute_pattern_metrics_no_provider_usage() {
        let conn = setup_metrics_test_db();

        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, duration_seconds) VALUES (?1, 1, 50.0)",
            [task_id],
        )
        .unwrap();
        let iter_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('NoUsage', 'No usage', 'runtime')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'some error')",
            rusqlite::params![iter_id, pattern_id],
        )
        .unwrap();

        let metrics = compute_pattern_metrics(&conn).unwrap();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].total_tokens_consumed, 0);
        assert!((metrics[0].total_cost_usd - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_pattern_metrics_to_json() {
        let m = PatternMetrics {
            pattern_key: "TestError".to_string(),
            category: "test".to_string(),
            total_occurrences: 5,
            total_resolution_time_secs: 300.0,
            avg_resolution_time_secs: 60.0,
            total_tokens_consumed: 5000,
            total_cost_usd: 0.25,
            auto_resolved_count: 2,
            manual_resolved_count: 1,
            unresolved_count: 2,
            first_seen: "2026-01-01T00:00:00".to_string(),
            last_seen: "2026-03-12T00:00:00".to_string(),
        };

        let json = m.to_json();
        assert!(json.contains("\"pattern_key\":\"TestError\""));
        assert!(json.contains("\"total_occurrences\":5"));
        assert!(json.contains("\"auto_resolved_count\":2"));
    }
}
