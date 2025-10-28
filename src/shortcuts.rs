use anyhow::Result;
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    prelude::*,
    style::palette::tailwind,
    widgets::*,
};

// Action enum - represents all possible actions in the application
#[derive(Debug, Clone)]
pub enum Action {
    Bootstrap,
    Rebase,
    RefreshCurrentRepo,
    CycleFilter,
    SelectNextRepo,
    SelectPreviousRepo,
    SelectRepoByIndex(usize),
    TogglePrSelection,
    NavigateToNextPr,
    NavigateToPreviousPr,
    MergeSelectedPrs,
    StartMergeBot,
    OpenCurrentPrInBrowser,
    OpenBuildLogs,
    OpenInIDE,
    CloseLogPanel,
    ScrollLogPanelUp,
    ScrollLogPanelDown,
    ScrollLogPanelLeft,
    ScrollLogPanelRight,
    NextLogSection,
    ToggleTimestamps,
    ToggleShortcuts,
    ScrollShortcutsUp,
    ScrollShortcutsDown,

    // Background task completion notifications
    BootstrapComplete(Result<BootstrapResult, String>),
    RepoDataLoaded(usize, Result<Vec<crate::pr::Pr>, String>),
    RefreshComplete(Result<Vec<crate::pr::Pr>, String>),
    MergeStatusUpdated(usize, usize, crate::pr::MergeableStatus), // repo_index, pr_number, status
    RebaseStatusUpdated(usize, usize, bool), // repo_index, pr_number, needs_rebase
    RebaseComplete(Result<(), String>),
    MergeComplete(Result<(), String>),
    PRMergedConfirmed(usize, usize, bool), // repo_index, pr_number, is_merged
    BuildLogsLoaded(Vec<crate::log::LogSection>, crate::log::PrContext),
    IDEOpenComplete(Result<(), String>),

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
    pub matcher: fn(&KeyEvent) -> bool,
}

/// Category of shortcuts
#[derive(Debug, Clone)]
pub struct ShortcutCategory {
    pub name: &'static str,
    pub shortcuts: Vec<Shortcut>,
}

impl Shortcut {
    /// Check if this shortcut matches the given key event
    pub fn matches(&self, key: &KeyEvent) -> bool {
        (self.matcher)(key)
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
                    matcher: |key| {
                        matches!(
                            key.code,
                            KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up
                        )
                    },
                },
                Shortcut {
                    key_display: "Tab or /",
                    description: "Switch to next repository",
                    action: Action::SelectNextRepo,
                    matcher: |key| {
                        matches!(key.code, KeyCode::Tab | KeyCode::Char('/'))
                            && !key.modifiers.contains(KeyModifiers::SHIFT)
                    },
                },
                Shortcut {
                    key_display: "Shift+Tab",
                    description: "Switch to previous repository",
                    action: Action::SelectPreviousRepo,
                    matcher: |key| {
                        matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
                            && key.modifiers.contains(KeyModifiers::SHIFT)
                            || matches!(key.code, KeyCode::BackTab)
                    },
                },
                Shortcut {
                    key_display: "1-9",
                    description: "Jump to repository by number",
                    action: Action::SelectRepoByIndex(0), // Placeholder
                    matcher: |key| matches!(key.code, KeyCode::Char('1'..='9')),
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
                    matcher: |key| matches!(key.code, KeyCode::Char(' ')),
                },
                Shortcut {
                    key_display: "m",
                    description: "Merge selected PRs",
                    action: Action::MergeSelectedPrs,
                    matcher: |key| matches!(key.code, KeyCode::Char('m')) && !key.modifiers.contains(KeyModifiers::CONTROL),
                },
                Shortcut {
                    key_display: "Ctrl+m",
                    description: "Start merge bot (auto-merge + rebase queue)",
                    action: Action::StartMergeBot,
                    matcher: |key| matches!(key.code, KeyCode::Char('m')) && key.modifiers.contains(KeyModifiers::CONTROL),
                },
                Shortcut {
                    key_display: "r",
                    description: "Rebase selected PRs (or auto-rebase if none selected)",
                    action: Action::Rebase,
                    matcher: |key| {
                        matches!(key.code, KeyCode::Char('r'))
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                    },
                },
                Shortcut {
                    key_display: "i",
                    description: "Open PR in IDE",
                    action: Action::OpenInIDE,
                    matcher: |key| matches!(key.code, KeyCode::Char('i')),
                },
                Shortcut {
                    key_display: "l",
                    description: "View build logs",
                    action: Action::OpenBuildLogs,
                    matcher: |key| matches!(key.code, KeyCode::Char('l')),
                },
                Shortcut {
                    key_display: "Enter",
                    description: "Open PR in browser",
                    action: Action::OpenCurrentPrInBrowser,
                    matcher: |key| matches!(key.code, KeyCode::Enter),
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
                    matcher: |key| matches!(key.code, KeyCode::Char('f')),
                },
                Shortcut {
                    key_display: "Ctrl+r",
                    description: "Refresh current repository",
                    action: Action::RefreshCurrentRepo,
                    matcher: |key| {
                        matches!(key.code, KeyCode::Char('r'))
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                    },
                },
            ],
        },
        ShortcutCategory {
            name: "Log Panel (when open)",
            shortcuts: vec![
                Shortcut {
                    key_display: "↑/↓ or j/k",
                    description: "Scroll vertically",
                    action: Action::NavigateToNextPr, // Reused
                    matcher: |key| {
                        matches!(
                            key.code,
                            KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up
                        )
                    },
                },
                Shortcut {
                    key_display: "←/→ or h",
                    description: "Scroll horizontally",
                    action: Action::ScrollLogPanelLeft, // Represents both
                    matcher: |key| {
                        matches!(
                            key.code,
                            KeyCode::Char('h') | KeyCode::Left | KeyCode::Right
                        )
                    },
                },
                Shortcut {
                    key_display: "n",
                    description: "Jump to next error section",
                    action: Action::NextLogSection,
                    matcher: |key| matches!(key.code, KeyCode::Char('n')),
                },
                Shortcut {
                    key_display: "t",
                    description: "Toggle timestamps",
                    action: Action::ToggleTimestamps,
                    matcher: |key| matches!(key.code, KeyCode::Char('t')),
                },
                Shortcut {
                    key_display: "x or Esc",
                    description: "Close log panel",
                    action: Action::CloseLogPanel,
                    matcher: |key| matches!(key.code, KeyCode::Char('x') | KeyCode::Esc),
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
                    matcher: |key| matches!(key.code, KeyCode::Char('?')),
                },
                Shortcut {
                    key_display: "q",
                    description: "Quit application",
                    action: Action::Quit,
                    matcher: |key| matches!(key.code, KeyCode::Char('q')),
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

/// Find the action for a given key event by checking all shortcuts
pub fn find_action_for_key(key: &KeyEvent) -> Action {
    // Handle special cases for number keys (repo selection)
    if let KeyCode::Char(c) = key.code {
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

    // Check all shortcuts for a match
    for shortcut in get_all_shortcuts_flat() {
        if shortcut.matches(key) {
            return shortcut.action.clone();
        }
    }

    Action::None
}

/// Render the shortcuts help panel as a centered floating window
/// Returns the maximum scroll offset
pub fn render_shortcuts_panel(f: &mut Frame, area: Rect, scroll_offset: usize, theme: &crate::theme::Theme) -> usize {
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
        Block::default().style(Style::default().bg(tailwind::SLATE.c800)),
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
                .fg(tailwind::YELLOW.c400)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )]));
        text_lines.push(Line::from(""));

        // Items in this category
        for shortcut in category.shortcuts {
            text_lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:18}", shortcut.key_display),
                    Style::default()
                        .fg(tailwind::GREEN.c400)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    shortcut.description,
                    Style::default().fg(tailwind::SLATE.c200),
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
                .fg(tailwind::CYAN.c400)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(
            Style::default()
                .fg(tailwind::CYAN.c400)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(tailwind::SLATE.c800));

    f.render_widget(block, popup_area);

    // Render scrollable content
    let paragraph = Paragraph::new(text_lines)
        .wrap(Wrap { trim: false })
        .scroll((actual_scroll as u16, 0))
        .style(Style::default().bg(tailwind::SLATE.c800));

    f.render_widget(paragraph, content_area);

    // Render sticky footer at the bottom
    let footer_line = Line::from(vec![
        Span::styled("Press ", Style::default().fg(tailwind::SLATE.c400)),
        Span::styled(
            "x",
            Style::default()
                .fg(tailwind::CYAN.c400)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(tailwind::SLATE.c400)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(tailwind::CYAN.c400)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " to close this help",
            Style::default().fg(tailwind::SLATE.c400),
        ),
    ]);

    let footer = Paragraph::new(footer_line)
        .style(Style::default().bg(tailwind::SLATE.c800))
        .alignment(ratatui::layout::Alignment::Center);

    f.render_widget(footer, footer_area);

    // Return the max scroll value so it can be stored in app state
    max_scroll
}
