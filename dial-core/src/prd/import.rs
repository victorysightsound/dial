use crate::errors::{DialError, Result};
use crate::prd::{get_or_init_prd_db, prd_delete_all_sections, prd_insert_section, prd_meta_set, prd_record_source};
use crate::prd::parser::parse_markdown_file;
use rusqlite::Connection;
use std::path::Path;
use walkdir::WalkDir;

/// Result of a PRD import operation.
pub struct ImportResult {
    pub files: usize,
    pub sections: usize,
}

/// Import all markdown files from a directory into prd.db.
///
/// Clears existing sections and re-imports everything.
/// Records each source file for tracking.
pub fn prd_import(specs_dir: &str) -> Result<ImportResult> {
    let specs_path = Path::new(specs_dir);
    if !specs_path.exists() {
        return Err(DialError::SpecsDirNotFound(specs_dir.to_string()));
    }

    let conn = get_or_init_prd_db()?;

    // Find all markdown files
    let md_files: Vec<_> = WalkDir::new(specs_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "md")
                .unwrap_or(false)
        })
        .collect();

    if md_files.is_empty() {
        return Ok(ImportResult { files: 0, sections: 0 });
    }

    // Clear existing data for fresh import
    prd_delete_all_sections(&conn)?;
    conn.execute("DELETE FROM sources", [])?;

    let mut total_sections = 0;
    let mut global_sort_offset = 0;

    for entry in &md_files {
        let md_path = entry.path();
        let sections = parse_markdown_file(md_path)?;
        let count = import_sections_to_db(&conn, &sections, global_sort_offset)?;
        total_sections += count;
        global_sort_offset += count as i32;

        // Record source
        let metadata = std::fs::metadata(md_path).ok();
        let file_size = metadata.as_ref().map(|m| m.len() as i64);
        let modified = metadata
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let datetime: chrono::DateTime<chrono::Utc> = t.into();
                datetime.format("%Y-%m-%dT%H:%M:%S").to_string()
            });

        prd_record_source(
            &conn,
            &md_path.to_string_lossy(),
            file_size,
            modified.as_deref(),
        )?;
    }

    prd_meta_set(&conn, "last_import", &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string())?;

    Ok(ImportResult {
        files: md_files.len(),
        sections: total_sections,
    })
}

/// Import a single markdown file into prd.db.
pub fn prd_import_file(file_path: &Path) -> Result<usize> {
    if !file_path.exists() {
        return Err(DialError::SpecsDirNotFound(file_path.to_string_lossy().to_string()));
    }

    let conn = get_or_init_prd_db()?;
    let sections = parse_markdown_file(file_path)?;
    let count = import_sections_to_db(&conn, &sections, 0)?;

    let metadata = std::fs::metadata(file_path).ok();
    let file_size = metadata.as_ref().map(|m| m.len() as i64);

    prd_record_source(
        &conn,
        &file_path.to_string_lossy(),
        file_size,
        None,
    )?;

    Ok(count)
}

/// Insert parsed sections into the database.
fn import_sections_to_db(
    conn: &Connection,
    sections: &[crate::prd::parser::ParsedSection],
    sort_offset: i32,
) -> Result<usize> {
    let mut count = 0;
    for section in sections {
        prd_insert_section(
            conn,
            &section.section_id,
            &section.title,
            section.parent_id.as_deref(),
            section.level,
            section.sort_order + sort_offset,
            &section.content,
            section.word_count,
        )?;
        count += 1;
    }
    Ok(count)
}
