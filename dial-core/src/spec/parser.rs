use regex::Regex;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SpecSection {
    pub heading_path: String,
    pub level: i32,
    pub content: String,
}

pub fn parse_markdown_sections(file_path: &Path) -> std::io::Result<Vec<SpecSection>> {
    let content = fs::read_to_string(file_path)?;
    let lines: Vec<&str> = content.lines().collect();

    let header_re = Regex::new(r"^(#{1,6})\s+(.+)$").unwrap();

    let mut sections = Vec::new();
    let mut current_headers: Vec<String> = Vec::new();
    let mut current_content: Vec<String> = Vec::new();
    let mut current_level = 0;

    for line in lines {
        if let Some(caps) = header_re.captures(line) {
            // Save previous section if exists
            if !current_headers.is_empty() {
                let content = current_content.join("\n").trim().to_string();
                sections.push(SpecSection {
                    heading_path: current_headers.join(" > "),
                    level: current_level,
                    content,
                });
            }

            let level = caps.get(1).unwrap().as_str().len() as i32;
            let title = caps.get(2).unwrap().as_str().trim().to_string();

            // Adjust header stack based on level
            while current_headers.len() >= level as usize {
                current_headers.pop();
            }
            current_headers.push(title);

            current_level = level;
            current_content.clear();
        } else {
            current_content.push(line.to_string());
        }
    }

    // Save last section
    if !current_headers.is_empty() {
        let content = current_content.join("\n").trim().to_string();
        sections.push(SpecSection {
            heading_path: current_headers.join(" > "),
            level: current_level,
            content,
        });
    }

    Ok(sections)
}
