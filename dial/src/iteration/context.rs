use crate::errors::Result;
use crate::learning::increment_learning_reference;
use crate::task::models::Task;
use crate::TRUST_THRESHOLD;
use rusqlite::Connection;

pub fn gather_context(conn: &Connection, task: &Task) -> Result<String> {
    let mut context = Vec::new();

    // Get relevant spec sections
    if let Some(spec_id) = task.spec_section_id {
        let mut stmt = conn.prepare(
            "SELECT content FROM spec_sections WHERE id = ?1",
        )?;

        if let Ok(content) = stmt.query_row([spec_id], |row| row.get::<_, String>(0)) {
            context.push(format!("## Relevant Specification\n\n{}", content));
        }
    }

    // Search for task-related specs
    let mut stmt = conn.prepare(
        "SELECT s.heading_path, s.content
         FROM spec_sections s
         INNER JOIN spec_sections_fts fts ON s.id = fts.rowid
         WHERE spec_sections_fts MATCH ?1
         ORDER BY rank LIMIT 3",
    )?;

    let related_specs: Vec<(String, String)> = stmt
        .query_map([&task.description], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    if !related_specs.is_empty() {
        context.push("## Related Specifications\n".to_string());
        for (heading, content) in related_specs {
            let preview = if content.len() > 500 {
                &content[..500]
            } else {
                &content
            };
            context.push(format!("### {}\n{}", heading, preview));
        }
    }

    // Get trusted solutions for common patterns
    let mut stmt = conn.prepare(
        "SELECT s.description, fp.pattern_key
         FROM solutions s
         INNER JOIN failure_patterns fp ON s.pattern_id = fp.id
         WHERE s.confidence >= ?1
         ORDER BY fp.occurrence_count DESC LIMIT 5",
    )?;

    let solutions: Vec<(String, String)> = stmt
        .query_map([TRUST_THRESHOLD], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    if !solutions.is_empty() {
        context.push("## Trusted Solutions (apply if relevant failure occurs)\n".to_string());
        for (description, pattern_key) in solutions {
            context.push(format!("- **{}**: {}", pattern_key, description));
        }
    }

    // Get recent failures to avoid
    let mut stmt = conn.prepare(
        "SELECT f.error_text, fp.pattern_key
         FROM failures f
         INNER JOIN failure_patterns fp ON f.pattern_id = fp.id
         WHERE f.resolved = 0
         ORDER BY f.created_at DESC LIMIT 5",
    )?;

    let failures: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    if !failures.is_empty() {
        context.push("## Recent Unresolved Failures (avoid these)\n".to_string());
        for (error_text, pattern_key) in failures {
            let preview = if error_text.len() > 200 {
                &error_text[..200]
            } else {
                &error_text
            };
            context.push(format!("- **{}**: {}", pattern_key, preview));
        }
    }

    // Get project learnings
    let mut stmt = conn.prepare(
        "SELECT id, category, description
         FROM learnings
         ORDER BY times_referenced DESC, discovered_at DESC
         LIMIT 10",
    )?;

    let learnings: Vec<(i64, Option<String>, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();

    if !learnings.is_empty() {
        context.push("## Project Learnings (apply these patterns)\n".to_string());
        for (id, category, description) in learnings {
            let cat_str = category.map(|c| format!("[{}]", c)).unwrap_or_default();
            context.push(format!("- {} {}", cat_str, description));
            // Increment reference count
            let _ = increment_learning_reference(conn, id);
        }
    }

    Ok(context.join("\n\n"))
}
