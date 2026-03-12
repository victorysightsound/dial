use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// A parsed section from a markdown document.
#[derive(Debug, Clone)]
pub struct ParsedSection {
    pub section_id: String,
    pub title: String,
    pub parent_id: Option<String>,
    pub level: i32,
    pub sort_order: i32,
    pub content: String,
    pub word_count: i32,
}

/// Parse markdown content into hierarchical sections with dotted IDs.
///
/// Features:
/// - Code fence awareness (``` and ~~~) — headings inside fences are ignored
/// - Hierarchical section_id generation using dotted notation (1, 1.1, 1.1.2)
/// - Parent ID determination
/// - Duplicate ID handling with _N suffix
/// - Word count per section
pub fn parse_markdown(content: &str) -> Vec<ParsedSection> {
    let header_re = Regex::new(r"^(#{1,6})\s+(.+)$").unwrap();
    let fence_re = Regex::new(r"^(`{3,}|~{3,})").unwrap();

    let mut sections = Vec::new();
    let mut seen_ids = HashSet::new();

    // Counter array for up to 6 heading levels
    let mut counters = [0u32; 6];
    // Track the last section_id at each level for parent determination
    let mut level_ids: [Option<String>; 6] = Default::default();

    let mut in_code_fence = false;
    let mut current_title: Option<String> = None;
    let mut current_section_id: Option<String> = None;
    let mut current_parent_id: Option<String> = None;
    let mut current_level: i32 = 0;
    let mut current_sort: i32 = 0;
    let mut current_content: Vec<String> = Vec::new();

    for line in content.lines() {
        // Toggle code fence state
        if fence_re.is_match(line) {
            in_code_fence = !in_code_fence;
            if current_title.is_some() {
                current_content.push(line.to_string());
            }
            continue;
        }

        // Skip heading detection inside code fences
        if in_code_fence {
            if current_title.is_some() {
                current_content.push(line.to_string());
            }
            continue;
        }

        if let Some(caps) = header_re.captures(line) {
            // Save previous section
            if let Some(title) = current_title.take() {
                let content_str = current_content.join("\n").trim().to_string();
                let word_count = content_str.split_whitespace().count() as i32;
                sections.push(ParsedSection {
                    section_id: current_section_id.take().unwrap(),
                    title,
                    parent_id: current_parent_id.take(),
                    level: current_level,
                    sort_order: current_sort,
                    content: content_str,
                    word_count,
                });
                current_sort += 1;
            }

            let level = caps.get(1).unwrap().as_str().len();
            let title = caps.get(2).unwrap().as_str().trim().to_string();

            // Increment counter at this level, reset deeper levels
            counters[level - 1] += 1;
            for i in level..6 {
                counters[i] = 0;
                level_ids[i] = None;
            }

            // Generate dotted section_id
            let base_id: String = counters[..level]
                .iter()
                .filter(|&&c| c > 0)
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(".");

            // Handle duplicates
            let section_id = if seen_ids.contains(&base_id) {
                let mut suffix = 2;
                loop {
                    let candidate = format!("{}_{}", base_id, suffix);
                    if !seen_ids.contains(&candidate) {
                        break candidate;
                    }
                    suffix += 1;
                }
            } else {
                base_id
            };

            seen_ids.insert(section_id.clone());

            // Determine parent_id (the section_id of the nearest ancestor level)
            let parent_id = if level > 1 {
                // Search backwards for nearest parent level
                (0..level - 1)
                    .rev()
                    .find_map(|i| level_ids[i].clone())
            } else {
                None
            };

            // Record this level's section_id for child resolution
            level_ids[level - 1] = Some(section_id.clone());

            current_title = Some(title);
            current_section_id = Some(section_id);
            current_parent_id = parent_id;
            current_level = level as i32;
            current_content.clear();
        } else if current_title.is_some() {
            current_content.push(line.to_string());
        }
    }

    // Save last section
    if let Some(title) = current_title {
        let content_str = current_content.join("\n").trim().to_string();
        let word_count = content_str.split_whitespace().count() as i32;
        sections.push(ParsedSection {
            section_id: current_section_id.unwrap(),
            title,
            parent_id: current_parent_id,
            level: current_level,
            sort_order: current_sort,
            content: content_str,
            word_count,
        });
    }

    sections
}

/// Parse a markdown file into sections.
pub fn parse_markdown_file(path: &Path) -> std::io::Result<Vec<ParsedSection>> {
    let content = fs::read_to_string(path)?;
    Ok(parse_markdown(&content))
}
