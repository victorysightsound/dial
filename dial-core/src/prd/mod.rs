pub mod schema;

use crate::errors::{DialError, Result};
use rusqlite::Connection;
use std::env;
use std::path::PathBuf;

/// Returns the path to the PRD database (.dial/prd.db).
pub fn get_prd_db_path() -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join(".dial").join("prd.db")
}

/// Returns true if a PRD database exists in the current project.
pub fn prd_db_exists() -> bool {
    get_prd_db_path().exists()
}

/// Opens an existing PRD database connection with WAL mode.
pub fn get_prd_db() -> Result<Connection> {
    let path = get_prd_db_path();
    if !path.exists() {
        return Err(DialError::UserError(
            "PRD database not found. Run 'dial spec import' or 'dial spec wizard' first.".to_string(),
        ));
    }
    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    Ok(conn)
}

/// Creates and initializes a new PRD database, applying the schema.
pub fn init_prd_db() -> Result<Connection> {
    let path = get_prd_db_path();

    // Ensure .dial directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    conn.execute_batch(schema::SCHEMA)?;

    // Set initial metadata
    conn.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1')",
        [],
    )?;

    Ok(conn)
}

/// Opens the PRD database, creating it if it doesn't exist.
pub fn get_or_init_prd_db() -> Result<Connection> {
    if prd_db_exists() {
        get_prd_db()
    } else {
        init_prd_db()
    }
}
