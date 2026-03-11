use lazy_static::lazy_static;
use regex::Regex;

#[derive(Debug, Clone)]
pub struct FailurePattern {
    pub pattern_key: &'static str,
    pub category: &'static str,
    regex: Regex,
}

impl FailurePattern {
    fn new(pattern: &str, pattern_key: &'static str, category: &'static str) -> Self {
        FailurePattern {
            pattern_key,
            category,
            regex: Regex::new(&format!("(?i){}", pattern)).unwrap(),
        }
    }

    pub fn matches(&self, error_text: &str) -> bool {
        self.regex.is_match(error_text)
    }
}

lazy_static! {
    pub static ref FAILURE_PATTERNS: Vec<FailurePattern> = vec![
        // Import errors
        FailurePattern::new(r"ImportError", "ImportError", "import"),
        FailurePattern::new(r"ModuleNotFoundError", "ModuleNotFoundError", "import"),

        // Syntax errors
        FailurePattern::new(r"SyntaxError", "SyntaxError", "syntax"),
        FailurePattern::new(r"IndentationError", "IndentationError", "syntax"),

        // Runtime errors
        FailurePattern::new(r"NameError", "NameError", "runtime"),
        FailurePattern::new(r"TypeError", "TypeError", "runtime"),
        FailurePattern::new(r"ValueError", "ValueError", "runtime"),
        FailurePattern::new(r"AttributeError", "AttributeError", "runtime"),
        FailurePattern::new(r"KeyError", "KeyError", "runtime"),
        FailurePattern::new(r"IndexError", "IndexError", "runtime"),
        FailurePattern::new(r"FileNotFoundError", "FileNotFoundError", "runtime"),
        FailurePattern::new(r"PermissionError", "PermissionError", "runtime"),
        FailurePattern::new(r"ConnectionError", "ConnectionError", "runtime"),
        FailurePattern::new(r"TimeoutError", "TimeoutError", "runtime"),

        // Test errors
        FailurePattern::new(r"FAILED.*test_", "TestFailure", "test"),
        FailurePattern::new(r"AssertionError", "AssertionError", "test"),

        // Build errors - Rust
        FailurePattern::new(r"error\[E\d+\]", "RustCompileError", "build"),
        FailurePattern::new(r"error: could not compile", "RustCompileError", "build"),
        FailurePattern::new(r"cargo build.*failed", "CargoBuildError", "build"),

        // Build errors - JavaScript/TypeScript
        FailurePattern::new(r"npm ERR!", "NpmError", "build"),
        FailurePattern::new(r"tsc.*error TS\d+", "TypeScriptError", "build"),
    ];
}

/// Detect failure pattern using hardcoded regex library (fallback).
pub fn detect_failure_pattern(error_text: &str) -> (&'static str, &'static str) {
    for pattern in FAILURE_PATTERNS.iter() {
        if pattern.matches(error_text) {
            return (pattern.pattern_key, pattern.category);
        }
    }
    ("UnknownError", "unknown")
}

/// Detect failure pattern by first checking DB patterns (with regex_pattern),
/// then falling back to hardcoded patterns.
/// Returns (pattern_key, category) as owned Strings.
pub fn detect_failure_pattern_from_db(
    conn: &rusqlite::Connection,
    error_text: &str,
) -> (String, String) {
    // Try DB patterns first (includes user-added patterns)
    if let Ok(mut stmt) = conn.prepare(
        "SELECT pattern_key, category, regex_pattern FROM failure_patterns
         WHERE regex_pattern IS NOT NULL AND status IN ('trusted', 'confirmed')
         ORDER BY occurrence_count DESC",
    ) {
        let db_patterns: Vec<(String, String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        for (key, category, regex_str) in &db_patterns {
            if let Ok(re) = Regex::new(regex_str) {
                if re.is_match(error_text) {
                    return (key.clone(), category.clone());
                }
            }
        }
    }

    // Fall back to hardcoded patterns
    let (key, cat) = detect_failure_pattern(error_text);
    (key.to_string(), cat.to_string())
}

/// A suggested pattern derived from clustering unknown errors.
#[derive(Debug, Clone)]
pub struct SuggestedPattern {
    pub common_substring: String,
    pub occurrence_count: usize,
    pub sample_errors: Vec<String>,
}

/// Cluster unknown errors by common substrings and suggest new patterns.
/// Only considers errors where the pattern_key is "UnknownError" and
/// the same substring appears in 3+ different errors.
pub fn suggest_patterns_from_clustering(conn: &rusqlite::Connection) -> Vec<SuggestedPattern> {
    // Get all UnknownError failure texts
    let mut stmt = match conn.prepare(
        "SELECT f.error_text FROM failures f
         INNER JOIN failure_patterns fp ON f.pattern_id = fp.id
         WHERE fp.pattern_key = 'UnknownError'
         ORDER BY f.created_at DESC LIMIT 100",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let error_texts: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    if error_texts.len() < 3 {
        return Vec::new();
    }

    // Extract significant words/phrases (> 4 chars) from each error
    let mut word_counts: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();

    for (idx, text) in error_texts.iter().enumerate() {
        // Extract words > 4 chars that look like error identifiers
        let words: std::collections::HashSet<String> = text
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|w| w.len() > 4)
            .map(|w| w.to_string())
            .collect();

        for word in words {
            word_counts.entry(word).or_default().push(idx);
        }
    }

    // Find words/phrases that appear in 3+ different errors
    let mut suggestions = Vec::new();
    let mut seen_indices: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();

    let mut entries: Vec<_> = word_counts.into_iter()
        .filter(|(_, indices)| indices.len() >= 3)
        .collect();
    entries.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    for (word, indices) in entries {
        // Deduplicate clusters with identical index sets
        if seen_indices.contains(&indices) {
            continue;
        }
        seen_indices.insert(indices.clone());

        let samples: Vec<String> = indices.iter()
            .take(3)
            .map(|&i| {
                let text = &error_texts[i];
                if text.len() > 150 { format!("{}...", &text[..150]) } else { text.clone() }
            })
            .collect();

        suggestions.push(SuggestedPattern {
            common_substring: word,
            occurrence_count: indices.len(),
            sample_errors: samples,
        });
    }

    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_import_error() {
        let (key, cat) = detect_failure_pattern("ImportError: No module named 'foo'");
        assert_eq!(key, "ImportError");
        assert_eq!(cat, "import");
    }

    #[test]
    fn test_detect_rust_compile_error() {
        let (key, cat) = detect_failure_pattern("error[E0308]: mismatched types");
        assert_eq!(key, "RustCompileError");
        assert_eq!(cat, "build");
    }

    #[test]
    fn test_detect_test_failure() {
        let (key, cat) = detect_failure_pattern("FAILED tests/test_foo.py::test_bar");
        assert_eq!(key, "TestFailure");
        assert_eq!(cat, "test");
    }

    #[test]
    fn test_detect_unknown_error() {
        let (key, cat) = detect_failure_pattern("Something completely unexpected");
        assert_eq!(key, "UnknownError");
        assert_eq!(cat, "unknown");
    }
}
