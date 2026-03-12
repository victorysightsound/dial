use crate::budget::{self, ContextItem};
use crate::errors::Result;
use crate::learning::increment_learning_reference;
use crate::prd;
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

    // Get relevant spec sections — prefer prd.db, fall back to spec_sections
    let prd_conn = if prd::prd_db_exists() { prd::get_prd_db().ok() } else { None };

    if let Some(ref prd_db) = prd_conn {
        // PRD: linked section via prd_section_id
        if let Some(ref prd_sid) = task.prd_section_id {
            if let Ok(Some(section)) = prd::prd_get_section(prd_db, prd_sid) {
                context.push(format!("## Relevant Specification ({})\n\n{}", section.title, section.content));
            }
        }

        // PRD: FTS search
        if let Ok(results) = prd::prd_search_sections(prd_db, &task.description) {
            if !results.is_empty() {
                context.push("## Related Specifications\n".to_string());
                for section in results.iter().take(3) {
                    let preview = if section.content.len() > 500 {
                        &section.content[..500]
                    } else {
                        &section.content
                    };
                    context.push(format!("### {} ({})\n{}", section.title, section.section_id, preview));
                }
            }
        }
    } else {
        // Fallback: spec_sections in phase DB
        if let Some(spec_id) = task.spec_section_id {
            let mut stmt = conn.prepare(
                "SELECT content FROM spec_sections WHERE id = ?1",
            )?;

            if let Ok(content) = stmt.query_row([spec_id], |row| row.get::<_, String>(0)) {
                context.push(format!("## Relevant Specification\n\n{}", content));
            }
        }

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

    // Get recent failures with matched solutions (known fixes) — higher priority than general solutions
    let mut stmt = conn.prepare(
        "SELECT DISTINCT s.description, s.confidence, fp.pattern_key
         FROM failures f
         INNER JOIN failure_patterns fp ON f.pattern_id = fp.id
         INNER JOIN solutions s ON s.pattern_id = fp.id
         WHERE f.resolved = 0 AND s.confidence >= ?1
         ORDER BY s.confidence DESC, f.created_at DESC LIMIT 5",
    )?;

    let known_fixes: Vec<(String, f64, String)> = stmt
        .query_map([TRUST_THRESHOLD], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();

    if !known_fixes.is_empty() {
        context.push("## Known Fixes for Recent Failures\n".to_string());
        for (description, confidence, _pattern_key) in known_fixes {
            context.push(format!("- KNOWN FIX (confidence: {:.2}): {}", confidence, description));
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

/// Priority levels for context items (lower = higher priority).
pub const PRIORITY_SIGNS: u32 = 0;
pub const PRIORITY_TASK_SPEC: u32 = 5;
pub const PRIORITY_FTS_SPECS: u32 = 10;
pub const PRIORITY_SUGGESTED_SOLUTIONS: u32 = 15;
pub const PRIORITY_TRUSTED_SOLUTIONS: u32 = 20;
pub const PRIORITY_FAILURES: u32 = 30;
pub const PRIORITY_LEARNINGS: u32 = 40;

/// Gather context as priority-ranked ContextItems for budget-aware assembly.
/// This is the structured alternative to `gather_context()`.
pub fn gather_context_items(conn: &Connection, task: &Task) -> Result<Vec<ContextItem>> {
    let mut items = Vec::new();

    // Signs (highest priority)
    let signs_content = SIGNS.iter()
        .map(|s| format!("- **{}**", s))
        .collect::<Vec<_>>()
        .join("\n");
    items.push(ContextItem::new("Signs (Critical Rules)", &signs_content, PRIORITY_SIGNS));

    // Task-linked and FTS spec sections — prefer prd.db, fall back to spec_sections
    let prd_conn = if prd::prd_db_exists() { prd::get_prd_db().ok() } else { None };

    if let Some(ref prd_db) = prd_conn {
        // PRD: linked section via prd_section_id
        if let Some(ref prd_sid) = task.prd_section_id {
            if let Ok(Some(section)) = prd::prd_get_section(prd_db, prd_sid) {
                items.push(ContextItem::new(
                    &format!("PRD: {}", section.title),
                    &section.content,
                    PRIORITY_TASK_SPEC,
                ));
            }
        }

        // PRD: FTS search
        if let Ok(results) = prd::prd_search_sections(prd_db, &task.description) {
            for section in results.iter().take(3) {
                let preview = if section.content.len() > 500 {
                    &section.content[..500]
                } else {
                    &section.content
                };
                items.push(ContextItem::new(
                    &format!("PRD: {}", section.title),
                    preview,
                    PRIORITY_FTS_SPECS,
                ));
            }
        }
    } else {
        // Fallback: spec_sections in phase DB
        if let Some(spec_id) = task.spec_section_id {
            let mut stmt = conn.prepare(
                "SELECT content FROM spec_sections WHERE id = ?1",
            )?;

            if let Ok(content) = stmt.query_row([spec_id], |row| row.get::<_, String>(0)) {
                items.push(ContextItem::new("Task Specification", &content, PRIORITY_TASK_SPEC));
            }
        }

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

        for (heading, content) in related_specs {
            let preview = if content.len() > 500 { &content[..500] } else { &content };
            items.push(ContextItem::new(
                &format!("Spec: {}", heading),
                preview,
                PRIORITY_FTS_SPECS,
            ));
        }
    }

    // Known fixes for recent failures (higher priority than general solutions)
    let mut stmt = conn.prepare(
        "SELECT DISTINCT s.description, s.confidence, fp.pattern_key
         FROM failures f
         INNER JOIN failure_patterns fp ON f.pattern_id = fp.id
         INNER JOIN solutions s ON s.pattern_id = fp.id
         WHERE f.resolved = 0 AND s.confidence >= ?1
         ORDER BY s.confidence DESC, f.created_at DESC LIMIT 5",
    )?;

    let known_fixes: Vec<(String, f64, String)> = stmt
        .query_map([TRUST_THRESHOLD], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();

    if !known_fixes.is_empty() {
        let content = known_fixes.iter()
            .map(|(desc, confidence, _)| format!("- KNOWN FIX (confidence: {:.2}): {}", confidence, desc))
            .collect::<Vec<_>>()
            .join("\n");
        items.push(ContextItem::new("Known Fixes for Recent Failures", &content, PRIORITY_SUGGESTED_SOLUTIONS));
    }

    // Trusted solutions
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
        let content = solutions.iter()
            .map(|(desc, key)| format!("- **{}**: {}", key, desc))
            .collect::<Vec<_>>()
            .join("\n");
        items.push(ContextItem::new("Trusted Solutions", &content, PRIORITY_TRUSTED_SOLUTIONS));
    }

    // Recent failures
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
        let content = failures.iter()
            .map(|(error_text, key)| {
                let preview = if error_text.len() > 200 { &error_text[..200] } else { error_text };
                format!("- **{}**: {}", key, preview)
            })
            .collect::<Vec<_>>()
            .join("\n");
        items.push(ContextItem::new("Recent Failures", &content, PRIORITY_FAILURES));
    }

    // Learnings
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
        let content = learnings.iter()
            .map(|(id, category, description)| {
                let _ = increment_learning_reference(conn, *id);
                let cat_str = category.as_ref().map(|c| format!("[{}]", c)).unwrap_or_default();
                format!("- {} {}", cat_str, description)
            })
            .collect::<Vec<_>>()
            .join("\n");
        items.push(ContextItem::new("Project Learnings", &content, PRIORITY_LEARNINGS));
    }

    Ok(items)
}

/// Gather context with a token budget. Returns the formatted context string
/// and a list of excluded items (for warning events).
pub fn gather_context_budgeted(
    conn: &Connection,
    task: &Task,
    token_budget: usize,
) -> Result<(String, Vec<String>)> {
    let items = gather_context_items(conn, task)?;
    let (included, excluded) = budget::assemble_context(&items, token_budget);

    let formatted = budget::format_context(&included);
    let excluded_labels: Vec<String> = excluded.iter().map(|item| item.label.clone()).collect();

    Ok((formatted, excluded_labels))
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
3. **Signal completion** by writing a JSON file to `.dial/signal.json`:

```json
{{
  "signals": [
    {{"type": "learning", "category": "<category>", "description": "<what you learned>"}},
    {{"type": "complete", "summary": "<summary of what was done>"}}
  ],
  "timestamp": "<ISO 8601 timestamp>"
}}
```

Signal types:
- `complete` — task finished: `{{"type": "complete", "summary": "..."}}`
- `blocked` — cannot proceed: `{{"type": "blocked", "reason": "..."}}`
- `learning` — valuable insight: `{{"type": "learning", "category": "...", "description": "..."}}`

Write the file as the **last step** before exiting. Include any learnings alongside your completion or blocked signal. If you cannot write the file, fall back to printing `DIAL_COMPLETE: <summary>`, `DIAL_BLOCKED: <reason>`, or `DIAL_LEARNING: <category>: <description>` as text output.

Do NOT deviate from this task. Do NOT start other tasks.
"#,
        id = task.id,
        description = task.description,
        context = context
    );

    Ok(prompt)
}
