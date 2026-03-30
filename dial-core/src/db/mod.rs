pub mod migrations;
pub mod schema;

use crate::errors::{DialError, Result};
use rusqlite::Connection;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub const DEFAULT_PHASE: &str = "default";
const LOCAL_AGENT_EXCLUDE_PATTERNS: &[&str] = &["/AGENTS.md", "/CLAUDE.md", "/GEMINI.md"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentsMode {
    Off,
    Local,
    Shared,
}

pub fn get_dial_dir() -> PathBuf {
    env::current_dir().unwrap_or_default().join(".dial")
}

pub fn get_db_path(phase: Option<&str>) -> PathBuf {
    let dial_dir = get_dial_dir();
    let phase = phase.unwrap_or_else(|| {
        get_current_phase()
            .unwrap_or_else(|_| DEFAULT_PHASE.to_string())
            .leak()
    });
    dial_dir.join(format!("{}.db", phase))
}

pub fn get_current_phase() -> Result<String> {
    let phase_file = get_dial_dir().join("current_phase");
    if phase_file.exists() {
        Ok(fs::read_to_string(&phase_file)?.trim().to_string())
    } else {
        Ok(DEFAULT_PHASE.to_string())
    }
}

pub fn set_current_phase(phase: &str) -> Result<()> {
    let dial_dir = get_dial_dir();
    let phase_file = dial_dir.join("current_phase");
    fs::write(&phase_file, phase)?;
    Ok(())
}

/// Execute a closure inside a SQLite transaction (BEGIN IMMEDIATE / COMMIT / ROLLBACK).
///
/// Uses BEGIN IMMEDIATE to acquire a write lock up front, preventing
/// SQLITE_BUSY errors in WAL mode when multiple writers contend.
/// On success the transaction is committed; on any error it is rolled back
/// and the error is propagated.
pub fn with_transaction<F, T>(conn: &Connection, f: F) -> Result<T>
where
    F: FnOnce(&Connection) -> Result<T>,
{
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match f(conn) {
        Ok(val) => {
            conn.execute_batch("COMMIT")?;
            Ok(val)
        }
        Err(e) => {
            // Best-effort rollback — if it fails the connection is left in
            // an indeterminate state, but the original error is more useful.
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

pub fn get_db(phase: Option<&str>) -> Result<Connection> {
    let db_path = get_db_path(phase);
    if !db_path.exists() {
        return Err(DialError::NotInitialized);
    }

    let conn = Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;

    // Run migrations
    migrations::run_migrations(&conn)?;

    Ok(conn)
}

pub fn init_db(
    phase: &str,
    import_solutions_from: Option<&str>,
    setup_agents: bool,
) -> Result<bool> {
    let agents_mode = if setup_agents {
        AgentsMode::Local
    } else {
        AgentsMode::Off
    };
    init_db_with_agents_mode(phase, import_solutions_from, agents_mode)
}

pub fn init_db_with_agents_mode(
    phase: &str,
    import_solutions_from: Option<&str>,
    agents_mode: AgentsMode,
) -> Result<bool> {
    let dial_dir = get_dial_dir();
    fs::create_dir_all(&dial_dir)?;

    let db_path = dial_dir.join(format!("{}.db", phase));

    if db_path.exists() {
        if !crate::output::prompt_yes_no(&format!(
            "Warning: Database {} already exists. Overwrite?",
            db_path.display()
        )) {
            println!("Aborted.");
            return Ok(false);
        }
        fs::remove_file(&db_path)?;
    }

    let conn = Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
    conn.execute_batch(schema::SCHEMA)?;

    // Set default config
    let now = chrono::Local::now().to_rfc3339();
    let project_name = env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    let defaults = [
        ("phase", phase),
        ("project_name", &project_name),
        ("build_cmd", ""),
        ("test_cmd", ""),
        ("build_timeout", "600"),
        ("test_timeout", "600"),
        ("enable_checkpoints", "true"),
        ("created_at", &now),
    ];

    for (key, value) in defaults {
        conn.execute(
            "INSERT INTO config (key, value) VALUES (?1, ?2)",
            [key, value],
        )?;
    }

    // Import solutions from another phase if requested
    if let Some(source_phase) = import_solutions_from {
        import_trusted_solutions(&conn, &dial_dir, source_phase)?;
    }

    set_git_info_exclude_pattern(".dial/", true)?;
    configure_local_agent_file_visibility(agents_mode)?;
    set_current_phase(phase)?;

    crate::output::print_success(&format!("Initialized DIAL database: {}", db_path.display()));

    if !matches!(agents_mode, AgentsMode::Off) {
        setup_agents_md(true)?;
    }

    Ok(true)
}

fn configure_local_agent_file_visibility(agents_mode: AgentsMode) -> Result<()> {
    let hide_local_agent_files = matches!(agents_mode, AgentsMode::Local);
    for pattern in LOCAL_AGENT_EXCLUDE_PATTERNS {
        set_git_info_exclude_pattern(pattern, hide_local_agent_files)?;
    }
    Ok(())
}

fn set_git_info_exclude_pattern(pattern: &str, enabled: bool) -> Result<()> {
    let output = match Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
    {
        Ok(output) => output,
        Err(_) => return Ok(()),
    };

    if !output.status.success() {
        return Ok(());
    }

    let git_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if git_dir.is_empty() {
        return Ok(());
    }

    let git_dir_path = PathBuf::from(&git_dir);
    let git_dir_path = if git_dir_path.is_absolute() {
        git_dir_path
    } else {
        env::current_dir()?.join(git_dir_path)
    };

    let exclude_path = git_dir_path.join("info").join("exclude");
    let existing = fs::read_to_string(&exclude_path).unwrap_or_default();
    let has_pattern = existing.lines().any(|line| line.trim() == pattern);
    if enabled && has_pattern {
        return Ok(());
    }
    if !enabled && !has_pattern {
        return Ok(());
    }

    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut lines: Vec<String> = existing.lines().map(|line| line.to_string()).collect();

    if enabled {
        lines.push(pattern.to_string());
    } else {
        lines.retain(|line| line.trim() != pattern);
    }

    let mut updated = lines.join("\n");
    if !updated.is_empty() {
        updated.push('\n');
    }
    fs::write(exclude_path, updated)?;

    Ok(())
}

fn import_trusted_solutions(
    conn: &Connection,
    dial_dir: &PathBuf,
    source_phase: &str,
) -> Result<()> {
    let source_db_path = dial_dir.join(format!("{}.db", source_phase));
    if !source_db_path.exists() {
        return Err(DialError::PhaseNotFound(source_phase.to_string()));
    }

    let source_conn = Connection::open(&source_db_path)?;
    source_conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;

    // Copy trusted solutions and their failure patterns
    let mut stmt = source_conn.prepare(
        "SELECT fp.* FROM failure_patterns fp
         INNER JOIN solutions s ON s.pattern_id = fp.id
         WHERE s.confidence >= ?1",
    )?;

    let patterns: Vec<_> = stmt
        .query_map([crate::TRUST_THRESHOLD], |row| {
            Ok((
                row.get::<_, i64>(0)?,            // id
                row.get::<_, String>(1)?,         // pattern_key
                row.get::<_, String>(2)?,         // description
                row.get::<_, Option<String>>(3)?, // category
                row.get::<_, i64>(4)?,            // occurrence_count
                row.get::<_, String>(5)?,         // first_seen_at
                row.get::<_, Option<String>>(6)?, // last_seen_at
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut pattern_id_map = std::collections::HashMap::new();

    for (
        old_id,
        pattern_key,
        description,
        category,
        occurrence_count,
        first_seen_at,
        last_seen_at,
    ) in &patterns
    {
        conn.execute(
            "INSERT INTO failure_patterns (pattern_key, description, category, occurrence_count, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![pattern_key, description, category, occurrence_count, first_seen_at, last_seen_at],
        )?;
        let new_id = conn.last_insert_rowid();
        pattern_id_map.insert(*old_id, new_id);
    }

    // Copy solutions
    let mut stmt = source_conn.prepare("SELECT * FROM solutions WHERE confidence >= ?1")?;

    let solutions: Vec<_> = stmt
        .query_map([crate::TRUST_THRESHOLD], |row| {
            Ok((
                row.get::<_, i64>(1)?,            // pattern_id
                row.get::<_, String>(2)?,         // description
                row.get::<_, Option<String>>(3)?, // code_example
                row.get::<_, f64>(4)?,            // confidence
                row.get::<_, i64>(5)?,            // success_count
                row.get::<_, i64>(6)?,            // failure_count
                row.get::<_, String>(7)?,         // created_at
                row.get::<_, Option<String>>(8)?, // last_used_at
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut count = 0;
    for (
        pattern_id,
        description,
        code_example,
        confidence,
        success_count,
        failure_count,
        created_at,
        last_used_at,
    ) in solutions
    {
        if let Some(&new_pattern_id) = pattern_id_map.get(&pattern_id) {
            conn.execute(
                "INSERT INTO solutions (pattern_id, description, code_example, confidence, success_count, failure_count, created_at, last_used_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![new_pattern_id, description, code_example, confidence, success_count, failure_count, created_at, last_used_at],
            )?;
            count += 1;
        }
    }

    crate::output::print_success(&format!(
        "Imported {} trusted solutions from '{}'.",
        count, source_phase
    ));

    Ok(())
}

const DIAL_AGENTS_SECTION: &str = r#"
---

## DIAL — Autonomous Development Loop

This project uses **DIAL** (Deterministic Iterative Agent Loop) for autonomous development.

### Get Full Instructions

```bash
sqlite3 ~/projects/dial/dial_guide.db "SELECT content FROM sections WHERE section_id LIKE '2.%' ORDER BY sort_order;"
```

### Quick Reference

```bash
dial status           # Current state
dial task list        # Show pending tasks
dial task next        # Show next task
dial iterate          # Start next task, get context
dial validate         # Run tests, commit on success
dial learn "text" -c category  # Record a learning
dial stats            # Statistics dashboard
```

### The DIAL Loop

1. `dial iterate` → Get task + context
2. Implement (one task only, no placeholders, search before creating)
3. `dial validate` → Test and commit
4. On success: next task. On failure: retry (max 3).

### Configuration

```bash
dial config set build_cmd "your build command"
dial config set test_cmd "your test command"
```
"#;

const OPTIONAL_SESSION_CONTEXT_SECTION: &str = r#"## On Entry

Run `session-context` if it is available in this environment.
If the command is not installed on this machine, continue without blocking on it.
"#;

fn new_agents_content(project_name: &str) -> String {
    format!(
        "# Project: {}\n\n{}\n{}",
        project_name, OPTIONAL_SESSION_CONTEXT_SECTION, DIAL_AGENTS_SECTION
    )
}

pub fn setup_agents_md(skip_if_exists: bool) -> Result<bool> {
    let project_root = env::current_dir()?;
    let agents_files = ["AGENTS.md", "CLAUDE.md", "GEMINI.md"];

    // Find existing AGENTS.md
    let mut agents_path: Option<PathBuf> = None;
    for name in agents_files {
        let path = project_root.join(name);
        if path.exists() {
            // Follow symlink if needed
            let real_path = if path.is_symlink() {
                fs::read_link(&path).ok().and_then(|p| {
                    if p.is_absolute() {
                        Some(p)
                    } else {
                        Some(project_root.join(p))
                    }
                })
            } else {
                Some(path.clone())
            };

            if let Some(rp) = real_path {
                if rp.exists() && !rp.is_symlink() {
                    agents_path = Some(rp);
                    break;
                }
            }
        }
    }

    let agents_path = match agents_path {
        Some(p) => p,
        None => {
            // Create new AGENTS.md
            let path = project_root.join("AGENTS.md");
            let project_name = project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Project".to_string());

            let content = new_agents_content(&project_name);
            fs::write(&path, content)?;
            crate::output::print_success(&format!(
                "Created {} with DIAL instructions.",
                path.display()
            ));
            return Ok(true);
        }
    };

    // Check if DIAL section already exists
    let existing_content = fs::read_to_string(&agents_path)?;
    if existing_content.contains("## DIAL") || existing_content.contains("dial iterate") {
        if skip_if_exists {
            crate::output::print_info("DIAL section already exists in AGENTS.md.");
            return Ok(true);
        }
        if !crate::output::prompt_yes_no("DIAL section already exists in AGENTS.md. Replace it?") {
            println!("Skipped AGENTS.md update.");
            return Ok(true);
        }
        // Remove existing DIAL section
        let re = regex::Regex::new(r"\n---\n\n## DIAL.*?(?=\n---\n|\n## [^D]|\z)").unwrap();
        let existing_content = re.replace(&existing_content, "").to_string();
        let new_content = format!("{}{}", existing_content.trim_end(), DIAL_AGENTS_SECTION);
        fs::write(&agents_path, new_content)?;
    } else {
        // Append DIAL section
        let new_content = format!("{}\n{}", existing_content.trim_end(), DIAL_AGENTS_SECTION);
        fs::write(&agents_path, new_content)?;
    }

    crate::output::print_success(&format!(
        "Added DIAL instructions to {}",
        agents_path.display()
    ));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::DialError;
    use serial_test::serial;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    /// Create an in-memory SQLite connection with a simple test table.
    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL);")
            .unwrap();
        conn
    }

    fn row_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn with_transaction_commits_on_success() {
        let conn = test_conn();

        let result = with_transaction(&conn, |c| {
            c.execute("INSERT INTO items (name) VALUES ('a')", [])?;
            c.execute("INSERT INTO items (name) VALUES ('b')", [])?;
            Ok(42)
        });

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(row_count(&conn), 2);
    }

    #[test]
    fn with_transaction_rolls_back_on_error() {
        let conn = test_conn();

        // Insert a row outside the transaction so we can verify it survives
        conn.execute("INSERT INTO items (name) VALUES ('pre')", [])
            .unwrap();
        assert_eq!(row_count(&conn), 1);

        let result: Result<()> = with_transaction(&conn, |c| {
            c.execute("INSERT INTO items (name) VALUES ('x')", [])?;
            c.execute("INSERT INTO items (name) VALUES ('y')", [])?;
            // Simulate a domain error mid-transaction
            Err(DialError::UserError("deliberate failure".into()))
        });

        assert!(result.is_err());
        // The two inserts inside the transaction should be rolled back;
        // only the pre-existing row remains.
        assert_eq!(row_count(&conn), 1);
    }

    #[test]
    fn with_transaction_propagates_original_error() {
        let conn = test_conn();

        let result: Result<()> = with_transaction(&conn, |_c| Err(DialError::TaskNotFound(999)));

        match result {
            Err(DialError::TaskNotFound(id)) => assert_eq!(id, 999),
            other => panic!("Expected TaskNotFound(999), got {:?}", other),
        }
    }

    #[test]
    fn with_transaction_returns_value_on_success() {
        let conn = test_conn();

        let id = with_transaction(&conn, |c| {
            c.execute("INSERT INTO items (name) VALUES ('z')", [])?;
            Ok(c.last_insert_rowid())
        })
        .unwrap();

        assert!(id > 0);
        assert_eq!(row_count(&conn), 1);
    }

    #[test]
    fn with_transaction_partial_writes_rolled_back() {
        let conn = test_conn();

        // Multiple inserts where the closure fails after the first succeeds
        let result = with_transaction(&conn, |c| {
            c.execute("INSERT INTO items (name) VALUES ('first')", [])?;
            // This will fail: NOT NULL constraint on name
            c.execute("INSERT INTO items (name) VALUES (NULL)", [])?;
            Ok(())
        });

        assert!(result.is_err());
        // Both writes should be rolled back
        assert_eq!(row_count(&conn), 0);
    }

    struct CwdGuard(PathBuf);

    impl CwdGuard {
        fn change_to(path: &std::path::Path) -> Self {
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

    #[test]
    fn new_agents_content_makes_session_context_optional() {
        let content = new_agents_content("demo-project");

        assert!(content.contains("# Project: demo-project"));
        assert!(content.contains("Run `session-context` if it is available"));
        assert!(content.contains("continue without blocking"));
        assert!(!content.contains("On Entry (MANDATORY)"));
        assert!(!content.contains("```bash\nsession-context\n```"));
    }

    #[test]
    #[serial(cwd)]
    fn setup_agents_md_creates_cross_platform_startup_guidance() {
        let tmp = TempDir::new().unwrap();
        let _guard = CwdGuard::change_to(tmp.path());

        let created = setup_agents_md(true).unwrap();
        assert!(created);

        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("Run `session-context` if it is available"));
        assert!(content.contains("continue without blocking"));
        assert!(!content.contains("On Entry (MANDATORY)"));
        assert!(!content.contains("```bash\nsession-context\n```"));
        assert!(content.contains("## DIAL"));
    }

    #[test]
    #[serial(cwd)]
    fn init_db_adds_dial_dir_to_git_info_exclude() {
        let tmp = TempDir::new().unwrap();
        let _guard = CwdGuard::change_to(tmp.path());

        Command::new("git").args(["init"]).output().unwrap();

        let created = init_db(DEFAULT_PHASE, None, false).unwrap();
        assert!(created);

        let exclude =
            fs::read_to_string(tmp.path().join(".git").join("info").join("exclude")).unwrap();
        assert!(exclude.lines().any(|line| line.trim() == ".dial/"));
    }

    #[test]
    #[serial(cwd)]
    fn init_db_local_agents_hide_agent_instruction_files() {
        let tmp = TempDir::new().unwrap();
        let _guard = CwdGuard::change_to(tmp.path());

        Command::new("git").args(["init"]).output().unwrap();

        let created = init_db_with_agents_mode(DEFAULT_PHASE, None, AgentsMode::Local).unwrap();
        assert!(created);

        let exclude =
            fs::read_to_string(tmp.path().join(".git").join("info").join("exclude")).unwrap();
        assert!(exclude.lines().any(|line| line.trim() == "/AGENTS.md"));
        assert!(exclude.lines().any(|line| line.trim() == "/CLAUDE.md"));
        assert!(exclude.lines().any(|line| line.trim() == "/GEMINI.md"));
    }

    #[test]
    #[serial(cwd)]
    fn init_db_shared_agents_keep_agent_instruction_files_visible() {
        let tmp = TempDir::new().unwrap();
        let _guard = CwdGuard::change_to(tmp.path());

        Command::new("git").args(["init"]).output().unwrap();

        let created = init_db_with_agents_mode(DEFAULT_PHASE, None, AgentsMode::Shared).unwrap();
        assert!(created);

        let exclude =
            fs::read_to_string(tmp.path().join(".git").join("info").join("exclude")).unwrap();
        assert!(!exclude.lines().any(|line| line.trim() == "/AGENTS.md"));
        assert!(!exclude.lines().any(|line| line.trim() == "/CLAUDE.md"));
        assert!(!exclude.lines().any(|line| line.trim() == "/GEMINI.md"));
    }

    #[test]
    #[serial(cwd)]
    fn init_db_off_agents_skips_file_creation() {
        let tmp = TempDir::new().unwrap();
        let _guard = CwdGuard::change_to(tmp.path());

        Command::new("git").args(["init"]).output().unwrap();

        let created = init_db_with_agents_mode(DEFAULT_PHASE, None, AgentsMode::Off).unwrap();
        assert!(created);

        assert!(!tmp.path().join("AGENTS.md").exists());
    }
}
