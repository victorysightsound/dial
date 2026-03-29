use crate::db::with_transaction;
use crate::errors::{DialError, Result};
use crate::prd::parser::parse_markdown_file;
use crate::prd::{
    get_or_init_prd_db, prd_delete_all_sections, prd_insert_section, prd_meta_set,
    prd_record_source,
};
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
        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
        .collect();

    if md_files.is_empty() {
        return Ok(ImportResult {
            files: 0,
            sections: 0,
        });
    }

    // Parse all files before starting the transaction so parse failures
    // don't leave the DB in a partially-cleared state.
    let mut parsed_files = Vec::new();
    for entry in &md_files {
        let md_path = entry.path();
        let sections = parse_markdown_file(md_path)?;
        let metadata = std::fs::metadata(md_path).ok();
        let file_size = metadata.as_ref().map(|m| m.len() as i64);
        let modified = metadata.and_then(|m| m.modified().ok()).map(|t| {
            let datetime: chrono::DateTime<chrono::Utc> = t.into();
            datetime.format("%Y-%m-%dT%H:%M:%S").to_string()
        });
        parsed_files.push((md_path.to_path_buf(), sections, file_size, modified));
    }

    // Wrap the entire clear-and-reimport in a transaction
    let total_sections = with_transaction(&conn, |conn| {
        prd_delete_all_sections(conn)?;
        conn.execute("DELETE FROM sources", [])?;

        let mut total_sections = 0;
        let mut global_sort_offset = 0;
        let mut top_level_offset = 0u32;

        for (md_path, sections, file_size, modified) in &parsed_files {
            let count =
                import_sections_to_db(conn, sections, global_sort_offset, top_level_offset)?;
            total_sections += count;
            global_sort_offset += count as i32;
            let h1_count = sections.iter().filter(|s| s.level == 1).count() as u32;
            top_level_offset += h1_count.max(1);

            prd_record_source(
                conn,
                &md_path.to_string_lossy(),
                *file_size,
                modified.as_deref(),
            )?;
        }

        prd_meta_set(
            conn,
            "last_import",
            &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        )?;

        Ok(total_sections)
    })?;

    Ok(ImportResult {
        files: md_files.len(),
        sections: total_sections,
    })
}

/// Import a single markdown file into prd.db.
pub fn prd_import_file(file_path: &Path) -> Result<usize> {
    if !file_path.exists() {
        return Err(DialError::SpecsDirNotFound(
            file_path.to_string_lossy().to_string(),
        ));
    }

    let conn = get_or_init_prd_db()?;
    let sections = parse_markdown_file(file_path)?;
    let count = import_sections_to_db(&conn, &sections, 0, 0)?;

    let metadata = std::fs::metadata(file_path).ok();
    let file_size = metadata.as_ref().map(|m| m.len() as i64);

    prd_record_source(&conn, &file_path.to_string_lossy(), file_size, None)?;

    Ok(count)
}

/// Migrate existing spec_sections from the phase DB into prd.db.
///
/// Reads all rows from spec_sections in the current phase database,
/// generates synthetic dotted IDs based on row order, extracts title
/// from heading_path, and inserts into prd.db.
pub fn migrate_spec_sections_to_prd() -> Result<usize> {
    let phase_conn = crate::db::get_db(None)?;
    let prd_conn = get_or_init_prd_db()?;

    let mut stmt = phase_conn.prepare(
        "SELECT id, file_path, heading_path, level, content FROM spec_sections ORDER BY file_path, id",
    )?;

    let rows: Vec<(i64, String, String, i32, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if rows.is_empty() {
        return Ok(0);
    }

    // Clear existing prd sections for clean migration
    prd_delete_all_sections(&prd_conn)?;

    let mut count = 0;
    let mut counters = [0u32; 6];
    let mut level_ids: [Option<String>; 6] = Default::default();

    for (_old_id, _file_path, heading_path, level, content) in &rows {
        let level = *level as usize;
        let level_clamped = level.clamp(1, 6);

        // Extract title (last component of heading_path "A > B > C" -> "C")
        let title = heading_path
            .rsplit(" > ")
            .next()
            .unwrap_or(heading_path)
            .trim();

        // Generate dotted section_id
        counters[level_clamped - 1] += 1;
        for i in level_clamped..6 {
            counters[i] = 0;
            level_ids[i] = None;
        }

        let section_id: String = counters[..level_clamped]
            .iter()
            .filter(|&&c| c > 0)
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(".");

        let parent_id = if level_clamped > 1 {
            (0..level_clamped - 1)
                .rev()
                .find_map(|i| level_ids[i].clone())
        } else {
            None
        };

        level_ids[level_clamped - 1] = Some(section_id.clone());

        let word_count = content.split_whitespace().count() as i32;

        prd_insert_section(
            &prd_conn,
            &section_id,
            title,
            parent_id.as_deref(),
            level_clamped as i32,
            count as i32,
            content,
            word_count,
        )?;
        count += 1;
    }

    Ok(count)
}

/// Insert parsed sections into the database.
/// `top_level_offset` offsets the first component of section IDs to avoid
/// collisions when importing multiple files (e.g., file 1 gets "1", "1.1",
/// file 2 gets "2", "2.1").
fn import_sections_to_db(
    conn: &Connection,
    sections: &[crate::prd::parser::ParsedSection],
    sort_offset: i32,
    top_level_offset: u32,
) -> Result<usize> {
    let mut count = 0;
    for section in sections {
        let section_id = offset_section_id(&section.section_id, top_level_offset);
        let parent_id = section
            .parent_id
            .as_ref()
            .map(|p| offset_section_id(p, top_level_offset));
        prd_insert_section(
            conn,
            &section_id,
            &section.title,
            parent_id.as_deref(),
            section.level,
            section.sort_order + sort_offset,
            &section.content,
            section.word_count,
        )?;
        count += 1;
    }
    Ok(count)
}

/// Offset the first component of a dotted section ID.
/// E.g., offset_section_id("1.2", 3) => "4.2"
fn offset_section_id(section_id: &str, offset: u32) -> String {
    if offset == 0 {
        return section_id.to_string();
    }
    let parts: Vec<&str> = section_id.split('.').collect();
    if parts.is_empty() {
        return section_id.to_string();
    }
    if let Ok(first) = parts[0].parse::<u32>() {
        let mut new_parts = vec![(first + offset).to_string()];
        new_parts.extend(parts[1..].iter().map(|s| s.to_string()));
        new_parts.join(".")
    } else {
        section_id.to_string()
    }
}
