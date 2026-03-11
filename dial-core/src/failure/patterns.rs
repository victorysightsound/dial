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

pub fn detect_failure_pattern(error_text: &str) -> (&'static str, &'static str) {
    for pattern in FAILURE_PATTERNS.iter() {
        if pattern.matches(error_text) {
            return (pattern.pattern_key, pattern.category);
        }
    }
    ("UnknownError", "unknown")
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
