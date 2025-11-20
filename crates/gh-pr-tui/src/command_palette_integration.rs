//! Integration of the generic command palette with the gh-pr-tui application
//!
//! This module bridges the generic command palette crate with our specific
//! application types (Action, AppState) by implementing CommandProvider.

use gh_pr_tui_command_palette::{CommandItem, CommandProvider};

use crate::actions::Action;
use crate::shortcuts::{Shortcut, get_all_shortcuts_flat};
use crate::state::AppState;

/// Parse shortcut hint and extract context information
///
/// Extracts context from parentheses (e.g., "a (when console open)" -> ("a", Some("when console open")))
/// Returns clean shortcut without context.
fn parse_shortcut_hint(key_display: &str) -> (String, Option<String>) {
    // Look for parenthetical context like "(when console open)"
    if let Some(paren_start) = key_display.find('(')
        && let Some(paren_end) = key_display.find(')')
    {
        let shortcut = key_display[..paren_start].trim();
        let context = key_display[paren_start + 1..paren_end].trim();
        return (shortcut.to_string(), Some(context.to_string()));
    }

    // No context found
    (key_display.to_string(), None)
}

/// Provides commands from keyboard shortcuts
///
/// This provider exposes all keyboard shortcuts as searchable commands in the palette.
/// Commands are context-filtered based on the current application state.
#[derive(Debug)]
pub struct ShortcutCommandProvider;

impl CommandProvider<Action, AppState> for ShortcutCommandProvider {
    fn commands(&self, state: &AppState) -> Vec<CommandItem<Action>> {
        get_all_shortcuts_flat()
            .into_iter()
            .filter_map(|shortcut| {
                // Skip shortcuts that aren't available in current context
                if !is_shortcut_available(&shortcut, state) {
                    return None;
                }

                // Parse context information from key_display (e.g., "a (when console open)")
                let (shortcut_hint, context) = parse_shortcut_hint(shortcut.key_display);

                // Add asterisk suffix to title if context is present
                let title = if context.is_some() {
                    format!("{} *", shortcut.description)
                } else {
                    shortcut.description.to_string()
                };

                Some(CommandItem {
                    title,
                    description: format!("Keyboard shortcut: {}", shortcut_hint),
                    category: extract_category(&shortcut),
                    shortcut_hint: Some(shortcut_hint),
                    context,
                    action: shortcut.action.clone(),
                })
            })
            .collect()
    }

    fn name(&self) -> &str {
        "Shortcuts"
    }
}

/// Extract category from shortcut based on action type
fn extract_category(shortcut: &Shortcut) -> String {
    match &shortcut.action {
        Action::MergeSelectedPrs
        | Action::ApprovePrs
        | Action::Rebase
        | Action::RerunFailedJobs
        | Action::ShowClosePrPopup => "PR Actions".to_string(),

        Action::SelectNextRepo
        | Action::SelectPreviousRepo
        | Action::SelectRepoByIndex(_)
        | Action::NavigateToNextPr
        | Action::NavigateToPreviousPr => "Navigation".to_string(),

        Action::OpenBuildLogs
        | Action::ToggleTimestamps
        | Action::NextError
        | Action::PrevError
        | Action::CloseLogPanel
        | Action::SelectNextJob
        | Action::SelectPrevJob => "Log Viewer".to_string(),

        Action::CycleFilter | Action::RefreshCurrentRepo | Action::ReloadRepo(_) => {
            "Views & Filters".to_string()
        }

        Action::ToggleShortcuts
        | Action::Quit
        | Action::ShowAddRepoPopup
        | Action::DeleteCurrentRepo
        | Action::ClearPrSelection => "General".to_string(),

        Action::ToggleDebugConsole | Action::ClearDebugLogs | Action::ToggleDebugAutoScroll => {
            "Debug".to_string()
        }

        _ => "Other".to_string(),
    }
}

/// Check if a shortcut is available in the current application context
fn is_shortcut_available(shortcut: &Shortcut, state: &AppState) -> bool {
    let has_prs = state
        .repos
        .repo_data
        .get(&state.repos.selected_repo)
        .map(|d| !d.prs.is_empty())
        .unwrap_or(false);

    let has_selection = state
        .repos
        .repo_data
        .get(&state.repos.selected_repo)
        .map(|d| !d.selected_pr_numbers.is_empty())
        .unwrap_or(false);

    let log_panel_open = state.log_panel.panel.is_some();

    // Determine availability based on action type
    match &shortcut.action {
        // Selection-dependent actions
        Action::MergeSelectedPrs | Action::ApprovePrs | Action::ShowClosePrPopup => has_selection,

        // Rebase can work with or without selection (auto-rebase)
        Action::Rebase => has_selection || has_prs,

        // PR-dependent actions
        Action::OpenBuildLogs
        | Action::OpenCurrentPrInBrowser
        | Action::OpenInIDE
        | Action::TogglePrSelection => has_prs,

        // Log panel actions
        Action::CloseLogPanel
        | Action::SelectNextJob
        | Action::SelectPrevJob
        | Action::ToggleTimestamps
        | Action::NextError
        | Action::PrevError => log_panel_open,

        // Most other actions are always available
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use gh_pr_tui_command_palette::CommandPalette;

    #[test]
    fn test_parse_shortcut_hint() {
        // Test context extraction
        let (hint, context) = parse_shortcut_hint("a (when console open)");
        assert_eq!(hint, "a");
        assert_eq!(context, Some("when console open".to_string()));

        // Test without context
        let (hint, context) = parse_shortcut_hint("Ctrl+P");
        assert_eq!(hint, "Ctrl+P");
        assert_eq!(context, None);

        // Test complex shortcut with context
        let (hint, context) = parse_shortcut_hint("j/k (when console open)");
        assert_eq!(hint, "j/k");
        assert_eq!(context, Some("when console open".to_string()));
    }

    #[test]
    fn test_shortcut_provider() {
        let mut palette = CommandPalette::new();
        palette.register(Box::new(ShortcutCommandProvider));

        let state = AppState::default();
        let commands = palette.all_commands(&state);

        // Should have some commands
        assert!(!commands.is_empty());

        // All commands should have categories
        for cmd in &commands {
            assert!(!cmd.category.is_empty());
        }
    }

    #[test]
    fn test_context_filtering() {
        let mut palette = CommandPalette::new();
        palette.register(Box::new(ShortcutCommandProvider));

        // State with no PRs - PR-dependent actions should not appear
        let empty_state = AppState::default();
        let commands = palette.all_commands(&empty_state);

        // Check that selection-dependent actions aren't present
        assert!(
            !commands
                .iter()
                .any(|cmd| matches!(cmd.action, Action::MergeSelectedPrs))
        );

        // Note: Full context testing with PRs would require building a complete Pr struct
        // which depends on many external types. The context filtering logic is tested
        // by manually verifying the is_shortcut_available function above.
    }
}
