use crate::budget::{self, ContextItem};
use crate::errors::Result;
use crate::learning::{increment_learning_reference, learnings_for_pattern};
use crate::prd;
use crate::task::find_similar_completed_tasks;
use crate::task::models::Task;
use crate::TRUST_THRESHOLD;
use rusqlite::Connection;

/// Extract error, diff_stat, and diff from structured iteration notes.
/// Returns `Some((error, diff_stat, diff))` if the notes contain FAILED_DIFF markers.
pub fn extract_failed_diff_parts(notes: &str) -> Option<(String, String, String)> {
    let stat_marker = "\nFAILED_DIFF_STAT:\n";
    let diff_marker = "\nFAILED_DIFF:\n";

    let stat_pos = notes.find(stat_marker)?;
    let diff_pos = notes.find(diff_marker)?;

    let error = notes[..stat_pos].to_string();
    let stat = notes[stat_pos + stat_marker.len()..diff_pos].to_string();
    let diff = notes[diff_pos + diff_marker.len()..].to_string();

    Some((error, stat, diff))
}

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

    // For retry attempts: include previous failed attempt's diff
    {
        let prev_failed_notes: Option<String> = conn
            .query_row(
                "SELECT notes FROM iterations WHERE task_id = ?1 AND status = 'failed' ORDER BY id DESC LIMIT 1",
                [task.id],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        if let Some(notes) = prev_failed_notes {
            if let Some((error, diff_stat, diff)) = extract_failed_diff_parts(&notes) {
                context.push(format!(
                    "## Previous Failed Attempt\n\nPREVIOUS ATTEMPT (failed):\nError: {}\nChanges attempted:\n{}\n{}\nDO NOT repeat this approach.",
                    error.trim(),
                    diff_stat.trim(),
                    diff.trim()
                ));
            }
        }
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

    // Get similar completed tasks
    let similar = find_similar_completed_tasks(conn, &task.description, 3)?;
    if !similar.is_empty() {
        context.push("## Similar Completed Tasks\n".to_string());
        for (similar_task, approach) in &similar {
            context.push(format!(
                "SIMILAR COMPLETED TASK: {}\n{}",
                similar_task.description, approach
            ));
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

        // Collect pattern IDs for pattern-linked learnings
        let mut seen_pattern_ids: Vec<i64> = Vec::new();

        for (error_text, pattern_key) in &failures {
            let preview = if error_text.len() > 200 {
                &error_text[..200]
            } else {
                error_text
            };
            context.push(format!("- **{}**: {}", pattern_key, preview));
        }

        // Gather pattern-linked learnings for each failure's pattern
        {
            let mut pattern_stmt = conn.prepare(
                "SELECT DISTINCT f.pattern_id, fp.pattern_key
                 FROM failures f
                 INNER JOIN failure_patterns fp ON f.pattern_id = fp.id
                 WHERE f.resolved = 0
                 ORDER BY f.created_at DESC LIMIT 5",
            )?;

            let pattern_ids: Vec<(i64, String)> = pattern_stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();

            let mut pattern_learnings_block = Vec::new();
            for (pid, pkey) in &pattern_ids {
                if seen_pattern_ids.contains(pid) {
                    continue;
                }
                seen_pattern_ids.push(*pid);
                let plearnings = learnings_for_pattern(conn, *pid)?;
                for pl in plearnings.iter().take(3) {
                    pattern_learnings_block.push(format!(
                        "- LEARNING (from pattern: {}): {}",
                        pkey, pl.description
                    ));
                    let _ = increment_learning_reference(conn, pl.id);
                }
            }

            if !pattern_learnings_block.is_empty() {
                context.push("## Pattern-Linked Learnings\n".to_string());
                for line in pattern_learnings_block {
                    context.push(line);
                }
            }
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
pub const PRIORITY_SIMILAR_TASKS: u32 = 25;
pub const PRIORITY_FAILURES: u32 = 30;
pub const PRIORITY_PATTERN_LEARNINGS: u32 = 35;
pub const PRIORITY_LEARNINGS: u32 = 40;

/// Gather context as priority-ranked ContextItems for budget-aware assembly.
/// This is the structured alternative to `gather_context()`.
pub fn gather_context_items(conn: &Connection, task: &Task) -> Result<Vec<ContextItem>> {
    gather_context_items_impl(conn, task, true)
}

/// Gather context items WITHOUT side effects (no learning reference increments).
/// Used by dry-run/preview mode to inspect context without mutating the DB.
pub fn gather_context_items_pure(conn: &Connection, task: &Task) -> Result<Vec<ContextItem>> {
    gather_context_items_impl(conn, task, false)
}

fn gather_context_items_impl(conn: &Connection, task: &Task, track_references: bool) -> Result<Vec<ContextItem>> {
    let mut items = Vec::new();

    // Signs (highest priority)
    let signs_content = SIGNS.iter()
        .map(|s| format!("- **{}**", s))
        .collect::<Vec<_>>()
        .join("\n");
    items.push(ContextItem::new("Signs (Critical Rules)", &signs_content, PRIORITY_SIGNS));

    // For retry attempts: include previous failed attempt's diff
    {
        let prev_failed_notes: Option<String> = conn
            .query_row(
                "SELECT notes FROM iterations WHERE task_id = ?1 AND status = 'failed' ORDER BY id DESC LIMIT 1",
                [task.id],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        if let Some(notes) = prev_failed_notes {
            if let Some((error, diff_stat, diff)) = extract_failed_diff_parts(&notes) {
                let content = format!(
                    "PREVIOUS ATTEMPT (failed):\nError: {}\nChanges attempted:\n{}\n{}\nDO NOT repeat this approach.",
                    error.trim(),
                    diff_stat.trim(),
                    diff.trim()
                );
                items.push(ContextItem::new(
                    "Previous Failed Attempt",
                    &content,
                    budget::FAILED_DIFF_PRIORITY,
                ));
            }
        }
    }

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

    // Similar completed tasks
    let similar = find_similar_completed_tasks(conn, &task.description, 3)?;
    if !similar.is_empty() {
        let content = similar
            .iter()
            .map(|(t, approach)| {
                format!("SIMILAR COMPLETED TASK: {}\n{}", t.description, approach)
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        items.push(ContextItem::new(
            "Similar Completed Tasks",
            &content,
            PRIORITY_SIMILAR_TASKS,
        ));
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

        // Pattern-linked learnings
        let mut pattern_stmt = conn.prepare(
            "SELECT DISTINCT f.pattern_id, fp.pattern_key
             FROM failures f
             INNER JOIN failure_patterns fp ON f.pattern_id = fp.id
             WHERE f.resolved = 0
             ORDER BY f.created_at DESC LIMIT 5",
        )?;

        let pattern_ids: Vec<(i64, String)> = pattern_stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let mut seen_pattern_ids: Vec<i64> = Vec::new();
        let mut pattern_learnings_lines = Vec::new();
        for (pid, pkey) in &pattern_ids {
            if seen_pattern_ids.contains(pid) {
                continue;
            }
            seen_pattern_ids.push(*pid);
            let plearnings = learnings_for_pattern(conn, *pid)?;
            for pl in plearnings.iter().take(3) {
                pattern_learnings_lines.push(format!(
                    "- LEARNING (from pattern: {}): {}",
                    pkey, pl.description
                ));
                if track_references {
                    let _ = increment_learning_reference(conn, pl.id);
                }
            }
        }

        if !pattern_learnings_lines.is_empty() {
            let content = pattern_learnings_lines.join("\n");
            items.push(ContextItem::new("Pattern-Linked Learnings", &content, PRIORITY_PATTERN_LEARNINGS));
        }
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
                if track_references {
                    let _ = increment_learning_reference(conn, *id);
                }
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
- `complete`: task finished with `{{"type": "complete", "summary": "..."}}`
- `blocked`: cannot proceed with `{{"type": "blocked", "reason": "..."}}`
- `learning`: valuable insight with `{{"type": "learning", "category": "...", "description": "..."}}`

Use only ASCII hyphen-minus characters in commands, flags, JSON, and code. Never use Unicode dash punctuation where syntax matters.

Write the file as the **last step** before exiting. Include any learnings alongside your completion or blocked signal. If you cannot write the file, fall back to printing `DIAL_COMPLETE: <summary>`, `DIAL_BLOCKED: <reason>`, or `DIAL_LEARNING: <category>: <description>` as text output.

Do NOT deviate from this task. Do NOT start other tasks.
"#,
        id = task.id,
        description = task.description,
        context = context
    );

    Ok(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use crate::learning::add_learning_with_conn;
    use crate::task::models::{Task, TaskStatus};

    /// Set up an in-memory DB with base schema + migration columns needed for context tests.
    fn setup_context_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .unwrap();
        conn.execute_batch(schema::SCHEMA).unwrap();
        // Add migration columns
        conn.execute_batch(
            r#"
            ALTER TABLE learnings ADD COLUMN pattern_id INTEGER REFERENCES failure_patterns(id);
            ALTER TABLE learnings ADD COLUMN iteration_id INTEGER REFERENCES iterations(id);
            ALTER TABLE tasks ADD COLUMN prd_section_id TEXT;
            ALTER TABLE tasks ADD COLUMN total_attempts INTEGER DEFAULT 0;
            ALTER TABLE tasks ADD COLUMN total_failures INTEGER DEFAULT 0;
            ALTER TABLE tasks ADD COLUMN last_failure_at TEXT;
            ALTER TABLE failure_patterns ADD COLUMN regex_pattern TEXT;
            ALTER TABLE failure_patterns ADD COLUMN status TEXT NOT NULL DEFAULT 'trusted';
            ALTER TABLE solutions ADD COLUMN source TEXT NOT NULL DEFAULT 'auto-learned';
            ALTER TABLE solutions ADD COLUMN last_validated_at TEXT;
            ALTER TABLE solutions ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
            "#,
        )
        .unwrap();
        conn
    }

    fn make_test_task(id: i64) -> Task {
        Task {
            id,
            description: "test task".to_string(),
            status: TaskStatus::InProgress,
            priority: 5,
            blocked_by: None,
            spec_section_id: None,
            prd_section_id: None,
            created_at: "2026-01-01T00:00:00".to_string(),
            started_at: None,
            completed_at: None,
            total_attempts: 0,
            total_failures: 0,
            last_failure_at: None,
        }
    }

    #[test]
    fn test_pattern_linked_learnings_included_in_context() {
        let conn = setup_context_test_db();

        // Create task
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('test task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Create iteration
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number) VALUES (?1, 1)",
            [task_id],
        )
        .unwrap();
        let iteration_id = conn.last_insert_rowid();

        // Create pattern
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('CompileErr', 'Compile error', 'build')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        // Create unresolved failure
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'error[E0308]')",
            rusqlite::params![iteration_id, pattern_id],
        )
        .unwrap();

        // Add pattern-linked learning
        add_learning_with_conn(
            &conn,
            "Always check type annotations when E0308 occurs",
            Some("pattern"),
            Some(pattern_id),
            Some(iteration_id),
        )
        .unwrap();

        // Add an unlinked learning too
        add_learning_with_conn(
            &conn,
            "General learning not linked",
            Some("other"),
            None,
            None,
        )
        .unwrap();

        let task = make_test_task(task_id);
        let context = gather_context(&conn, &task).unwrap();

        // Should contain the pattern-linked learning with the expected format
        assert!(
            context.contains("LEARNING (from pattern: CompileErr): Always check type annotations"),
            "Context should contain pattern-linked learning. Got:\n{}",
            context
        );
    }

    #[test]
    fn test_pattern_linked_learnings_in_context_items() {
        let conn = setup_context_test_db();

        // Create task
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('test task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Create iteration
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number) VALUES (?1, 1)",
            [task_id],
        )
        .unwrap();
        let iteration_id = conn.last_insert_rowid();

        // Create pattern
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('TestFail', 'Test failure', 'test')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        // Create unresolved failure
        conn.execute(
            "INSERT INTO failures (iteration_id, pattern_id, error_text) VALUES (?1, ?2, 'FAILED test_foo')",
            rusqlite::params![iteration_id, pattern_id],
        )
        .unwrap();

        // Add pattern-linked learning
        add_learning_with_conn(
            &conn,
            "Run with --nocapture for verbose test output",
            Some("test"),
            Some(pattern_id),
            None,
        )
        .unwrap();

        let task = make_test_task(task_id);
        let items = gather_context_items(&conn, &task).unwrap();

        // Should have a "Pattern-Linked Learnings" item at priority 35
        let pattern_item = items.iter().find(|i| i.label == "Pattern-Linked Learnings");
        assert!(
            pattern_item.is_some(),
            "Should have a Pattern-Linked Learnings context item"
        );
        let item = pattern_item.unwrap();
        assert_eq!(item.priority, PRIORITY_PATTERN_LEARNINGS);
        assert!(item.content.contains("LEARNING (from pattern: TestFail)"));
        assert!(item.content.contains("Run with --nocapture"));
    }

    #[test]
    fn test_no_pattern_linked_learnings_when_no_failures() {
        let conn = setup_context_test_db();

        // Create task with no failures
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('clean task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Add a learning linked to a pattern (but no failures reference it)
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category) VALUES ('SomeErr', 'Some err', 'build')",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();
        add_learning_with_conn(
            &conn,
            "This should not appear",
            Some("pattern"),
            Some(pattern_id),
            None,
        )
        .unwrap();

        let task = make_test_task(task_id);
        let context = gather_context(&conn, &task).unwrap();

        // No failures, so no pattern-linked learnings section
        assert!(
            !context.contains("Pattern-Linked Learnings"),
            "Should not have pattern-linked learnings without failures"
        );
    }

    #[test]
    fn test_gather_context_items_pure_no_reference_increment() {
        let conn = setup_context_test_db();

        // Create task
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('pure test task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Add a learning
        add_learning_with_conn(
            &conn,
            "Pure mode should not increment this",
            Some("test"),
            None,
            None,
        )
        .unwrap();
        let learning_id = conn.last_insert_rowid();

        // Verify initial reference count is 0
        let initial_refs: i64 = conn.query_row(
            "SELECT times_referenced FROM learnings WHERE id = ?1",
            [learning_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(initial_refs, 0);

        let task = make_test_task(task_id);

        // Call pure version
        let items = gather_context_items_pure(&conn, &task).unwrap();
        assert!(!items.is_empty(), "Should return context items");

        // Verify reference count is still 0
        let after_refs: i64 = conn.query_row(
            "SELECT times_referenced FROM learnings WHERE id = ?1",
            [learning_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(after_refs, 0, "Pure mode should not increment references");
    }

    #[test]
    fn test_extract_failed_diff_parts_valid() {
        let notes = "build error: something failed\nFAILED_DIFF_STAT:\n 2 files changed, 10 insertions(+), 3 deletions(-)\nFAILED_DIFF:\n--- a/src/main.rs\n+++ b/src/main.rs\n+new broken line";
        let result = extract_failed_diff_parts(notes);
        assert!(result.is_some());
        let (error, stat, diff) = result.unwrap();
        assert_eq!(error, "build error: something failed");
        assert!(stat.contains("2 files changed"));
        assert!(diff.contains("+new broken line"));
    }

    #[test]
    fn test_extract_failed_diff_parts_no_markers() {
        let notes = "just an error without diff markers";
        let result = extract_failed_diff_parts(notes);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_failed_diff_parts_empty_diff() {
        let notes = "error msg\nFAILED_DIFF_STAT:\n\nFAILED_DIFF:\n";
        let result = extract_failed_diff_parts(notes);
        assert!(result.is_some());
        let (error, stat, diff) = result.unwrap();
        assert_eq!(error, "error msg");
        assert_eq!(stat, "");
        assert_eq!(diff, "");
    }

    #[test]
    fn test_diff_truncation_preserved_in_extraction() {
        // Verify extraction works with a large diff (truncation happens at storage time)
        let long_diff = "x".repeat(3000);
        let notes = format!("error\nFAILED_DIFF_STAT:\nstat line\nFAILED_DIFF:\n{}", long_diff);
        let result = extract_failed_diff_parts(&notes);
        assert!(result.is_some());
        let (_, _, diff) = result.unwrap();
        assert_eq!(diff.len(), 3000);
    }

    #[test]
    fn test_retry_context_includes_previous_failed_diff() {
        let conn = setup_context_test_db();

        // Create task
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('retry task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Create a failed iteration with diff in notes
        let notes = "build error\nFAILED_DIFF_STAT:\n 1 file changed, 5 insertions(+)\nFAILED_DIFF:\n--- a/src/lib.rs\n+++ b/src/lib.rs\n+broken code here";
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, status, notes) VALUES (?1, 1, 'failed', ?2)",
            rusqlite::params![task_id, notes],
        )
        .unwrap();

        let task = make_test_task(task_id);
        let context = gather_context(&conn, &task).unwrap();

        assert!(
            context.contains("PREVIOUS ATTEMPT (failed):"),
            "Context should contain previous failed attempt section. Got:\n{}",
            context
        );
        assert!(
            context.contains("DO NOT repeat this approach"),
            "Context should contain warning not to repeat. Got:\n{}",
            context
        );
        assert!(
            context.contains("broken code here"),
            "Context should contain the diff content. Got:\n{}",
            context
        );
    }

    #[test]
    fn test_retry_context_items_includes_previous_diff_at_priority_12() {
        let conn = setup_context_test_db();

        // Create task
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('retry items task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Create a failed iteration with diff in notes
        let notes = "test failure\nFAILED_DIFF_STAT:\n 2 files changed\nFAILED_DIFF:\n+added bad code";
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, status, notes) VALUES (?1, 1, 'failed', ?2)",
            rusqlite::params![task_id, notes],
        )
        .unwrap();

        let task = make_test_task(task_id);
        let items = gather_context_items(&conn, &task).unwrap();

        let diff_item = items.iter().find(|i| i.label == "Previous Failed Attempt");
        assert!(
            diff_item.is_some(),
            "Should have a Previous Failed Attempt context item"
        );
        let item = diff_item.unwrap();
        assert_eq!(item.priority, crate::budget::FAILED_DIFF_PRIORITY);
        assert!(item.content.contains("PREVIOUS ATTEMPT (failed):"));
        assert!(item.content.contains("DO NOT repeat this approach"));
        assert!(item.content.contains("+added bad code"));
    }

    #[test]
    fn test_no_retry_context_when_no_failed_iterations() {
        let conn = setup_context_test_db();

        // Create task with no failed iterations
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('fresh task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        let task = make_test_task(task_id);
        let context = gather_context(&conn, &task).unwrap();

        assert!(
            !context.contains("PREVIOUS ATTEMPT"),
            "Should not have previous attempt section without failed iterations"
        );
    }

    #[test]
    fn test_no_retry_context_when_failed_without_diff() {
        let conn = setup_context_test_db();

        // Create task
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('no diff task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Create a failed iteration WITHOUT diff markers in notes
        conn.execute(
            "INSERT INTO iterations (task_id, attempt_number, status, notes) VALUES (?1, 1, 'failed', 'plain error')",
            rusqlite::params![task_id],
        )
        .unwrap();

        let task = make_test_task(task_id);
        let context = gather_context(&conn, &task).unwrap();

        assert!(
            !context.contains("PREVIOUS ATTEMPT"),
            "Should not have previous attempt section when notes lack diff markers"
        );
    }

    #[test]
    fn test_gather_context_items_does_increment_references() {
        let conn = setup_context_test_db();

        // Create task
        conn.execute(
            "INSERT INTO tasks (description, status) VALUES ('ref test task', 'in_progress')",
            [],
        )
        .unwrap();
        let task_id = conn.last_insert_rowid();

        // Add a learning
        add_learning_with_conn(
            &conn,
            "Normal mode should increment this",
            Some("test"),
            None,
            None,
        )
        .unwrap();
        let learning_id = conn.last_insert_rowid();

        let task = make_test_task(task_id);

        // Call normal version (with side effects)
        let _items = gather_context_items(&conn, &task).unwrap();

        // Verify reference count was incremented
        let after_refs: i64 = conn.query_row(
            "SELECT times_referenced FROM learnings WHERE id = ?1",
            [learning_id],
            |row| row.get(0),
        ).unwrap();
        assert!(after_refs > 0, "Normal mode should increment references");
    }

    #[test]
    fn test_generate_subagent_prompt_uses_ascii_dash_guidance() {
        let conn = setup_context_test_db();
        let task = make_test_task(1);

        let prompt = generate_subagent_prompt(&conn, &task).unwrap();

        assert!(prompt.contains("Use only ASCII hyphen-minus characters"));
        assert!(!prompt.contains('—'));
        assert!(!prompt.contains('–'));
    }
}
