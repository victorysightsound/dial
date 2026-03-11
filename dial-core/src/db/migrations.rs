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
}
