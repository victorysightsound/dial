use rusqlite::Connection;
use serde::Serialize;

use crate::errors::Result;

/// A single factor contributing to the overall project health score.
#[derive(Debug, Clone, Serialize)]
pub struct HealthFactor {
    /// Name of this health factor.
    pub name: String,
    /// Score from 0 to 100.
    pub score: u32,
    /// Weight of this factor (0.0 to 1.0).
    pub weight: f64,
    /// Human-readable detail about this factor's score.
    pub detail: String,
}

/// Direction the project health is trending.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    Improving,
    Stable,
    Declining,
}

impl std::fmt::Display for Trend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Trend::Improving => write!(f, "Improving"),
            Trend::Stable => write!(f, "Stable"),
            Trend::Declining => write!(f, "Declining"),
        }
    }
}

/// Overall project health score with contributing factors and trend.
#[derive(Debug, Clone, Serialize)]
pub struct HealthScore {
    /// Composite score from 0 to 100.
    pub score: u32,
    /// Trend compared to 7 days ago.
    pub trend: Trend,
    /// Individual contributing factors.
    pub factors: Vec<HealthFactor>,
}

/// Compute the project health score from the database.
///
/// Uses 6 weighted factors:
/// - success_rate (0.30): success rate of last 20 iterations
/// - success_trend (0.15): last 10 vs previous 10 iteration success rates
/// - solution_confidence (0.15): average solution confidence
/// - blocked_task_ratio (0.15): ratio of non-blocked tasks
/// - learning_utilization (0.10): ratio of referenced learnings to total
/// - pattern_resolution_rate (0.15): ratio of resolved failures to total
pub fn compute_health(conn: &Connection) -> Result<HealthScore> {
    let factors = vec![
        compute_success_rate(conn)?,
        compute_success_trend(conn)?,
        compute_solution_confidence(conn)?,
        compute_blocked_task_ratio(conn)?,
        compute_learning_utilization(conn)?,
        compute_pattern_resolution_rate(conn)?,
    ];

    let weighted_sum: f64 = factors
        .iter()
        .map(|f| f.score as f64 * f.weight)
        .sum();

    let score = weighted_sum.round() as u32;
    let trend = compute_trend(conn)?;

    Ok(HealthScore {
        score,
        trend,
        factors,
    })
}

/// Success rate of the last 20 iterations (weight: 0.30).
fn compute_success_rate(conn: &Connection) -> Result<HealthFactor> {
    let mut stmt = conn.prepare(
        "SELECT status FROM iterations ORDER BY id DESC LIMIT 20",
    )?;

    let statuses: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let total = statuses.len();
    let completed = statuses.iter().filter(|s| s == &"completed").count();

    let (score, detail) = if total == 0 {
        (50, "No iterations yet".to_string())
    } else {
        let rate = completed as f64 / total as f64;
        let s = (rate * 100.0).round() as u32;
        (s, format!("{}/{} recent iterations succeeded ({:.0}%)", completed, total, rate * 100.0))
    };

    Ok(HealthFactor {
        name: "success_rate".to_string(),
        score,
        weight: 0.30,
        detail,
    })
}

/// Success trend: last 10 vs previous 10 iterations (weight: 0.15).
fn compute_success_trend(conn: &Connection) -> Result<HealthFactor> {
    let mut stmt = conn.prepare(
        "SELECT status FROM iterations ORDER BY id DESC LIMIT 20",
    )?;

    let statuses: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let total = statuses.len();

    if total < 2 {
        return Ok(HealthFactor {
            name: "success_trend".to_string(),
            score: 50,
            weight: 0.15,
            detail: "Not enough iterations to compute trend".to_string(),
        });
    }

    let split = total.min(10);
    let recent = &statuses[..split];
    let previous = &statuses[split..];

    let recent_rate = if recent.is_empty() {
        0.0
    } else {
        recent.iter().filter(|s| s == &"completed").count() as f64 / recent.len() as f64
    };

    let previous_rate = if previous.is_empty() {
        0.0
    } else {
        previous.iter().filter(|s| s == &"completed").count() as f64 / previous.len() as f64
    };

    // Map the delta (-1.0 to +1.0) to a 0-100 score
    // +1.0 delta = 100, 0.0 delta = 50, -1.0 delta = 0
    let delta = recent_rate - previous_rate;
    let score = ((delta * 50.0) + 50.0).round().clamp(0.0, 100.0) as u32;

    let detail = format!(
        "Recent: {:.0}%, Previous: {:.0}% (delta: {:+.0}%)",
        recent_rate * 100.0,
        previous_rate * 100.0,
        delta * 100.0,
    );

    Ok(HealthFactor {
        name: "success_trend".to_string(),
        score,
        weight: 0.15,
        detail,
    })
}

/// Average solution confidence (weight: 0.15).
fn compute_solution_confidence(conn: &Connection) -> Result<HealthFactor> {
    let result: std::result::Result<(f64, i64), _> = conn.query_row(
        "SELECT COALESCE(AVG(confidence), 0.0), COUNT(*) FROM solutions",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );

    let (avg_conf, count) = result.unwrap_or((0.0, 0));

    let (score, detail) = if count == 0 {
        (50, "No solutions recorded".to_string())
    } else {
        // confidence is 0.0 to 1.0, map to 0-100
        let s = (avg_conf * 100.0).round() as u32;
        (s, format!("Average confidence: {:.2} across {} solutions", avg_conf, count))
    };

    Ok(HealthFactor {
        name: "solution_confidence".to_string(),
        score,
        weight: 0.15,
        detail,
    })
}

/// Blocked task ratio (weight: 0.15). Higher score = fewer blocked tasks.
fn compute_blocked_task_ratio(conn: &Connection) -> Result<HealthFactor> {
    let total_tasks: i64 = conn
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .unwrap_or(0);

    let blocked_tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'blocked'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let (score, detail) = if total_tasks == 0 {
        (50, "No tasks yet".to_string())
    } else {
        let non_blocked_ratio = (total_tasks - blocked_tasks) as f64 / total_tasks as f64;
        let s = (non_blocked_ratio * 100.0).round() as u32;
        (s, format!("{}/{} tasks not blocked ({:.0}%)", total_tasks - blocked_tasks, total_tasks, non_blocked_ratio * 100.0))
    };

    Ok(HealthFactor {
        name: "blocked_task_ratio".to_string(),
        score,
        weight: 0.15,
        detail,
    })
}

/// Learning utilization: referenced learnings / total learnings (weight: 0.10).
fn compute_learning_utilization(conn: &Connection) -> Result<HealthFactor> {
    let total_learnings: i64 = conn
        .query_row("SELECT COUNT(*) FROM learnings", [], |row| row.get(0))
        .unwrap_or(0);

    let referenced_learnings: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM learnings WHERE times_referenced > 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let (score, detail) = if total_learnings == 0 {
        (50, "No learnings recorded".to_string())
    } else {
        let ratio = referenced_learnings as f64 / total_learnings as f64;
        let s = (ratio * 100.0).round() as u32;
        (s, format!("{}/{} learnings referenced ({:.0}%)", referenced_learnings, total_learnings, ratio * 100.0))
    };

    Ok(HealthFactor {
        name: "learning_utilization".to_string(),
        score,
        weight: 0.10,
        detail,
    })
}

/// Pattern resolution rate: resolved failures / total failures (weight: 0.15).
fn compute_pattern_resolution_rate(conn: &Connection) -> Result<HealthFactor> {
    let total_failures: i64 = conn
        .query_row("SELECT COUNT(*) FROM failures", [], |row| row.get(0))
        .unwrap_or(0);

    let resolved_failures: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM failures WHERE resolved = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let (score, detail) = if total_failures == 0 {
        (50, "No failures recorded".to_string())
    } else {
        let ratio = resolved_failures as f64 / total_failures as f64;
        let s = (ratio * 100.0).round() as u32;
        (s, format!("{}/{} failures resolved ({:.0}%)", resolved_failures, total_failures, ratio * 100.0))
    };

    Ok(HealthFactor {
        name: "pattern_resolution_rate".to_string(),
        score,
        weight: 0.15,
        detail,
    })
}

/// Compute trend by comparing current health factors to 7 days ago.
fn compute_trend(conn: &Connection) -> Result<Trend> {
    // Current success rate from last 20 iterations
    let current_rate: f64 = {
        let mut stmt = conn.prepare(
            "SELECT status FROM iterations ORDER BY id DESC LIMIT 20",
        )?;
        let statuses: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        if statuses.is_empty() {
            return Ok(Trend::Stable);
        }
        statuses.iter().filter(|s| s == &"completed").count() as f64 / statuses.len() as f64
    };

    // Success rate from iterations that ended 7+ days ago (last 20 of those)
    let past_rate: f64 = {
        let mut stmt = conn.prepare(
            "SELECT status FROM iterations WHERE started_at <= datetime('now', '-7 days') ORDER BY id DESC LIMIT 20",
        )?;
        let statuses: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        if statuses.is_empty() {
            return Ok(Trend::Stable);
        }
        statuses.iter().filter(|s| s == &"completed").count() as f64 / statuses.len() as f64
    };

    let delta = current_rate - past_rate;
    if delta > 0.05 {
        Ok(Trend::Improving)
    } else if delta < -0.05 {
        Ok(Trend::Declining)
    } else {
        Ok(Trend::Stable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .unwrap();
        conn.execute_batch(schema::SCHEMA).unwrap();
        // Add migration columns needed for tests
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
            ALTER TABLE solutions ADD COLUMN source TEXT NOT NULL DEFAULT 'auto-learned';
            ALTER TABLE solutions ADD COLUMN last_validated_at TEXT;
            ALTER TABLE solutions ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
            ALTER TABLE failure_patterns ADD COLUMN regex_pattern TEXT;
            ALTER TABLE failure_patterns ADD COLUMN status TEXT NOT NULL DEFAULT 'suggested';
            ALTER TABLE learnings ADD COLUMN pattern_id INTEGER REFERENCES failure_patterns(id);
            ALTER TABLE learnings ADD COLUMN iteration_id INTEGER REFERENCES iterations(id);
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_health_empty_project() {
        let conn = setup_test_db();
        let health = compute_health(&conn).unwrap();

        // All factors return 50 for empty data
        assert_eq!(health.score, 50);
        assert_eq!(health.trend, Trend::Stable);
        assert_eq!(health.factors.len(), 6);

        for factor in &health.factors {
            assert_eq!(factor.score, 50, "Factor {} should be 50 for empty project", factor.name);
        }
    }

    #[test]
    fn test_health_perfect_project() {
        let conn = setup_test_db();

        // Create a task
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('perfect task', 'completed')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // 20 successful iterations
        for i in 1..=20 {
            conn.execute(
                "INSERT INTO iterations (task_id, attempt_number, status, started_at) VALUES (?1, ?2, 'completed', datetime('now', ?3))",
                rusqlite::params![task_id, i, format!("-{} hours", i)],
            )
            .unwrap();
        }

        // A high-confidence solution
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('TestErr', 'Test', 'test')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'fix it', 0.95)",
            [pattern_id],
        )
        .unwrap();

        // A referenced learning
        conn.execute(
            "INSERT INTO learnings (category, description, times_referenced) VALUES ('pattern', 'learned something', 5)",
            [],
        )
        .unwrap();

        // A resolved failure
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text, resolved) VALUES (1, ?1, 'error', 1)",
            [pattern_id],
        )
        .unwrap();

        let health = compute_health(&conn).unwrap();

        // success_rate: 100 * 0.30 = 30
        // success_trend: 50 (all same rate) * 0.15 = 7.5
        // solution_confidence: 95 * 0.15 = 14.25
        // blocked_task_ratio: 100 * 0.15 = 15
        // learning_utilization: 100 * 0.10 = 10
        // pattern_resolution_rate: 100 * 0.15 = 15
        // total = 91.75 -> 92
        assert!(health.score >= 90, "Perfect project should score >= 90, got {}", health.score);
        assert_eq!(health.factors.len(), 6);

        // Verify individual factors
        let success_rate = health.factors.iter().find(|f| f.name == "success_rate").unwrap();
        assert_eq!(success_rate.score, 100);

        let blocked = health.factors.iter().find(|f| f.name == "blocked_task_ratio").unwrap();
        assert_eq!(blocked.score, 100);

        let learning = health.factors.iter().find(|f| f.name == "learning_utilization").unwrap();
        assert_eq!(learning.score, 100);

        let resolution = health.factors.iter().find(|f| f.name == "pattern_resolution_rate").unwrap();
        assert_eq!(resolution.score, 100);
    }

    #[test]
    fn test_health_failing_project() {
        let conn = setup_test_db();

        // Create tasks, some blocked
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('blocked task', 'blocked')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('another blocked', 'blocked')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('pending task', 'pending')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // 20 failed iterations
        for i in 1..=20 {
            conn.execute(
                "INSERT INTO iterations (task_id, attempt_number, status) VALUES (?1, ?2, 'failed')",
                rusqlite::params![task_id, i],
            )
            .unwrap();
        }

        // Low-confidence solution
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('Err', 'Error', 'build')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence) VALUES (?1, 'maybe fix', 0.1)",
            [pattern_id],
        )
        .unwrap();

        // Unreferenced learnings
        conn.execute(
            "INSERT INTO learnings (category, description, times_referenced) VALUES ('build', 'learned', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO learnings (category, description, times_referenced) VALUES ('test', 'also learned', 0)",
            [],
        )
        .unwrap();

        // Unresolved failures
        for iter_id in 1..=5 {
            conn.execute(
                "INSERT INTO failures (iteration_id, pattern_id, error_text, resolved) VALUES (?1, ?2, 'err', 0)",
                rusqlite::params![iter_id, pattern_id],
            )
            .unwrap();
        }

        let health = compute_health(&conn).unwrap();

        // success_rate: 0 * 0.30 = 0
        // success_trend: 50 (all same) * 0.15 = 7.5
        // solution_confidence: 10 * 0.15 = 1.5
        // blocked_task_ratio: 33 * 0.15 = ~5
        // learning_utilization: 0 * 0.10 = 0
        // pattern_resolution_rate: 0 * 0.15 = 0
        // total ~= 14
        assert!(health.score < 40, "Failing project should score < 40, got {}", health.score);
        assert_eq!(health.factors.len(), 6);

        let success_rate = health.factors.iter().find(|f| f.name == "success_rate").unwrap();
        assert_eq!(success_rate.score, 0);

        let learning = health.factors.iter().find(|f| f.name == "learning_utilization").unwrap();
        assert_eq!(learning.score, 0);

        let resolution = health.factors.iter().find(|f| f.name == "pattern_resolution_rate").unwrap();
        assert_eq!(resolution.score, 0);
    }

    #[test]
    fn test_health_factor_weights_sum_to_one() {
        let conn = setup_test_db();
        let health = compute_health(&conn).unwrap();

        let total_weight: f64 = health.factors.iter().map(|f| f.weight).sum();
        assert!(
            (total_weight - 1.0).abs() < 0.001,
            "Weights should sum to 1.0, got {}",
            total_weight
        );
    }

    #[test]
    fn test_health_mixed_iterations() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // 15 completed, 5 failed = 75% success
        for i in 1..=15 {
            conn.execute(
                "INSERT INTO iterations (task_id, attempt_number, status) VALUES (?1, ?2, 'completed')",
                rusqlite::params![task_id, i],
            )
            .unwrap();
        }
        for i in 16..=20 {
            conn.execute(
                "INSERT INTO iterations (task_id, attempt_number, status) VALUES (?1, ?2, 'failed')",
                rusqlite::params![task_id, i],
            )
            .unwrap();
        }

        let health = compute_health(&conn).unwrap();

        let success_rate = health.factors.iter().find(|f| f.name == "success_rate").unwrap();
        assert_eq!(success_rate.score, 75);
    }

    #[test]
    fn test_success_trend_improving() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Previous 10 (lower IDs = older): all failed
        for i in 1..=10 {
            conn.execute(
                "INSERT INTO iterations (task_id, attempt_number, status) VALUES (?1, ?2, 'failed')",
                rusqlite::params![task_id, i],
            )
            .unwrap();
        }
        // Recent 10 (higher IDs = newer): all completed
        for i in 11..=20 {
            conn.execute(
                "INSERT INTO iterations (task_id, attempt_number, status) VALUES (?1, ?2, 'completed')",
                rusqlite::params![task_id, i],
            )
            .unwrap();
        }

        let health = compute_health(&conn).unwrap();

        let trend_factor = health.factors.iter().find(|f| f.name == "success_trend").unwrap();
        // ORDER BY id DESC: IDs 20..11 are recent (completed), IDs 10..1 are previous (failed)
        // recent 100%, previous 0%, delta = +1.0, score = 100
        assert_eq!(trend_factor.score, 100);
    }

    #[test]
    fn test_health_score_serialization() {
        let health = HealthScore {
            score: 75,
            trend: Trend::Improving,
            factors: vec![
                HealthFactor {
                    name: "test_factor".to_string(),
                    score: 80,
                    weight: 1.0,
                    detail: "test detail".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&health).unwrap();
        assert!(json.contains("\"score\":75"));
        assert!(json.contains("\"trend\":\"improving\""));
        assert!(json.contains("\"test_factor\""));
    }
}
