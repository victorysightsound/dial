use crate::errors::Result;
use crate::learning::increment_learning_reference;
use crate::task::models::Task;
use crate::TRUST_THRESHOLD;
use rusqlite::Connection;

/// Behavioral guardrails ("signs") that prevent context rot
/// by reminding the agent of critical rules at the start of each task.
const SIGNS: &[&str] = &[
    "ONE TASK ONLY: Complete exactly this task. No scope creep.",
    "SEARCH BEFORE CREATE: Always search for existing files/functions before creating new ones.",
    "NO PLACEHOLDERS: Every implementation must be complete. No TODO, FIXME, or stub code.",
    "VALIDATE BEFORE DONE: Run `dial validate` after implementing. Don't mark complete without testing.",
    "RECORD LEARNINGS: After success, capture what you learned with `dial learn \"...\" -c category`.",
    "FAIL FAST: If blocked or confused, stop and ask rather than guessing.",
];

pub fn gather_context(conn: &Connection, task: &Task) -> Result<String> {
    gather_context_impl(conn, task, true)
}

pub fn gather_context_without_signs(conn: &Connection, task: &Task) -> Result<String> {
    gather_context_impl(conn, task, false)
}

fn gather_context_impl(conn: &Connection, task: &Task, include_signs: bool) -> Result<String> {
    let mut context = Vec::new();

    // Add behavioral signs first (most important for context rot prevention)
    if include_signs {
        context.push("## ⚠️ SIGNS (Critical Rules)\n".to_string());
        for sign in SIGNS {
            context.push(format!("- **{}**", sign));
        }
        context.push(String::new());
    }

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

/// Generate a fresh context prompt for spawning a sub-agent (orchestrator mode).
/// This produces a self-contained prompt that can be used to spawn a fresh AI session.
pub fn generate_subagent_prompt(conn: &Connection, task: &Task) -> Result<String> {
    let context = gather_context(conn, task)?;

    let prompt = format!(
        r#"# DIAL Sub-Agent Task

You are a fresh AI agent spawned by DIAL to complete ONE task with clean context.

## Your Task
**Task #{id}:** {description}

{context}

## Instructions

1. **Implement** the task completely (no placeholders)
2. **Test** your implementation locally if possible
3. When done, output: `DIAL_COMPLETE: <summary of what was done>`
4. If blocked, output: `DIAL_BLOCKED: <what is blocking>`
5. If you learned something valuable, output: `DIAL_LEARNING: <category>: <what you learned>`

Do NOT deviate from this task. Do NOT start other tasks.
"#,
        id = task.id,
        description = task.description,
        context = context
    );

    Ok(prompt)
}
