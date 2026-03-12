pub mod schema;

use crate::errors::{DialError, Result};
use rusqlite::{params, Connection};
use std::env;
use std::path::PathBuf;

/// A section in the PRD database.
#[derive(Debug, Clone)]
pub struct PrdSection {
    pub id: i64,
    pub section_id: String,
    pub title: String,
    pub parent_id: Option<String>,
    pub level: i32,
    pub sort_order: i32,
    pub content: String,
    pub word_count: i32,
    pub created_at: String,
    pub updated_at: Option<String>,
}

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

// --- Section CRUD ---

/// Insert a section into the PRD database. Returns the row id.
pub fn prd_insert_section(
    conn: &Connection,
    section_id: &str,
    title: &str,
    parent_id: Option<&str>,
    level: i32,
    sort_order: i32,
    content: &str,
    word_count: i32,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO sections (section_id, title, parent_id, level, sort_order, content, word_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![section_id, title, parent_id, level, sort_order, content, word_count],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Get a section by its dotted section_id (e.g. "1.2.3").
pub fn prd_get_section(conn: &Connection, section_id: &str) -> Result<Option<PrdSection>> {
    let mut stmt = conn.prepare(
        "SELECT id, section_id, title, parent_id, level, sort_order, content, word_count, created_at, updated_at
         FROM sections WHERE section_id = ?1",
    )?;

    let result = stmt
        .query_row(params![section_id], |row| {
            Ok(PrdSection {
                id: row.get(0)?,
                section_id: row.get(1)?,
                title: row.get(2)?,
                parent_id: row.get(3)?,
                level: row.get(4)?,
                sort_order: row.get(5)?,
                content: row.get(6)?,
                word_count: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })
        .ok();

    Ok(result)
}

/// List all sections ordered by sort_order.
pub fn prd_list_sections(conn: &Connection) -> Result<Vec<PrdSection>> {
    let mut stmt = conn.prepare(
        "SELECT id, section_id, title, parent_id, level, sort_order, content, word_count, created_at, updated_at
         FROM sections ORDER BY sort_order",
    )?;

    let rows = stmt
        .query_map([], |row| {
            Ok(PrdSection {
                id: row.get(0)?,
                section_id: row.get(1)?,
                title: row.get(2)?,
                parent_id: row.get(3)?,
                level: row.get(4)?,
                sort_order: row.get(5)?,
                content: row.get(6)?,
                word_count: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Full-text search sections by query.
pub fn prd_search_sections(conn: &Connection, query: &str) -> Result<Vec<PrdSection>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.section_id, s.title, s.parent_id, s.level, s.sort_order, s.content, s.word_count, s.created_at, s.updated_at
         FROM sections s
         INNER JOIN sections_fts fts ON s.id = fts.rowid
         WHERE sections_fts MATCH ?1
         ORDER BY rank
         LIMIT 10",
    )?;

    let rows = stmt
        .query_map(params![query], |row| {
            Ok(PrdSection {
                id: row.get(0)?,
                section_id: row.get(1)?,
                title: row.get(2)?,
                parent_id: row.get(3)?,
                level: row.get(4)?,
                sort_order: row.get(5)?,
                content: row.get(6)?,
                word_count: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Update a section's content by section_id.
pub fn prd_update_section(conn: &Connection, section_id: &str, content: &str) -> Result<()> {
    let word_count = content.split_whitespace().count() as i32;
    let updated = conn.execute(
        "UPDATE sections SET content = ?1, word_count = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%S', 'now')
         WHERE section_id = ?3",
        params![content, word_count, section_id],
    )?;
    if updated == 0 {
        return Err(DialError::PrdSectionNotFound(section_id.to_string()));
    }
    Ok(())
}

/// Delete all sections (used before re-import).
pub fn prd_delete_all_sections(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM sections", [])?;
    Ok(())
}
