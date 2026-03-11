use crate::db::get_db;
use crate::errors::{DialError, Result};
use crate::output::{blue, bold, dim, yellow};
use rusqlite::Connection;

pub const LEARNING_CATEGORIES: &[&str] = &["build", "test", "setup", "gotcha", "pattern", "tool", "other"];

pub fn add_learning(description: &str, category: Option<&str>) -> Result<i64> {
    let conn = get_db(None)?;

    // Validate category
    let category = match category {
        Some(c) if LEARNING_CATEGORIES.contains(&c) => Some(c),
        Some(c) => {
            println!("{}", yellow(&format!("Warning: Unknown category '{}'. Using 'other'.", c)));
            Some("other")
        }
        None => None,
    };

    conn.execute(
        "INSERT INTO learnings (category, description) VALUES (?1, ?2)",
        rusqlite::params![category, description],
    )?;

    let learning_id = conn.last_insert_rowid();
    Ok(learning_id)
}

pub fn list_learnings(category: Option<&str>) -> Result<()> {
    let conn = get_db(None)?;

    let rows: Vec<(i64, Option<String>, String, String, i64)> = if let Some(cat) = category {
        let mut stmt = conn.prepare(
            "SELECT id, category, description, discovered_at, times_referenced
             FROM learnings WHERE category = ?1
             ORDER BY discovered_at DESC",
        )?;
        let result = stmt.query_map([cat], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        result
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, category, description, discovered_at, times_referenced
             FROM learnings ORDER BY discovered_at DESC",
        )?;
        let result = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        result
    };

    if rows.is_empty() {
        println!("{}", dim("No learnings recorded."));
        return Ok(());
    }

    let title = if let Some(cat) = category {
        format!("Learnings ({})", cat)
    } else {
        "Learnings".to_string()
    };

    println!("{}", bold(&title));
    println!("{}", "=".repeat(60));

    for (id, cat, description, discovered_at, times_referenced) in rows {
        let cat_str = cat
            .map(|c| format!("[{}]", c))
            .unwrap_or_else(|| "[uncategorized]".to_string());

        let ref_str = if times_referenced > 0 {
            format!("(referenced {}x)", times_referenced)
        } else {
            String::new()
        };

        println!("\n  #{} {} {}", id, blue(&cat_str), ref_str);
        println!("     {}", description);
        println!("{}", dim(&format!("     Discovered: {}", &discovered_at[..10])));
    }

    Ok(())
}

pub fn search_learnings(query: &str) -> Result<Vec<LearningResult>> {
    let conn = get_db(None)?;

    let mut stmt = conn.prepare(
        "SELECT l.id, l.category, l.description, l.times_referenced
         FROM learnings l
         INNER JOIN learnings_fts fts ON l.id = fts.rowid
         WHERE learnings_fts MATCH ?1
         ORDER BY rank LIMIT 10",
    )?;

    let rows: Vec<LearningResult> = stmt
        .query_map([query], |row| {
            Ok(LearningResult {
                id: row.get(0)?,
                category: row.get(1)?,
                description: row.get(2)?,
                times_referenced: row.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        println!("{}", dim(&format!("No learnings matching '{}'.", query)));
        return Ok(rows);
    }

    println!("{}", bold(&format!("Learnings matching '{}':", query)));
    println!("{}", "=".repeat(60));

    for row in &rows {
        let cat_str = row
            .category
            .as_ref()
            .map(|c| format!("[{}]", c))
            .unwrap_or_default();
        println!("\n  #{} {}", row.id, blue(&cat_str));
        println!("     {}", row.description);
    }

    Ok(rows)
}

#[derive(Debug, Clone)]
pub struct LearningResult {
    pub id: i64,
    pub category: Option<String>,
    pub description: String,
    pub times_referenced: i64,
}

pub fn delete_learning(learning_id: i64) -> Result<()> {
    let conn = get_db(None)?;

    let changed = conn.execute("DELETE FROM learnings WHERE id = ?1", [learning_id])?;

    if changed == 0 {
        return Err(DialError::LearningNotFound(learning_id));
    }

    Ok(())
}

pub fn increment_learning_reference(conn: &Connection, learning_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE learnings SET times_referenced = times_referenced + 1 WHERE id = ?1",
        [learning_id],
    )?;
    Ok(())
}
