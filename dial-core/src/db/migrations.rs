use rusqlite::Connection;
use crate::errors::Result;

/// Each migration has a version number and an apply function.
/// Migrations are applied in order. Once applied, the version is recorded
/// in the migrations table so it won't run again.
struct Migration {
    version: i64,
    description: &'static str,
    apply: fn(&Connection) -> Result<()>,
}

/// All migrations in order. New migrations are appended to this list.
/// Version numbers must be sequential starting from 1.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Add learnings table with FTS5",
        apply: migrate_001_learnings,
    },
    Migration {
        version: 2,
        description: "Add task_dependencies table for dependency graph",
        apply: migrate_002_task_dependencies,
    },
    Migration {
        version: 3,
        description: "Add provider_usage table for token and cost tracking",
        apply: migrate_003_provider_usage,
    },
    Migration {
        version: 4,
        description: "Add validation_steps table for configurable pipeline",
        apply: migrate_004_validation_steps,
    },
    Migration {
        version: 5,
        description: "Seed failure_patterns from hardcoded patterns",
        apply: migrate_005_seed_failure_patterns,
    },
    Migration {
        version: 6,
        description: "Add regex_pattern and status columns to failure_patterns for DB-driven detection",
        apply: migrate_006_pattern_regex_and_status,
    },
    Migration {
        version: 7,
        description: "Add solution provenance columns and solution_history table",
        apply: migrate_007_solution_provenance,
    },
    Migration {
        version: 8,
        description: "Expand iterations status CHECK to include awaiting_approval and rejected",
        apply: migrate_008_iterations_approval_status,
    },
];

/// Ensure the migrations tracking table exists, then apply any pending migrations.
pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS migrations (
            version INTEGER PRIMARY KEY,
            description TEXT NOT NULL,
            applied_at TEXT DEFAULT CURRENT_TIMESTAMP
        );"
    )?;

    let current_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    for migration in MIGRATIONS {
        if migration.version > current_version {
            (migration.apply)(conn)?;
            conn.execute(
                "INSERT INTO migrations (version, description) VALUES (?1, ?2)",
                rusqlite::params![migration.version, migration.description],
            )?;
        }
    }

    Ok(())
}

/// Return the current schema version.
pub fn current_version(conn: &Connection) -> Result<i64> {
    // migrations table might not exist yet
    let exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='migrations'",
        [],
        |row| row.get(0),
    )?;

    if !exists {
        return Ok(0);
    }

    let version: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM migrations",
        [],
        |row| row.get(0),
    )?;

    Ok(version)
}

/// Return the latest available migration version.
pub fn latest_version() -> i64 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

// --- Migration Functions ---

fn migrate_001_learnings(conn: &Connection) -> Result<()> {
    let has_learnings: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='learnings'",
        [],
        |row| row.get(0),
    )?;

    if !has_learnings {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS learnings (
                id INTEGER PRIMARY KEY,
                category TEXT,
                description TEXT NOT NULL,
                discovered_at TEXT DEFAULT CURRENT_TIMESTAMP,
                times_referenced INTEGER DEFAULT 0
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS learnings_fts USING fts5(
                category, description,
                content='learnings', content_rowid='id',
                tokenize='porter'
            );

            CREATE TRIGGER IF NOT EXISTS learnings_ai AFTER INSERT ON learnings BEGIN
                INSERT INTO learnings_fts(rowid, category, description)
                VALUES (NEW.id, COALESCE(NEW.category, ''), NEW.description);
            END;

            CREATE TRIGGER IF NOT EXISTS learnings_ad AFTER DELETE ON learnings BEGIN
                INSERT INTO learnings_fts(learnings_fts, rowid, category, description)
                VALUES('delete', OLD.id, COALESCE(OLD.category, ''), OLD.description);
            END;

            CREATE TRIGGER IF NOT EXISTS learnings_au AFTER UPDATE ON learnings BEGIN
                INSERT INTO learnings_fts(learnings_fts, rowid, category, description)
                VALUES('delete', OLD.id, COALESCE(OLD.category, ''), OLD.description);
                INSERT INTO learnings_fts(rowid, category, description)
                VALUES (NEW.id, COALESCE(NEW.category, ''), NEW.description);
            END;
            "#,
        )?;
    }
    Ok(())
}

fn migrate_002_task_dependencies(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS task_dependencies (
            task_id INTEGER NOT NULL,
            depends_on_id INTEGER NOT NULL,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (task_id, depends_on_id),
            FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
            FOREIGN KEY (depends_on_id) REFERENCES tasks(id) ON DELETE CASCADE
        );
        "#,
    )?;
    Ok(())
}

fn migrate_003_provider_usage(conn: &Connection) -> Result<()> {
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
        "#,
    )?;
    Ok(())
}

fn migrate_004_validation_steps(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS validation_steps (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            command TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            required INTEGER NOT NULL DEFAULT 1,
            timeout_secs INTEGER,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )?;
    Ok(())
}

fn migrate_006_pattern_regex_and_status(conn: &Connection) -> Result<()> {
    // Add regex_pattern column for DB-driven pattern detection
    conn.execute_batch(
        r#"
        ALTER TABLE failure_patterns ADD COLUMN regex_pattern TEXT;
        ALTER TABLE failure_patterns ADD COLUMN status TEXT NOT NULL DEFAULT 'trusted';
        "#,
    )?;

    // Populate regex patterns for seeded entries
    let patterns = [
        ("ImportError", "(?i)ImportError"),
        ("ModuleNotFoundError", "(?i)ModuleNotFoundError"),
        ("SyntaxError", "(?i)SyntaxError"),
        ("IndentationError", "(?i)IndentationError"),
        ("NameError", "(?i)NameError"),
        ("TypeError", "(?i)TypeError"),
        ("ValueError", "(?i)ValueError"),
        ("AttributeError", "(?i)AttributeError"),
        ("KeyError", "(?i)KeyError"),
        ("IndexError", "(?i)IndexError"),
        ("FileNotFoundError", "(?i)FileNotFoundError"),
        ("PermissionError", "(?i)PermissionError"),
        ("ConnectionError", "(?i)ConnectionError"),
        ("TimeoutError", "(?i)TimeoutError"),
        ("TestFailure", "(?i)FAILED.*test_"),
        ("AssertionError", "(?i)AssertionError"),
        ("RustCompileError", r"(?i)error\[E\d+\]|error: could not compile"),
        ("CargoBuildError", "(?i)cargo build.*failed"),
        ("NpmError", "(?i)npm ERR!"),
        ("TypeScriptError", r"(?i)tsc.*error TS\d+"),
    ];

    let mut stmt = conn.prepare(
        "UPDATE failure_patterns SET regex_pattern = ?2 WHERE pattern_key = ?1",
    )?;

    for (key, regex) in &patterns {
        stmt.execute(rusqlite::params![key, regex])?;
    }

    Ok(())
}

fn migrate_007_solution_provenance(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        ALTER TABLE solutions ADD COLUMN source TEXT NOT NULL DEFAULT 'auto-learned';
        ALTER TABLE solutions ADD COLUMN last_validated_at TEXT;
        ALTER TABLE solutions ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

        CREATE TABLE IF NOT EXISTS solution_history (
            id INTEGER PRIMARY KEY,
            solution_id INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            old_confidence REAL,
            new_confidence REAL,
            notes TEXT,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (solution_id) REFERENCES solutions(id) ON DELETE CASCADE
        );
        "#,
    )?;
    Ok(())
}

fn migrate_005_seed_failure_patterns(conn: &Connection) -> Result<()> {
    // Seed hardcoded patterns into the failure_patterns table.
    // Uses INSERT OR IGNORE to avoid duplicates if patterns already exist.
    let patterns = [
        ("ImportError", "Import/module error in Python", "import"),
        ("ModuleNotFoundError", "Module not found in Python", "import"),
        ("SyntaxError", "Syntax error", "syntax"),
        ("IndentationError", "Indentation error in Python", "syntax"),
        ("NameError", "Name not defined error", "runtime"),
        ("TypeError", "Type error", "runtime"),
        ("ValueError", "Value error", "runtime"),
        ("AttributeError", "Attribute error", "runtime"),
        ("KeyError", "Key error in dict/map", "runtime"),
        ("IndexError", "Index out of range", "runtime"),
        ("FileNotFoundError", "File not found", "runtime"),
        ("PermissionError", "Permission denied", "runtime"),
        ("ConnectionError", "Connection error", "runtime"),
        ("TimeoutError", "Timeout error", "runtime"),
        ("TestFailure", "Test case failure", "test"),
        ("AssertionError", "Assertion failed", "test"),
        ("RustCompileError", "Rust compilation error", "build"),
        ("CargoBuildError", "Cargo build failure", "build"),
        ("NpmError", "NPM error", "build"),
        ("TypeScriptError", "TypeScript compilation error", "build"),
        ("UnknownError", "Unrecognized error pattern", "unknown"),
    ];

    let mut stmt = conn.prepare(
        "INSERT OR IGNORE INTO failure_patterns (pattern_key, description, category)
         VALUES (?1, ?2, ?3)",
    )?;

    for (key, desc, cat) in &patterns {
        stmt.execute(rusqlite::params![key, desc, cat])?;
    }

    Ok(())
}

fn migrate_008_iterations_approval_status(conn: &Connection) -> Result<()> {
    // SQLite doesn't support ALTER TABLE to modify CHECK constraints.
    // Recreate the iterations table with the expanded CHECK constraint.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS iterations_new (
            id INTEGER PRIMARY KEY,
            task_id INTEGER NOT NULL,
            status TEXT DEFAULT 'in_progress'
                CHECK(status IN ('in_progress', 'completed', 'failed', 'reverted', 'awaiting_approval', 'rejected')),
            attempt_number INTEGER DEFAULT 1,
            started_at TEXT DEFAULT CURRENT_TIMESTAMP,
            ended_at TEXT,
            duration_seconds REAL,
            commit_hash TEXT,
            notes TEXT,
            FOREIGN KEY (task_id) REFERENCES tasks(id)
        );
        INSERT OR IGNORE INTO iterations_new SELECT * FROM iterations;
        DROP TABLE iterations;
        ALTER TABLE iterations_new RENAME TO iterations;
        "#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;").unwrap();
        conn.execute_batch(schema::SCHEMA).unwrap();
        conn
    }

    #[test]
    fn test_migrations_run_on_fresh_db() {
        let conn = setup_test_db();
        run_migrations(&conn).unwrap();

        let version = current_version(&conn).unwrap();
        assert_eq!(version, latest_version());
    }

    #[test]
    fn test_migrations_are_idempotent() {
        let conn = setup_test_db();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap(); // should not fail

        let version = current_version(&conn).unwrap();
        assert_eq!(version, latest_version());
    }

    #[test]
    fn test_migration_versions_are_sequential() {
        for (i, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(migration.version, (i + 1) as i64,
                "Migration at index {} has version {} but expected {}",
                i, migration.version, i + 1);
        }
    }

    #[test]
    fn test_current_version_no_table() {
        let conn = Connection::open_in_memory().unwrap();
        let version = current_version(&conn).unwrap();
        assert_eq!(version, 0);
    }

    #[test]
    fn test_task_dependencies_table_created() {
        let conn = setup_test_db();
        run_migrations(&conn).unwrap();

        // Verify table exists
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='task_dependencies'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(exists);
    }

    #[test]
    fn test_validation_steps_table_created() {
        let conn = setup_test_db();
        run_migrations(&conn).unwrap();

        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='validation_steps'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(exists);

        // Verify we can insert and read back a step
        conn.execute(
            "INSERT INTO validation_steps (name, command, sort_order, required, timeout_secs) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["build", "cargo build", 0, 1, 300],
        ).unwrap();

        let (name, command, sort_order, required, timeout_secs): (String, String, i64, i64, Option<i64>) = conn.query_row(
            "SELECT name, command, sort_order, required, timeout_secs FROM validation_steps WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).unwrap();

        assert_eq!(name, "build");
        assert_eq!(command, "cargo build");
        assert_eq!(sort_order, 0);
        assert_eq!(required, 1);
        assert_eq!(timeout_secs, Some(300));
    }
}
