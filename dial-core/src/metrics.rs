use rusqlite::Connection;
use crate::errors::Result;

/// Structured metrics report returned by Engine::stats().
#[derive(Debug, Clone)]
pub struct MetricsReport {
    pub total_iterations: i64,
    pub completed_iterations: i64,
    pub failed_iterations: i64,
    pub success_rate: f64,
    pub total_tasks: i64,
    pub completed_tasks: i64,
    pub pending_tasks: i64,
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
    pub total_cost_usd: f64,
    pub total_duration_secs: f64,
    pub avg_iteration_duration_secs: f64,
    pub total_failures: i64,
    pub total_learnings: i64,
}

/// Compute a structured metrics report from the database.
pub fn compute_metrics(conn: &Connection) -> Result<MetricsReport> {
    let total_iterations: i64 = conn.query_row(
        "SELECT COUNT(*) FROM iterations", [], |row| row.get(0),
    ).unwrap_or(0);

    let completed_iterations: i64 = conn.query_row(
        "SELECT COUNT(*) FROM iterations WHERE status = 'completed'", [], |row| row.get(0),
    ).unwrap_or(0);

    let failed_iterations: i64 = conn.query_row(
        "SELECT COUNT(*) FROM iterations WHERE status = 'failed'", [], |row| row.get(0),
    ).unwrap_or(0);

    let success_rate = if total_iterations > 0 {
        completed_iterations as f64 / total_iterations as f64
    } else {
        0.0
    };

    let total_tasks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks", [], |row| row.get(0),
    ).unwrap_or(0);

    let completed_tasks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'done'", [], |row| row.get(0),
    ).unwrap_or(0);

    let pending_tasks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'pending'", [], |row| row.get(0),
    ).unwrap_or(0);

    // Token and cost totals from provider_usage
    let (total_tokens_in, total_tokens_out, total_cost_usd): (i64, i64, f64) = conn.query_row(
        "SELECT COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0), COALESCE(SUM(cost_usd), 0.0) FROM provider_usage",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap_or((0, 0, 0.0));

    let total_duration_secs: f64 = conn.query_row(
        "SELECT COALESCE(SUM(duration_seconds), 0.0) FROM iterations WHERE duration_seconds IS NOT NULL",
        [],
        |row| row.get(0),
    ).unwrap_or(0.0);

    let avg_iteration_duration_secs = if completed_iterations > 0 {
        total_duration_secs / completed_iterations as f64
    } else {
        0.0
    };

    let total_failures: i64 = conn.query_row(
        "SELECT COUNT(*) FROM failures", [], |row| row.get(0),
    ).unwrap_or(0);

    let total_learnings: i64 = conn.query_row(
        "SELECT COUNT(*) FROM learnings", [], |row| row.get(0),
    ).unwrap_or(0);

    Ok(MetricsReport {
        total_iterations,
        completed_iterations,
        failed_iterations,
        success_rate,
        total_tasks,
        completed_tasks,
        pending_tasks,
        total_tokens_in,
        total_tokens_out,
        total_cost_usd,
        total_duration_secs,
        avg_iteration_duration_secs,
        total_failures,
        total_learnings,
    })
}

/// A single data point for trend analysis.
#[derive(Debug, Clone)]
pub struct TrendPoint {
    pub date: String,
    pub iterations: i64,
    pub successes: i64,
    pub failures: i64,
    pub success_rate: f64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: f64,
}

/// Compute daily trend data over the last N days.
pub fn compute_trends(conn: &Connection, days: i64) -> Result<Vec<TrendPoint>> {
    let mut stmt = conn.prepare(
        "SELECT
            date(started_at) as day,
            COUNT(*) as total,
            SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as successes,
            SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failures
         FROM iterations
         WHERE started_at >= date('now', ?1)
         GROUP BY date(started_at)
         ORDER BY day",
    )?;

    let offset = format!("-{} days", days);
    let rows = stmt.query_map([&offset], |row| {
        let total: i64 = row.get(1)?;
        let successes: i64 = row.get(2)?;
        let failures: i64 = row.get(3)?;
        Ok((row.get::<_, String>(0)?, total, successes, failures))
    })?;

    let mut trends = Vec::new();
    for row in rows {
        let (day, _total, successes, failures) = row?;

        // Get token/cost data for this day
        let (tokens_in, tokens_out, cost_usd): (i64, i64, f64) = conn.query_row(
            "SELECT COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0), COALESCE(SUM(cost_usd), 0.0)
             FROM provider_usage WHERE date(created_at) = ?1",
            [&day],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap_or((0, 0, 0.0));

        let success_rate = if successes + failures > 0 {
            successes as f64 / (successes + failures) as f64
        } else {
            0.0
        };

        trends.push(TrendPoint {
            date: day,
            iterations: successes + failures,
            successes,
            failures,
            success_rate,
            tokens_in,
            tokens_out,
            cost_usd,
        });
    }

    Ok(trends)
}

/// Record a metric snapshot after an iteration completes.
/// This records into the metrics table for historical tracking.
pub fn record_iteration_metric(
    conn: &Connection,
    iteration_id: i64,
    task_id: i64,
    success: bool,
    duration_secs: f64,
    tokens_in: i64,
    tokens_out: i64,
    cost_usd: f64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO metrics (iteration_id, task_id, success, duration_secs, tokens_in, tokens_out, cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![iteration_id, task_id, success, duration_secs, tokens_in, tokens_out, cost_usd],
    )?;
    Ok(())
}

impl MetricsReport {
    /// Format as JSON string.
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"total_iterations":{},"completed_iterations":{},"failed_iterations":{},"success_rate":{:.4},"total_tasks":{},"completed_tasks":{},"pending_tasks":{},"total_tokens_in":{},"total_tokens_out":{},"total_cost_usd":{:.4},"total_duration_secs":{:.1},"avg_iteration_duration_secs":{:.1},"total_failures":{},"total_learnings":{}}}"#,
            self.total_iterations, self.completed_iterations, self.failed_iterations,
            self.success_rate, self.total_tasks, self.completed_tasks, self.pending_tasks,
            self.total_tokens_in, self.total_tokens_out, self.total_cost_usd,
            self.total_duration_secs, self.avg_iteration_duration_secs,
            self.total_failures, self.total_learnings,
        )
    }

    /// Format as CSV string (header + row).
    pub fn to_csv(&self) -> String {
        let header = "total_iterations,completed_iterations,failed_iterations,success_rate,total_tasks,completed_tasks,pending_tasks,total_tokens_in,total_tokens_out,total_cost_usd,total_duration_secs,avg_iteration_duration_secs,total_failures,total_learnings";
        let row = format!(
            "{},{},{},{:.4},{},{},{},{},{},{:.4},{:.1},{:.1},{},{}",
            self.total_iterations, self.completed_iterations, self.failed_iterations,
            self.success_rate, self.total_tasks, self.completed_tasks, self.pending_tasks,
            self.total_tokens_in, self.total_tokens_out, self.total_cost_usd,
            self.total_duration_secs, self.avg_iteration_duration_secs,
            self.total_failures, self.total_learnings,
        );
        format!("{}\n{}", header, row)
    }
}

impl TrendPoint {
    /// Format as JSON string.
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"date":"{}","iterations":{},"successes":{},"failures":{},"success_rate":{:.4},"tokens_in":{},"tokens_out":{},"cost_usd":{:.4}}}"#,
            self.date, self.iterations, self.successes, self.failures,
            self.success_rate, self.tokens_in, self.tokens_out, self.cost_usd,
        )
    }
}
