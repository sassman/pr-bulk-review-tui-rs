//! Integration of the generic command palette with the gh-pr-tui application
//!
//! This module bridges the generic command palette crate with our specific
//! application types (Action, AppState) by implementing CommandProvider.

use gh_pr_tui_command_palette::{CommandItem, CommandProvider};

use crate::actions::Action;
use crate::shortcuts::{get_all_shortcuts_flat, Shortcut};
use crate::state::AppState;

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

                Some(CommandItem {
                    title: shortcut.description.to_string(),
                    description: format!("Keyboard shortcut: {}", shortcut.key_display),
                    category: extract_category(&shortcut),
                    shortcut_hint: Some(shortcut.key_display.to_string()),
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

        Action::ToggleDebugConsole
        | Action::ClearDebugLogs
        | Action::ToggleDebugAutoScroll => "Debug".to_string(),

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
    use crate::state::{AppState, RepoData, ReposState};
    use gh_pr_tui_command_palette::CommandPalette;
    use std::collections::HashMap;

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
        assert!(!commands
            .iter()
            .any(|cmd| matches!(cmd.action, Action::MergeSelectedPrs)));

        // State with PRs and selection
        let mut state_with_prs = AppState::default();
        let mut repo_data = RepoData::default();
        repo_data.selected_pr_numbers.insert(1);
        repo_data.prs = vec![crate::pr::Pr {
            number: 1,
            title: "Test PR".to_string(),
            author: "test".to_string(),
            url: "http://example.com".to_string(),
            status: crate::pr::PrStatus::Ready,
            mergeable_status: crate::pr::MergeableStatus::Unknown,
            needs_rebase: false,
            comments: 0,
            is_dependabot: false,
        }];

        let mut repo_data_map = HashMap::new();
        repo_data_map.insert(0, repo_data);

        state_with_prs.repos = ReposState {
            repo_data: repo_data_map,
            selected_repo: 0,
            ..Default::default()
        };

        let commands_with_selection = palette.all_commands(&state_with_prs);

        // Now selection-dependent actions should be present
        assert!(commands_with_selection
            .iter()
            .any(|cmd| matches!(cmd.action, Action::MergeSelectedPrs)));
    }
}
