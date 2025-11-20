use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::actions::Action;

/// Shortcut key definition with key matching capability
#[derive(Debug, Clone)]
pub struct Shortcut {
    pub key_display: &'static str,
    pub description: &'static str,
    pub action: Action,
    pub matcher: ShortcutMatcher,
}

/// Matcher for shortcuts - can be single key or two-key combination
#[derive(Clone)]
pub enum ShortcutMatcher {
    /// Single key press
    SingleKey(fn(&KeyEvent) -> bool),
    /// Two-key combination: (first_key, second_key)
    /// Example: ('p', 'a') for "p then a"
    TwoKey(char, char),
}

impl std::fmt::Debug for ShortcutMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShortcutMatcher::SingleKey(_) => write!(f, "SingleKey"),
            ShortcutMatcher::TwoKey(k1, k2) => write!(f, "TwoKey({}, {})", k1, k2),
        }
    }
}

/// Category of shortcuts
#[derive(Debug, Clone)]
pub struct ShortcutCategory {
    pub name: &'static str,
    pub shortcuts: Vec<Shortcut>,
}

impl Shortcut {
    /// Check if this shortcut matches the given key event (for single-key shortcuts)
    pub fn matches(&self, key: &KeyEvent) -> bool {
        match &self.matcher {
            ShortcutMatcher::SingleKey(func) => func(key),
            ShortcutMatcher::TwoKey(_, _) => false, // Two-key shortcuts don't match single key
        }
    }

    /// Check if this is a two-key shortcut with the given first key
    pub fn is_two_key_starting_with(&self, first_key: char) -> bool {
        match &self.matcher {
            ShortcutMatcher::TwoKey(k1, _) => *k1 == first_key,
            _ => false,
        }
    }

    /// Check if this two-key shortcut completes with the given second key
    pub fn completes_two_key_with(&self, second_key: char) -> bool {
        match &self.matcher {
            ShortcutMatcher::TwoKey(_, k2) => *k2 == second_key,
            _ => false,
        }
    }
}

/// Get all shortcut definitions organized by category
pub fn get_shortcuts() -> Vec<ShortcutCategory> {
    vec![
        ShortcutCategory {
            name: "Navigation",
            shortcuts: vec![
                Shortcut {
                    key_display: "↑/↓ or j/k",
                    description: "Navigate through PRs",
                    action: Action::NavigateToNextPr, // Represents both up/down
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(
                            key.code,
                            KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up
                        )
                    }),
                },
                Shortcut {
                    key_display: "Tab or /",
                    description: "Switch to next repository",
                    action: Action::SelectNextRepo,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Tab | KeyCode::Char('/'))
                            && !key.modifiers.contains(KeyModifiers::SHIFT)
                    }),
                },
                Shortcut {
                    key_display: "Shift+Tab",
                    description: "Switch to previous repository",
                    action: Action::SelectPreviousRepo,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
                            && key.modifiers.contains(KeyModifiers::SHIFT)
                            || matches!(key.code, KeyCode::BackTab)
                    }),
                },
                Shortcut {
                    key_display: "1-9",
                    description: "Jump to repository by number",
                    action: Action::SelectRepoByIndex(0), // Placeholder
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('1'..='9'))
                    }),
                },
            ],
        },
        ShortcutCategory {
            name: "PR Actions",
            shortcuts: vec![
                Shortcut {
                    key_display: "Space",
                    description: "Select/deselect PR",
                    action: Action::TogglePrSelection,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char(' '))
                    }),
                },
                Shortcut {
                    key_display: "m",
                    description: "Merge selected PRs",
                    action: Action::MergeSelectedPrs,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('m'))
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                    }),
                },
                Shortcut {
                    key_display: "a",
                    description: "Approve selected PRs",
                    action: Action::ApprovePrs,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('a'))
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                    }),
                },
                Shortcut {
                    key_display: "Ctrl+m",
                    description: "Start merge bot (auto-merge + rebase queue)",
                    action: Action::StartMergeBot,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('m'))
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                    }),
                },
                Shortcut {
                    key_display: "r",
                    description: "Rebase selected PRs (or auto-rebase if none selected)",
                    action: Action::Rebase,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('r'))
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                    }),
                },
                Shortcut {
                    key_display: "Shift+R",
                    description: "Rerun failed CI jobs for current/selected PRs",
                    action: Action::RerunFailedJobs,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('R'))
                            && key.modifiers.contains(KeyModifiers::SHIFT)
                    }),
                },
                Shortcut {
                    key_display: "i",
                    description: "Open PR in IDE",
                    action: Action::OpenInIDE,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('i'))
                    }),
                },
                Shortcut {
                    key_display: "l",
                    description: "View build logs",
                    action: Action::OpenBuildLogs,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('l'))
                    }),
                },
                Shortcut {
                    key_display: "Enter",
                    description: "Open PR in browser",
                    action: Action::OpenCurrentPrInBrowser,
                    matcher: ShortcutMatcher::SingleKey(|key| matches!(key.code, KeyCode::Enter)),
                },
                Shortcut {
                    key_display: "c",
                    description: "Close selected PRs",
                    action: Action::ShowClosePrPopup,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('c'))
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                    }),
                },
            ],
        },
        ShortcutCategory {
            name: "Filters & Views",
            shortcuts: vec![
                Shortcut {
                    key_display: "f",
                    description: "Cycle PR filter (None/Ready/Build Failed)",
                    action: Action::CycleFilter,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('f'))
                    }),
                },
                Shortcut {
                    key_display: "Ctrl+r",
                    description: "Refresh current repository",
                    action: Action::RefreshCurrentRepo,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('r'))
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                    }),
                },
            ],
        },
        ShortcutCategory {
            name: "Log Panel (when open)",
            shortcuts: vec![
                Shortcut {
                    key_display: "↑/↓ or j/k",
                    description: "Navigate through tree (jobs/steps/logs)",
                    action: Action::NavigateToNextPr, // Reused
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(
                            key.code,
                            KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up
                        )
                    }),
                },
                Shortcut {
                    key_display: "Space",
                    description: "Page down (scroll by screen height)",
                    action: Action::PageLogPanelDown,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char(' '))
                    }),
                },
                Shortcut {
                    key_display: "←/→ or h/l",
                    description: "Scroll horizontally",
                    action: Action::ScrollLogPanelLeft, // Represents both
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(
                            key.code,
                            KeyCode::Char('h')
                                | KeyCode::Left
                                | KeyCode::Right
                                | KeyCode::Char('l')
                        )
                    }),
                },
                Shortcut {
                    key_display: "n",
                    description: "Jump to next failed step/job",
                    action: Action::NextError,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('n'))
                    }),
                },
                Shortcut {
                    key_display: "p",
                    description: "Jump to previous failed step/job",
                    action: Action::PrevError,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('p'))
                    }),
                },
                Shortcut {
                    key_display: "Enter",
                    description: "Expand/collapse tree node",
                    action: Action::ToggleTreeNode,
                    matcher: ShortcutMatcher::SingleKey(|key| matches!(key.code, KeyCode::Enter)),
                },
                Shortcut {
                    key_display: "t",
                    description: "Toggle timestamps",
                    action: Action::ToggleTimestamps,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('t'))
                    }),
                },
                Shortcut {
                    key_display: "x or Esc",
                    description: "Close log panel",
                    action: Action::CloseLogPanel,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('x') | KeyCode::Esc)
                    }),
                },
            ],
        },
        ShortcutCategory {
            name: "Debug",
            shortcuts: vec![
                Shortcut {
                    key_display: "` or ~",
                    description: "Toggle debug console",
                    action: Action::ToggleDebugConsole,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('`') | KeyCode::Char('~'))
                    }),
                },
                Shortcut {
                    key_display: "j/k (when console open)",
                    description: "Scroll debug console",
                    action: Action::ScrollDebugConsoleDown, // Represents both
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('j') | KeyCode::Char('k'))
                    }),
                },
                Shortcut {
                    key_display: "a (when console open)",
                    description: "Toggle auto-scroll",
                    action: Action::ToggleDebugAutoScroll,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('a'))
                    }),
                },
                Shortcut {
                    key_display: "c (when console open)",
                    description: "Clear debug logs",
                    action: Action::ClearDebugLogs,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('c'))
                    }),
                },
            ],
        },
        ShortcutCategory {
            name: "General",
            shortcuts: vec![
                Shortcut {
                    key_display: "Ctrl+P",
                    description: "Open command palette",
                    action: Action::ShowCommandPalette,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('p'))
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                    }),
                },
                Shortcut {
                    key_display: "?",
                    description: "Toggle this help (you are here!)",
                    action: Action::ToggleShortcuts,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('?'))
                    }),
                },
                Shortcut {
                    key_display: "Esc → Esc",
                    description: "Clear all PR selections",
                    action: Action::ClearPrSelection,
                    matcher: ShortcutMatcher::SingleKey(|_| false), // Handled specially in main.rs
                },
                Shortcut {
                    key_display: "p → a",
                    description: "Add new repository",
                    action: Action::ShowAddRepoPopup,
                    matcher: ShortcutMatcher::TwoKey('p', 'a'),
                },
                Shortcut {
                    key_display: "p → d",
                    description: "Drop current repository",
                    action: Action::DeleteCurrentRepo,
                    matcher: ShortcutMatcher::TwoKey('p', 'd'),
                },
                Shortcut {
                    key_display: "q",
                    description: "Quit application",
                    action: Action::Quit,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('q'))
                    }),
                },
            ],
        },
    ]
}

/// Get all shortcuts in a flat list for easy iteration
pub fn get_all_shortcuts_flat() -> Vec<Shortcut> {
    get_shortcuts()
        .into_iter()
        .flat_map(|category| category.shortcuts)
        .collect()
}

/// Find the action for a given key event, handling two-key combinations
/// Returns (action, should_clear_pending_key, new_pending_key)
pub fn find_action_for_key_with_pending(
    key: &KeyEvent,
    pending_key: Option<&crate::state::PendingKeyPress>,
) -> (Action, bool, Option<char>) {
    const TWO_KEY_TIMEOUT_SECS: u64 = 3;

    // Get the current character if it's a simple char press
    let current_char = if let KeyCode::Char(c) = key.code {
        if !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            Some(c)
        } else {
            None
        }
    } else {
        None
    };

    // Check if we have a valid pending key (not timed out)
    let valid_pending =
        pending_key.filter(|p| p.timestamp.elapsed().as_secs() < TWO_KEY_TIMEOUT_SECS);

    // If we have a valid pending key, try to complete a two-key combination
    if let (Some(pending), Some(current)) = (valid_pending, current_char) {
        for shortcut in get_all_shortcuts_flat() {
            if shortcut.is_two_key_starting_with(pending.key)
                && shortcut.completes_two_key_with(current)
            {
                // Two-key combination matched!
                return (shortcut.action.clone(), true, None);
            }
        }
        // Pending key didn't match, clear it and process current key normally
        return (find_single_key_action(key, current_char), true, None);
    }

    // No valid pending key - check if current key starts a two-key combination
    if let Some(current) = current_char {
        for shortcut in get_all_shortcuts_flat() {
            if shortcut.is_two_key_starting_with(current) {
                // This key starts a two-key combination - save it as pending
                return (Action::None, false, Some(current));
            }
        }
    }

    // Not a two-key combo - process as single key
    (find_single_key_action(key, current_char), true, None)
}

/// Find action for a single key press (no two-key combination logic)
fn find_single_key_action(key: &KeyEvent, current_char: Option<char>) -> Action {
    // Handle special cases for number keys (repo selection)
    if let Some(c) = current_char
        && c.is_ascii_digit()
        && c != '0'
    {
        let index = c.to_digit(10).unwrap() as usize - 1;
        return Action::SelectRepoByIndex(index);
    }

    // Handle up/down separately since they map to different actions
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => return Action::NavigateToPreviousPr,
        KeyCode::Down | KeyCode::Char('j') => return Action::NavigateToNextPr,
        KeyCode::Left | KeyCode::Char('h') => return Action::ScrollLogPanelLeft,
        KeyCode::Right => return Action::ScrollLogPanelRight,
        _ => {}
    }

    // Check all shortcuts for a match (only single-key shortcuts)
    for shortcut in get_all_shortcuts_flat() {
        if shortcut.matches(key) {
            return shortcut.action.clone();
        }
    }

    Action::None
}
