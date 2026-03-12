pub mod parser;
pub mod schema;
pub mod templates;

use crate::errors::{DialError, Result};
use rusqlite::{params, Connection};
use std::env;
use std::path::PathBuf;

/// A terminology entry in the PRD database.
#[derive(Debug, Clone)]
pub struct PrdTerm {
    pub id: i64,
    pub canonical: String,
    pub variants: String,
    pub definition: String,
    pub category: String,
    pub first_used_in: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

/// A source file record tracking what was imported.
#[derive(Debug, Clone)]
pub struct PrdSource {
    pub id: i64,
    pub file_path: String,
    pub imported_at: String,
    pub file_size: Option<i64>,
    pub modified_at: Option<String>,
}

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

// --- Terminology CRUD ---

/// Add a terminology entry.
pub fn prd_add_term(
    conn: &Connection,
    canonical: &str,
    variants_json: &str,
    definition: &str,
    category: &str,
    first_used_in: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO terminology (canonical, variants, definition, category, first_used_in)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![canonical, variants_json, definition, category, first_used_in],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Full-text search terminology.
pub fn prd_search_terms(conn: &Connection, query: &str) -> Result<Vec<PrdTerm>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.canonical, t.variants, t.definition, t.category, t.first_used_in, t.created_at, t.updated_at
         FROM terminology t
         INNER JOIN terminology_fts fts ON t.id = fts.rowid
         WHERE terminology_fts MATCH ?1
         ORDER BY rank
         LIMIT 20",
    )?;

    let rows = stmt
        .query_map(params![query], |row| {
            Ok(PrdTerm {
                id: row.get(0)?,
                canonical: row.get(1)?,
                variants: row.get(2)?,
                definition: row.get(3)?,
                category: row.get(4)?,
                first_used_in: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// List all terminology entries, optionally filtered by category.
pub fn prd_list_terms(conn: &Connection, category: Option<&str>) -> Result<Vec<PrdTerm>> {
    let (sql, param): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match category {
        Some(cat) => (
            "SELECT id, canonical, variants, definition, category, first_used_in, created_at, updated_at
             FROM terminology WHERE category = ?1 ORDER BY canonical",
            vec![Box::new(cat.to_string())],
        ),
        None => (
            "SELECT id, canonical, variants, definition, category, first_used_in, created_at, updated_at
             FROM terminology ORDER BY canonical",
            vec![],
        ),
    };

    let mut stmt = conn.prepare(sql)?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = param.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(params_refs.as_slice(), |row| {
            Ok(PrdTerm {
                id: row.get(0)?,
                canonical: row.get(1)?,
                variants: row.get(2)?,
                definition: row.get(3)?,
                category: row.get(4)?,
                first_used_in: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Delete a terminology entry by canonical name.
pub fn prd_delete_term(conn: &Connection, canonical: &str) -> Result<()> {
    conn.execute("DELETE FROM terminology WHERE canonical = ?1", params![canonical])?;
    Ok(())
}

// --- Sources CRUD ---

/// Record a source file import.
pub fn prd_record_source(
    conn: &Connection,
    file_path: &str,
    file_size: Option<i64>,
    modified_at: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO sources (file_path, file_size, modified_at) VALUES (?1, ?2, ?3)",
        params![file_path, file_size, modified_at],
    )?;
    Ok(conn.last_insert_rowid())
}

/// List all recorded source files.
pub fn prd_list_sources(conn: &Connection) -> Result<Vec<PrdSource>> {
    let mut stmt = conn.prepare(
        "SELECT id, file_path, imported_at, file_size, modified_at FROM sources ORDER BY imported_at DESC",
    )?;

    let rows = stmt
        .query_map([], |row| {
            Ok(PrdSource {
                id: row.get(0)?,
                file_path: row.get(1)?,
                imported_at: row.get(2)?,
                file_size: row.get(3)?,
                modified_at: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

// --- Meta CRUD ---

/// Set a metadata key-value pair (upsert).
pub fn prd_meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Get a metadata value by key.
pub fn prd_meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM meta WHERE key = ?1")?;
    let result = stmt.query_row(params![key], |row| row.get(0)).ok();
    Ok(result)
}
