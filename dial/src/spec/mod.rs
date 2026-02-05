pub mod parser;

use crate::db::get_db;
use crate::errors::{DialError, Result};
use crate::output::{blue, bold, dim, print_success, yellow};
use std::env;
use std::path::Path;
use walkdir::WalkDir;

pub fn index_specs(specs_dir: &str) -> Result<bool> {
    let specs_path = Path::new(specs_dir);

    if !specs_path.exists() {
        return Err(DialError::SpecsDirNotFound(specs_dir.to_string()));
    }

    let conn = get_db(None)?;

    // Clear existing spec sections
    conn.execute("DELETE FROM spec_sections", [])?;

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
        println!("{}", yellow(&format!("No markdown files found in '{}'.", specs_dir)));
        return Ok(true);
    }

    let cwd = env::current_dir()?;
    let mut total_sections = 0;

    for entry in &md_files {
        let md_path = entry.path();
        let relative_path = md_path
            .strip_prefix(&cwd)
            .unwrap_or(md_path)
            .to_string_lossy()
            .to_string();

        let sections = parser::parse_markdown_sections(md_path)?;

        for section in sections {
            if !section.content.is_empty() {
                conn.execute(
                    "INSERT INTO spec_sections (file_path, heading_path, level, content)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![relative_path, section.heading_path, section.level, section.content],
                )?;
                total_sections += 1;
            }
        }
    }

    print_success(&format!(
        "Indexed {} sections from {} files.",
        total_sections,
        md_files.len()
    ));

    Ok(true)
}

pub fn spec_search(query: &str) -> Result<Vec<SpecSearchResult>> {
    let conn = get_db(None)?;

    let mut stmt = conn.prepare(
        "SELECT s.id, s.file_path, s.heading_path, s.content
         FROM spec_sections s
         INNER JOIN spec_sections_fts fts ON s.id = fts.rowid
         WHERE spec_sections_fts MATCH ?1
         ORDER BY rank
         LIMIT 10",
    )?;

    let rows: Vec<SpecSearchResult> = stmt
        .query_map([query], |row| {
            Ok(SpecSearchResult {
                id: row.get(0)?,
                file_path: row.get(1)?,
                heading_path: row.get(2)?,
                content: row.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        println!("{}", dim(&format!("No spec sections matching '{}'.", query)));
        return Ok(rows);
    }

    println!("{}", bold(&format!("Spec sections matching '{}':", query)));
    println!("{}", "=".repeat(60));

    for row in &rows {
        let id_str = format!("[{}]", row.id);
        println!("\n{} {}", blue(&id_str), bold(&row.heading_path));
        println!("{}", dim(&format!("    File: {}", row.file_path)));
        // Show first 200 chars of content
        let preview = if row.content.len() > 200 {
            format!("{}...", &row.content[..200])
        } else {
            row.content.clone()
        };
        println!("    {}", preview);
    }

    Ok(rows)
}

#[derive(Debug, Clone)]
pub struct SpecSearchResult {
    pub id: i64,
    pub file_path: String,
    pub heading_path: String,
    pub content: String,
}

pub fn spec_show(section_id: i64) -> Result<Option<SpecSearchResult>> {
    let conn = get_db(None)?;

    let mut stmt = conn.prepare(
        "SELECT id, file_path, heading_path, content FROM spec_sections WHERE id = ?1",
    )?;

    let result = stmt
        .query_row([section_id], |row| {
            Ok(SpecSearchResult {
                id: row.get(0)?,
                file_path: row.get(1)?,
                heading_path: row.get(2)?,
                content: row.get(3)?,
            })
        })
        .ok();

    match &result {
        Some(spec) => {
            println!("{}", bold(&spec.heading_path));
            println!("{}", dim(&format!("File: {}", spec.file_path)));
            println!("{}", "=".repeat(60));
            println!("{}", spec.content);
        }
        None => {
            return Err(DialError::SpecSectionNotFound(section_id));
        }
    }

    Ok(result)
}

pub fn spec_list() -> Result<()> {
    let conn = get_db(None)?;

    let mut stmt = conn.prepare(
        "SELECT id, file_path, heading_path, level
         FROM spec_sections ORDER BY file_path, id",
    )?;

    let rows: Vec<(i64, String, String, i32)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        println!("{}", dim("No spec sections indexed. Run 'dial index' first."));
        return Ok(());
    }

    println!("{}", bold("Indexed Spec Sections"));
    println!("{}", "=".repeat(60));

    let mut current_file = String::new();
    for (id, file_path, heading_path, level) in rows {
        if file_path != current_file {
            current_file = file_path.clone();
            println!("\n{}", blue(&current_file));
        }
        let indent = "  ".repeat(level as usize);
        println!("  {}[{}] {}", indent, id, heading_path);
    }

    Ok(())
}
