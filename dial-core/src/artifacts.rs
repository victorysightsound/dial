use crate::db::get_dial_dir;
use crate::errors::Result;
use crate::TRUST_THRESHOLD;
use chrono::Local;
use rusqlite::Connection;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressOutcome {
    Completed,
    Failed,
    Blocked,
    NoSignal,
}

impl ProgressOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::NoSignal => "no-signal",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProgressLogEntry {
    pub task_id: i64,
    pub task_description: String,
    pub iteration_id: i64,
    pub attempt_number: i32,
    pub outcome: ProgressOutcome,
    pub summary: Option<String>,
    pub changed_files_summary: Option<String>,
    pub commit_hash: Option<String>,
    pub learnings: Vec<(String, String)>,
}

pub fn progress_log_path() -> PathBuf {
    get_dial_dir().join("progress.md")
}

pub fn patterns_path() -> PathBuf {
    get_dial_dir().join("patterns.md")
}

pub fn task_ledger_path() -> PathBuf {
    get_dial_dir().join("task-ledger.md")
}

pub fn append_progress_log_entry(entry: &ProgressLogEntry) -> Result<()> {
    fs::create_dir_all(get_dial_dir())?;
    ensure_progress_log_header(&progress_log_path())?;

    let mut file = OpenOptions::new().append(true).open(progress_log_path())?;

    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S %z");
    writeln!(
        file,
        "## {} - Task #{} (iteration #{}, attempt {})",
        timestamp, entry.task_id, entry.iteration_id, entry.attempt_number
    )?;
    writeln!(file)?;
    writeln!(file, "- Result: `{}`", entry.outcome.as_str())?;
    writeln!(file, "- Task: {}", entry.task_description)?;

    if let Some(summary) = entry
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        writeln!(file, "- Summary: {}", summary)?;
    }

    if let Some(commit_hash) = entry
        .commit_hash
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let short = &commit_hash[..commit_hash.len().min(8)];
        writeln!(file, "- Commit: `{}`", short)?;
    }

    if let Some(changed) = entry
        .changed_files_summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        writeln!(file)?;
        writeln!(file, "### Changed Files")?;
        writeln!(file)?;
        writeln!(file, "```text")?;
        writeln!(file, "{}", changed)?;
        writeln!(file, "```")?;
    }

    if !entry.learnings.is_empty() {
        writeln!(file)?;
        writeln!(file, "### Learnings")?;
        writeln!(file)?;
        let mut seen = HashSet::new();
        for (category, description) in &entry.learnings {
            let key = (
                category.trim().to_ascii_lowercase(),
                description.trim().to_string(),
            );
            if !seen.insert(key) {
                continue;
            }
            if category.trim().is_empty() {
                writeln!(file, "- {}", description.trim())?;
            } else {
                writeln!(file, "- [{}] {}", category.trim(), description.trim())?;
            }
        }
    }

    writeln!(file)?;
    writeln!(file, "---")?;
    writeln!(file)?;
    Ok(())
}

pub fn sync_patterns_digest(conn: &Connection) -> Result<String> {
    let content = render_patterns_digest(conn)?;
    fs::create_dir_all(get_dial_dir())?;
    fs::write(patterns_path(), &content)?;
    Ok(content)
}

pub fn render_patterns_digest(conn: &Connection) -> Result<String> {
    let sections = collect_pattern_sections(conn, 5, 4)?;
    let mut out = String::new();
    out.push_str("# DIAL Codebase Patterns\n\n");
    out.push_str(&format!(
        "Generated: {}\n\n",
        Local::now().format("%Y-%m-%d %H:%M:%S %z")
    ));

    if sections.is_empty() {
        out.push_str("No stable patterns recorded yet.\n");
        return Ok(out);
    }

    for (title, lines) in sections {
        out.push_str(&format!("## {}\n\n", title));
        for line in lines {
            out.push_str("- ");
            out.push_str(&line);
            out.push('\n');
        }
        out.push('\n');
    }

    Ok(out)
}

pub fn render_patterns_context(conn: &Connection) -> Result<Option<String>> {
    let sections = collect_pattern_sections(conn, 3, 3)?;
    if sections.is_empty() {
        return Ok(None);
    }

    let mut out = String::new();
    for (idx, (title, lines)) in sections.into_iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{}:\n", title));
        for line in lines {
            out.push_str("- ");
            out.push_str(&line);
            out.push('\n');
        }
    }

    Ok(Some(out.trim().to_string()))
}

pub fn sync_task_ledger(conn: &Connection) -> Result<String> {
    let content = render_task_ledger(conn)?;
    fs::create_dir_all(get_dial_dir())?;
    fs::write(task_ledger_path(), &content)?;
    Ok(content)
}

pub fn render_task_ledger(conn: &Connection) -> Result<String> {
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
        [],
        |row| row.get(0),
    )?;
    let in_progress: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'in_progress'",
        [],
        |row| row.get(0),
    )?;
    let completed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'completed'",
        [],
        |row| row.get(0),
    )?;
    let blocked: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'blocked'",
        [],
        |row| row.get(0),
    )?;
    let cancelled: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'cancelled'",
        [],
        |row| row.get(0),
    )?;

    let current: Option<(i64, String, i32)> = conn
        .query_row(
            "SELECT t.id, t.description, i.attempt_number
             FROM tasks t
             INNER JOIN iterations i ON i.task_id = t.id
             WHERE i.status = 'in_progress'
             ORDER BY i.id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();

    let ready: Vec<(i64, String)> = conn
        .prepare(
            "SELECT id, description
             FROM tasks WHERE status = 'pending'
             AND id NOT IN (
                 SELECT td.task_id FROM task_dependencies td
                 INNER JOIN tasks dep ON dep.id = td.depends_on_id
                 WHERE dep.status != 'completed'
             )
             ORDER BY priority, id LIMIT 8",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let blocked_tasks: Vec<(i64, String, Option<String>)> = conn
        .prepare(
            "SELECT id, description, blocked_by
             FROM tasks
             WHERE status = 'blocked'
             ORDER BY priority, id LIMIT 8",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let recent_completed: Vec<(i64, String, Option<String>)> = conn
        .prepare(
            "SELECT id, description, completed_at
             FROM tasks
             WHERE status = 'completed'
             ORDER BY completed_at DESC, id DESC LIMIT 5",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let mut out = String::new();
    out.push_str("# DIAL Task Ledger\n\n");
    out.push_str(&format!(
        "Generated: {}\n\n",
        Local::now().format("%Y-%m-%d %H:%M:%S %z")
    ));

    out.push_str("## Summary\n\n");
    out.push_str(&format!("- Pending: {}\n", pending));
    out.push_str(&format!("- In Progress: {}\n", in_progress));
    out.push_str(&format!("- Completed: {}\n", completed));
    out.push_str(&format!("- Blocked: {}\n", blocked));
    out.push_str(&format!("- Cancelled: {}\n", cancelled));
    out.push('\n');

    out.push_str("## Current Work\n\n");
    match current {
        Some((task_id, description, attempt)) => {
            out.push_str(&format!(
                "- Task #{} (attempt {}): {}\n\n",
                task_id, attempt, description
            ));
        }
        None => out.push_str("No iteration in progress.\n\n"),
    }

    out.push_str("## Ready Next\n\n");
    if ready.is_empty() {
        out.push_str("No ready pending tasks.\n\n");
    } else {
        for (task_id, description) in ready {
            out.push_str(&format!("- Task #{}: {}\n", task_id, description));
        }
        out.push('\n');
    }

    out.push_str("## Blocked Tasks\n\n");
    if blocked_tasks.is_empty() {
        out.push_str("No blocked tasks.\n\n");
    } else {
        for (task_id, description, blocked_by) in blocked_tasks {
            match blocked_by
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(reason) => out.push_str(&format!(
                    "- Task #{}: {} ({})\n",
                    task_id, description, reason
                )),
                None => out.push_str(&format!("- Task #{}: {}\n", task_id, description)),
            }
        }
        out.push('\n');
    }

    out.push_str("## Recently Completed\n\n");
    if recent_completed.is_empty() {
        out.push_str("No completed tasks yet.\n");
    } else {
        for (task_id, description, completed_at) in recent_completed {
            let completed_on = completed_at
                .as_deref()
                .and_then(|ts| ts.get(..10))
                .unwrap_or("unknown");
            out.push_str(&format!(
                "- Task #{}: {} ({})\n",
                task_id, description, completed_on
            ));
        }
    }

    Ok(out)
}

pub fn sync_operator_artifacts(conn: &Connection) -> Result<()> {
    let _ = sync_patterns_digest(conn)?;
    let _ = sync_task_ledger(conn)?;
    Ok(())
}

pub fn tail_progress_log(limit: usize) -> Result<Option<String>> {
    let path = progress_log_path();
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;
    if limit == 0 {
        return Ok(Some(content));
    }

    let entries = split_progress_entries(&content);
    if entries.is_empty() {
        return Ok(Some(content));
    }

    let total = entries.len();
    let start = total.saturating_sub(limit);
    let mut out = String::from("# DIAL Progress Log\n\n");
    for entry in entries.into_iter().skip(start) {
        out.push_str(&entry);
        out.push_str("\n\n");
    }
    Ok(Some(out.trim_end().to_string()))
}

fn ensure_progress_log_header(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(
        path,
        format!(
            "# DIAL Progress Log\n\nStarted: {}\n\n---\n\n",
            Local::now().format("%Y-%m-%d %H:%M:%S %z")
        ),
    )?;
    Ok(())
}

fn split_progress_entries(content: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = Vec::new();

    for line in content.lines() {
        if line.starts_with("## ") {
            if !current.is_empty() {
                entries.push(current.join("\n").trim().to_string());
                current.clear();
            }
        }
        if !current.is_empty() || line.starts_with("## ") {
            current.push(line.to_string());
        }
    }

    if !current.is_empty() {
        entries.push(current.join("\n").trim().to_string());
    }

    entries
}

fn collect_pattern_sections(
    conn: &Connection,
    learning_limit: usize,
    solution_limit: usize,
) -> Result<Vec<(String, Vec<String>)>> {
    let category_specs = [
        ("pattern", "Reusable Patterns"),
        ("gotcha", "Gotchas"),
        ("test", "Testing Conventions"),
        ("build", "Build Conventions"),
        ("setup", "Environment Requirements"),
        ("tool", "Tooling Notes"),
    ];

    let mut sections = Vec::new();
    for (category, title) in category_specs {
        let lines = top_learnings_for_category(conn, category, learning_limit)?;
        if !lines.is_empty() {
            sections.push((title.to_string(), lines));
        }
    }

    let trusted_solutions = top_trusted_solutions(conn, solution_limit)?;
    if !trusted_solutions.is_empty() {
        sections.push(("Trusted Solutions".to_string(), trusted_solutions));
    }

    Ok(sections)
}

fn top_learnings_for_category(
    conn: &Connection,
    category: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT description
         FROM learnings
         WHERE category = ?1
         ORDER BY times_referenced DESC, discovered_at DESC
         LIMIT ?2",
    )?;

    let mut seen = HashSet::new();
    let rows = stmt
        .query_map(rusqlite::params![category, limit as i64], |row| {
            row.get::<_, String>(0)
        })?
        .filter_map(|r| r.ok())
        .filter(|description| seen.insert(description.trim().to_string()))
        .collect();

    Ok(rows)
}

fn top_trusted_solutions(conn: &Connection, limit: usize) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT description, confidence
         FROM solutions
         WHERE confidence >= ?1
         ORDER BY confidence DESC, success_count DESC, last_used_at DESC
         LIMIT ?2",
    )?;

    let rows = stmt
        .query_map(rusqlite::params![TRUST_THRESHOLD, limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?
        .filter_map(|r| r.ok())
        .map(|(description, confidence)| format!("[{:.2}] {}", confidence, description))
        .collect();

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;
    use crate::db::schema::SCHEMA;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct CwdGuard(PathBuf);

    impl CwdGuard {
        fn change_to(path: &Path) -> Self {
            let original = env::current_dir().unwrap();
            env::set_current_dir(path).unwrap();
            Self(original)
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.0);
        }
    }

    fn setup_temp_project() -> (TempDir, CwdGuard) {
        let tmp = TempDir::new().unwrap();
        let guard = CwdGuard::change_to(tmp.path());
        fs::create_dir_all(tmp.path().join(".dial")).unwrap();
        (tmp, guard)
    }

    fn setup_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        migrations::run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    #[serial(cwd)]
    fn test_append_progress_log_entry_creates_log() {
        let (_tmp, _guard) = setup_temp_project();
        let entry = ProgressLogEntry {
            task_id: 7,
            task_description: "Add progress tracking".to_string(),
            iteration_id: 3,
            attempt_number: 1,
            outcome: ProgressOutcome::Completed,
            summary: Some("Validation passed".to_string()),
            changed_files_summary: Some("src/main.rs | 8 +++++---".to_string()),
            commit_hash: Some("abcdef1234567890".to_string()),
            learnings: vec![(
                "pattern".to_string(),
                "Prefer host-owned logging".to_string(),
            )],
        };

        append_progress_log_entry(&entry).unwrap();

        let content = fs::read_to_string(progress_log_path()).unwrap();
        assert!(content.contains("# DIAL Progress Log"));
        assert!(content.contains("Task #7"));
        assert!(content.contains("Validation passed"));
        assert!(content.contains("src/main.rs | 8 +++++---"));
        assert!(content.contains("[pattern] Prefer host-owned logging"));
    }

    #[test]
    #[serial(cwd)]
    fn test_render_patterns_digest_groups_content() {
        let conn = setup_memory_db();
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category, occurrence_count)
             VALUES ('sql-lock', 'SQLite lock', 'db', 4)",
            [],
        )
        .unwrap();
        let pattern_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO learnings (category, description, times_referenced)
             VALUES ('pattern', 'Use one task at a time', 5)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO learnings (category, description, times_referenced)
             VALUES ('gotcha', 'UI tasks need a browser check', 3)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO solutions (pattern_id, description, confidence, success_count)
             VALUES (?1, 'Retry after releasing the write lock', 0.82, 2)",
            [pattern_id],
        )
        .unwrap();

        let digest = render_patterns_digest(&conn).unwrap();
        assert!(digest.contains("## Reusable Patterns"));
        assert!(digest.contains("Use one task at a time"));
        assert!(digest.contains("## Gotchas"));
        assert!(digest.contains("UI tasks need a browser check"));
        assert!(digest.contains("## Trusted Solutions"));
        assert!(digest.contains("[0.82] Retry after releasing the write lock"));
    }

    #[test]
    #[serial(cwd)]
    fn test_render_task_ledger_includes_summary_and_sections() {
        let conn = setup_memory_db();
        conn.execute(
            "INSERT INTO tasks (description, status, priority) VALUES ('Pending task', 'pending', 5)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (description, status, priority) VALUES ('Blocked task', 'blocked', 5)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (description, status, priority, completed_at)
             VALUES ('Done task', 'completed', 5, '2026-03-30T13:00:00-05:00')",
            [],
        )
        .unwrap();

        let ledger = render_task_ledger(&conn).unwrap();
        assert!(ledger.contains("# DIAL Task Ledger"));
        assert!(ledger.contains("## Summary"));
        assert!(ledger.contains("Pending: 1"));
        assert!(ledger.contains("Blocked: 1"));
        assert!(ledger.contains("## Ready Next"));
        assert!(ledger.contains("Pending task"));
        assert!(ledger.contains("## Blocked Tasks"));
        assert!(ledger.contains("Blocked task"));
        assert!(ledger.contains("## Recently Completed"));
        assert!(ledger.contains("Done task"));
    }

    #[test]
    #[serial(cwd)]
    fn test_tail_progress_log_returns_recent_entries() {
        let (_tmp, _guard) = setup_temp_project();
        fs::write(
            progress_log_path(),
            "# DIAL Progress Log\n\nStarted: now\n\n---\n\n## First\nA\n\n## Second\nB\n\n## Third\nC\n",
        )
        .unwrap();

        let tail = tail_progress_log(2).unwrap().unwrap();
        assert!(tail.contains("## Second"));
        assert!(tail.contains("## Third"));
        assert!(!tail.contains("## First"));
    }
}
