use crate::errors::Result;
use crate::{TRUST_DECREMENT, TRUST_INCREMENT, TRUST_THRESHOLD};
use chrono::Local;
use rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct Solution {
    pub id: i64,
    pub pattern_id: i64,
    pub description: String,
    pub code_example: Option<String>,
    pub confidence: f64,
    pub success_count: i64,
    pub failure_count: i64,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

impl Solution {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Solution {
            id: row.get("id")?,
            pattern_id: row.get("pattern_id")?,
            description: row.get("description")?,
            code_example: row.get("code_example")?,
            confidence: row.get("confidence")?,
            success_count: row.get("success_count")?,
            failure_count: row.get("failure_count")?,
            created_at: row.get("created_at")?,
            last_used_at: row.get("last_used_at")?,
        })
    }

    pub fn is_trusted(&self) -> bool {
        self.confidence >= TRUST_THRESHOLD
    }
}

pub fn find_trusted_solutions(conn: &Connection, pattern_id: i64) -> Result<Vec<Solution>> {
    let mut stmt = conn.prepare(
        "SELECT id, pattern_id, description, code_example, confidence, success_count, failure_count, created_at, last_used_at
         FROM solutions
         WHERE pattern_id = ?1 AND confidence >= ?2
         ORDER BY confidence DESC",
    )?;

    let solutions = stmt
        .query_map(rusqlite::params![pattern_id, TRUST_THRESHOLD], |row| {
            Solution::from_row(row)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(solutions)
}

pub fn record_solution(
    conn: &Connection,
    pattern_id: i64,
    description: &str,
    code_example: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO solutions (pattern_id, description, code_example)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![pattern_id, description, code_example],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn apply_solution_success(conn: &Connection, solution_id: i64) -> Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "UPDATE solutions
         SET confidence = MIN(1.0, confidence + ?1),
             success_count = success_count + 1,
             last_used_at = ?2
         WHERE id = ?3",
        rusqlite::params![TRUST_INCREMENT, now, solution_id],
    )?;
    Ok(())
}

pub fn apply_solution_failure(conn: &Connection, solution_id: i64) -> Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "UPDATE solutions
         SET confidence = MAX(0.0, confidence - ?1),
             failure_count = failure_count + 1,
             last_used_at = ?2
         WHERE id = ?3",
        rusqlite::params![TRUST_DECREMENT, now, solution_id],
    )?;
    Ok(())
}
