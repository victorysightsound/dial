/// Approximate token counting and context budget management.
///
/// Uses a chars/4 heuristic for token estimation, which is a reasonable
/// approximation for most LLM tokenizers (GPT-family, Claude, etc.).

/// Estimate token count for a string using chars/4 heuristic.
pub fn estimate_tokens(text: &str) -> usize {
    // chars/4 is a widely-used approximation.
    // More accurate than bytes/4 for non-ASCII text.
    (text.chars().count() + 3) / 4
}

/// Context item with priority for budget-aware assembly.
#[derive(Debug, Clone)]
pub struct ContextItem {
    /// Label for this context block (e.g., "task_spec", "spec:auth", "solution:42").
    pub label: String,
    /// The actual text content.
    pub content: String,
    /// Priority (lower = higher priority, assembled first).
    /// Recommended: task_spec=0, fts_specs=10, trusted_solutions=20, failures=30, learnings=40.
    pub priority: u32,
    /// Estimated token count (computed on creation).
    pub tokens: usize,
}

impl ContextItem {
    /// Create a new context item, automatically estimating its token count.
    pub fn new(label: &str, content: &str, priority: u32) -> Self {
        let tokens = estimate_tokens(content);
        Self {
            label: label.to_string(),
            content: content.to_string(),
            priority,
            tokens,
        }
    }
}

/// Assemble context items within a token budget.
/// Returns (included items, excluded items) — both sorted by priority.
/// Items are added in priority order (lowest number first) until budget is exhausted.
pub fn assemble_context(items: &[ContextItem], token_budget: usize) -> (Vec<&ContextItem>, Vec<&ContextItem>) {
    let mut sorted: Vec<&ContextItem> = items.iter().collect();
    sorted.sort_by_key(|item| item.priority);

    let mut included = Vec::new();
    let mut excluded = Vec::new();
    let mut used = 0;

    for item in sorted {
        if used + item.tokens <= token_budget {
            used += item.tokens;
            included.push(item);
        } else {
            excluded.push(item);
        }
    }

    (included, excluded)
}

/// Format included context items into a single string.
pub fn format_context(items: &[&ContextItem]) -> String {
    items
        .iter()
        .map(|item| format!("## {}\n\n{}", item.label, item.content))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_short() {
        // "hello" = 5 chars -> (5+3)/4 = 2
        assert_eq!(estimate_tokens("hello"), 2);
    }

    #[test]
    fn test_estimate_tokens_longer() {
        // 100 chars -> 25 tokens (approx)
        let text = "a".repeat(100);
        assert_eq!(estimate_tokens(&text), 25);
    }

    #[test]
    fn test_estimate_tokens_unicode() {
        // Unicode chars counted correctly
        let text = "你好世界"; // 4 chars
        assert_eq!(estimate_tokens(text), 1);
    }

    #[test]
    fn test_context_item_creation() {
        let item = ContextItem::new("test", "hello world", 10);
        assert_eq!(item.label, "test");
        assert_eq!(item.priority, 10);
        assert!(item.tokens > 0);
    }

    #[test]
    fn test_assemble_context_within_budget() {
        let items = vec![
            ContextItem::new("a", &"x".repeat(40), 0),  // ~10 tokens
            ContextItem::new("b", &"x".repeat(40), 10), // ~10 tokens
        ];

        let (included, excluded) = assemble_context(&items, 100);
        assert_eq!(included.len(), 2);
        assert_eq!(excluded.len(), 0);
    }

    #[test]
    fn test_assemble_context_exceeds_budget() {
        let items = vec![
            ContextItem::new("high", &"x".repeat(40), 0),   // ~10 tokens
            ContextItem::new("low", &"x".repeat(400), 10),  // ~100 tokens
        ];

        let (included, excluded) = assemble_context(&items, 20);
        assert_eq!(included.len(), 1);
        assert_eq!(included[0].label, "high");
        assert_eq!(excluded.len(), 1);
        assert_eq!(excluded[0].label, "low");
    }

    #[test]
    fn test_assemble_context_priority_order() {
        let items = vec![
            ContextItem::new("low_priority", "small", 40),
            ContextItem::new("high_priority", "small", 0),
            ContextItem::new("mid_priority", "small", 20),
        ];

        let (included, _excluded) = assemble_context(&items, 1000);
        assert_eq!(included[0].label, "high_priority");
        assert_eq!(included[1].label, "mid_priority");
        assert_eq!(included[2].label, "low_priority");
    }

    #[test]
    fn test_assemble_context_zero_budget() {
        let items = vec![
            ContextItem::new("a", "content", 0),
        ];

        let (included, excluded) = assemble_context(&items, 0);
        assert_eq!(included.len(), 0);
        assert_eq!(excluded.len(), 1);
    }

    #[test]
    fn test_format_context() {
        let items = vec![
            ContextItem::new("Section A", "Content A", 0),
            ContextItem::new("Section B", "Content B", 10),
        ];

        let refs: Vec<&ContextItem> = items.iter().collect();
        let formatted = format_context(&refs);
        assert!(formatted.contains("## Section A"));
        assert!(formatted.contains("Content A"));
        assert!(formatted.contains("## Section B"));
        assert!(formatted.contains("---"));
    }
}
