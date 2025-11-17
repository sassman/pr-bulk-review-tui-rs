use anyhow::Result;
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    prelude::*,
    widgets::*,
};

// Action enum - represents all possible actions in the application
#[derive(Debug, Clone)]
pub enum Action {
    // User-initiated actions
    Bootstrap,
    Rebase,
    RefreshCurrentRepo,
    ReloadRepo(usize), // Reload specific repo by index (e.g., after PR merged)
    RerunFailedJobs,
    CycleFilter,
    SelectNextRepo,
    SelectPreviousRepo,
    SelectRepoByIndex(usize),
    TogglePrSelection,
    NavigateToNextPr,
    NavigateToPreviousPr,
    MergeSelectedPrs,
    ApprovePrs,
    StartMergeBot,
    OpenCurrentPrInBrowser,
    OpenBuildLogs,
    OpenInIDE,
    CloseLogPanel,
    // Log panel - tree navigation
    SelectNextJob,
    SelectPrevJob,
    FocusJobList,
    FocusLogViewer,
    ToggleTreeNode,  // Toggle expand/collapse at cursor
    // Log panel - log viewer scrolling
    ScrollLogPanelUp,
    ScrollLogPanelDown,
    PageLogPanelDown,
    ScrollLogPanelLeft,
    ScrollLogPanelRight,
    // Log panel - step and error navigation
    NextStep,
    PrevStep,
    NextError,        // Jump to next step/job with errors
    PrevError,        // Jump to previous step/job with errors
    NextLogSection,   // Error navigation (kept for backwards compat)
    PrevLogSection,   // Error navigation (kept for backwards compat)
    ToggleTimestamps,
    ToggleShortcuts,
    ScrollShortcutsUp,
    ScrollShortcutsDown,

    // Add repository popup
    ShowAddRepoPopup,
    HideAddRepoPopup,
    AddRepoFormInput(char),
    AddRepoFormBackspace,
    AddRepoFormNextField,
    AddRepoFormSubmit,

    // Repository management
    DeleteCurrentRepo,

    // State update actions (dispatched internally)
    SetBootstrapState(crate::state::BootstrapState),
    SetLoadingState(crate::state::LoadingState),
    SetTaskStatus(Option<crate::state::TaskStatus>),
    SetReposLoading(Vec<usize>), // Set multiple repos to loading state
    TickSpinner,                 // Increment spinner animation frame

    // Background task completion notifications
    BootstrapComplete(Result<BootstrapResult, String>),
    RepoLoadingStarted(usize), // Sent when we start fetching repo data
    RepoDataLoaded(usize, Result<Vec<crate::pr::Pr>, String>),
    RefreshComplete(Result<Vec<crate::pr::Pr>, String>),
    MergeStatusUpdated(usize, usize, crate::pr::MergeableStatus), // repo_index, pr_number, status
    RebaseStatusUpdated(usize, usize, bool), // repo_index, pr_number, needs_rebase
    CommentCountUpdated(usize, usize, usize), // repo_index, pr_number, comment_count
    RebaseComplete(Result<(), String>),
    MergeComplete(Result<(), String>),
    RerunJobsComplete(Result<(), String>),
    ApprovalComplete(Result<(), String>),
    PRMergedConfirmed(usize, usize, bool), // repo_index, pr_number, is_merged
    BuildLogsLoaded(Vec<(crate::log::JobMetadata, gh_actions_log_parser::JobLog)>, crate::log::PrContext),
    IDEOpenComplete(Result<(), String>),

    // Auto-merge queue management
    AddToAutoMergeQueue(usize, usize),      // repo_index, pr_number
    RemoveFromAutoMergeQueue(usize, usize), // repo_index, pr_number
    AutoMergeStatusCheck(usize, usize),     // repo_index, pr_number - periodic check

    // Operation monitoring (rebase/merge progress tracking)
    StartOperationMonitor(usize, usize, crate::state::OperationType), // repo_index, pr_number, operation
    OperationMonitorCheck(usize, usize),  // repo_index, pr_number - periodic check
    RemoveFromOperationMonitor(usize, usize), // repo_index, pr_number

    // Debug console (Quake-style drop-down)
    ToggleDebugConsole,
    ScrollDebugConsoleUp,
    ScrollDebugConsoleDown,
    PageDebugConsoleDown,
    ToggleDebugAutoScroll,
    ClearDebugLogs,

    // Viewport height updates (for page down scrolling)
    UpdateLogPanelViewport(usize),
    UpdateDebugConsoleViewport(usize),

    Quit,
    None,
}

// Result types for background tasks
#[derive(Debug, Clone)]
pub struct BootstrapResult {
    pub repos: Vec<crate::Repo>,
    pub selected_repo: usize,
}

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
                            KeyCode::Char('h') | KeyCode::Left | KeyCode::Right | KeyCode::Char('l')
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
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Enter)
                    }),
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
                    key_display: "?",
                    description: "Toggle this help (you are here!)",
                    action: Action::ToggleShortcuts,
                    matcher: ShortcutMatcher::SingleKey(|key| {
                        matches!(key.code, KeyCode::Char('?'))
                    }),
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
    if let Some(c) = current_char {
        if c.is_ascii_digit() && c != '0' {
            let index = c.to_digit(10).unwrap() as usize - 1;
            return Action::SelectRepoByIndex(index);
        }
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

/// Render the shortcuts help panel as a centered floating window
/// Returns the maximum scroll offset
pub fn render_shortcuts_panel(
    f: &mut Frame,
    area: Rect,
    scroll_offset: usize,
    theme: &crate::theme::Theme,
) -> usize {
    use ratatui::widgets::{Clear, Wrap};

    // Calculate centered area (80% width, 90% height)
    let popup_width = (area.width * 80 / 100).min(100);
    let popup_height = (area.height * 90 / 100).min(40);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect {
        x: area.x + popup_x,
        y: area.y + popup_y,
        width: popup_width,
        height: popup_height,
    };

    // Clear the area and render background
    f.render_widget(Clear, popup_area);
    f.render_widget(
        Block::default().style(Style::default().bg(theme.bg_panel)),
        popup_area,
    );

    // Calculate inner area and split into content and sticky footer
    let inner = popup_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    // Split inner area: content area and 1-line footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Scrollable content
            Constraint::Length(1), // Sticky footer
        ])
        .split(inner);

    let content_area = chunks[0];
    let footer_area = chunks[1];

    // Build text content (without footer - it will be rendered separately)
    let mut text_lines = Vec::new();

    for category in get_shortcuts() {
        // Category header
        text_lines.push(Line::from(vec![Span::styled(
            category.name,
            Style::default()
                .fg(theme.status_warning)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )]));
        text_lines.push(Line::from(""));

        // Items in this category
        for shortcut in category.shortcuts {
            text_lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:18}", shortcut.key_display),
                    Style::default()
                        .fg(theme.status_success)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    shortcut.description,
                    Style::default().fg(theme.text_secondary),
                ),
            ]));
        }

        text_lines.push(Line::from(""));
    }

    // Calculate visible area and apply scrolling
    let total_lines = text_lines.len();
    let visible_height = content_area.height as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let actual_scroll = scroll_offset.min(max_scroll);

    // Add scroll indicator to title if scrollable
    let title = if total_lines > visible_height {
        format!(
            " Keyboard Shortcuts  [{}/{}] ",
            actual_scroll + 1,
            total_lines
        )
    } else {
        " Keyboard Shortcuts ".to_string()
    };

    // Render block with updated title
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(theme.bg_panel));

    f.render_widget(block, popup_area);

    // Render scrollable content
    let paragraph = Paragraph::new(text_lines)
        .wrap(Wrap { trim: false })
        .scroll((actual_scroll as u16, 0))
        .style(Style::default().bg(theme.bg_panel));

    f.render_widget(paragraph, content_area);

    // Render sticky footer at the bottom
    let footer_line = Line::from(vec![
        Span::styled("Press ", Style::default().fg(theme.text_muted)),
        Span::styled(
            "x",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(theme.text_muted)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to close this help", Style::default().fg(theme.text_muted)),
    ]);

    let footer = Paragraph::new(footer_line)
        .style(Style::default().bg(theme.bg_panel))
        .alignment(ratatui::layout::Alignment::Center);

    f.render_widget(footer, footer_area);

    // Return the max scroll value so it can be stored in app state
    max_scroll
}
