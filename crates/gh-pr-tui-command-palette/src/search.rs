//! Fuzzy search functionality for commands

use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::provider::CommandItem;

/// Filter and score commands based on a search query
///
/// Uses nucleo-matcher's fuzzy matching algorithm (same as Helix/Zed).
/// Returns a vector of (command, score) pairs sorted by score (highest first).
///
/// # Arguments
///
/// * `commands` - All available commands to search through
/// * `query` - The search query string
///
/// # Returns
///
/// Vector of (CommandItem, score) tuples, sorted by relevance (highest score first).
/// Empty query returns all commands with score 0.
pub fn filter_commands<A: Clone>(
    commands: &[CommandItem<A>],
    query: &str,
) -> Vec<(CommandItem<A>, u16)> {
    // Empty query - return all commands
    if query.trim().is_empty() {
        return commands.iter().map(|c| (c.clone(), 0)).collect();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);

    // Reusable buffers for UTF-32 conversion (optimization to avoid allocations)
    let mut haystack_buf = Vec::new();
    let mut needle_buf = Vec::new();

    let query_lower = query.to_lowercase();

    let mut results: Vec<(CommandItem<A>, u16)> = commands
        .iter()
        .filter_map(|cmd| {
            let haystack = cmd.searchable_text();

            haystack_buf.clear();
            needle_buf.clear();

            let haystack_str = Utf32Str::new(&haystack, &mut haystack_buf);
            let query_str = Utf32Str::new(query, &mut needle_buf);

            matcher
                .fuzzy_match(haystack_str, query_str)
                .map(|mut score| {
                    // Boost score if title or description starts with query (case-insensitive)
                    // This ensures prefix matches rank higher than fuzzy matches
                    let title_lower = cmd.title.to_lowercase();
                    let desc_lower = cmd.description.to_lowercase();

                    if title_lower.starts_with(&query_lower) {
                        // Title prefix match gets highest boost
                        score = score.saturating_add(10000);
                    } else if desc_lower.starts_with(&query_lower) {
                        // Description prefix match gets moderate boost
                        score = score.saturating_add(5000);
                    } else if title_lower.contains(&query_lower) {
                        // Title contains (not prefix) gets small boost
                        score = score.saturating_add(1000);
                    }

                    (cmd.clone(), score)
                })
        })
        .collect();

    // Sort by score (descending)
    results.sort_by(|a, b| b.1.cmp(&a.1));

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestAction {
        Open,
        Save,
        Close,
    }

    fn create_test_commands() -> Vec<CommandItem<TestAction>> {
        vec![
            CommandItem {
                title: "Open File".into(),
                description: "Open a file from disk".into(),
                category: "File".into(),
                shortcut_hint: Some("Ctrl+O".into()),
                context: None,
                action: TestAction::Open,
            },
            CommandItem {
                title: "Save File".into(),
                description: "Save the current file".into(),
                category: "File".into(),
                shortcut_hint: Some("Ctrl+S".into()),
                context: None,
                action: TestAction::Save,
            },
            CommandItem {
                title: "Close Window".into(),
                description: "Close the current window".into(),
                category: "Window".into(),
                shortcut_hint: Some("Ctrl+W".into()),
                context: None,
                action: TestAction::Close,
            },
        ]
    }

    #[test]
    fn test_empty_query_returns_all() {
        let commands = create_test_commands();
        let results = filter_commands(&commands, "");
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].1, 0); // Score is 0 for empty query
    }

    #[test]
    fn test_fuzzy_match() {
        let commands = create_test_commands();

        // Search for "save"
        let results = filter_commands(&commands, "save");
        assert!(!results.is_empty());
        assert_eq!(results[0].0.action, TestAction::Save);
    }

    #[test]
    fn test_case_insensitive() {
        let commands = create_test_commands();

        // Search with different case (lowercase query should match Title Case)
        let results = filter_commands(&commands, "save");
        assert!(!results.is_empty());
        assert_eq!(results[0].0.action, TestAction::Save);
    }

    #[test]
    fn test_partial_match() {
        let commands = create_test_commands();

        // "op" should match "Open"
        let results = filter_commands(&commands, "op");
        assert!(!results.is_empty());
        assert_eq!(results[0].0.action, TestAction::Open);
    }

    #[test]
    fn test_no_match() {
        let commands = create_test_commands();

        // Query that doesn't match anything
        let results = filter_commands(&commands, "xyz123");
        assert!(results.is_empty());
    }

    #[test]
    fn test_scoring_order() {
        let commands = create_test_commands();

        // "file" should match both Open File and Save File
        // But exact word match should score higher
        let results = filter_commands(&commands, "file");
        assert!(results.len() >= 2);

        // All results should have positive scores
        for (_, score) in &results {
            assert!(*score > 0);
        }
    }

    #[test]
    fn test_search_includes_category() {
        let commands = create_test_commands();

        // Search by category
        let results = filter_commands(&commands, "window");
        assert!(!results.is_empty());
        assert_eq!(results[0].0.action, TestAction::Close);
    }

    #[test]
    fn test_prefix_match_priority() {
        // Test that prefix matches rank higher than fuzzy matches
        let commands = vec![
            CommandItem {
                title: "Rebase selected PRs".into(),
                description: "Rebase the selected pull requests".into(),
                category: "PR Actions".into(),
                shortcut_hint: Some("r".into()),
                context: None,
                action: TestAction::Open,
            },
            CommandItem {
                title: "Rerun failed jobs".into(),
                description: "Rerun failed CI jobs for current/selected PRs".into(),
                category: "PR Actions".into(),
                shortcut_hint: Some("Shift+R".into()),
                context: None,
                action: TestAction::Save,
            },
            CommandItem {
                title: "Open in IDE".into(),
                description: "Open PR in your IDE for easy rebase".into(),
                category: "PR Actions".into(),
                shortcut_hint: Some("i".into()),
                context: None,
                action: TestAction::Close,
            },
        ];

        // Search for "rebase" - should prioritize "Rebase selected PRs"
        let results = filter_commands(&commands, "rebase");
        assert!(!results.is_empty());
        assert_eq!(results[0].0.action, TestAction::Open); // "Rebase selected PRs"

        // Verify the title prefix match has the highest score
        assert!(results[0].1 > 10000); // Should have prefix boost
    }
}
