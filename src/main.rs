use anyhow::{Context, Result, bail};
use octocrab::{Octocrab, params};
use ratatui::{
    crossterm::{
        self,
        event::{self, Event, KeyEvent, KeyEventKind},
    },
    layout::Margin,
    prelude::*,
    style::palette::tailwind,
    widgets::*,
};
use serde::{Deserialize, Serialize};
use std::{env, fs::File, io::BufReader, path::PathBuf};
use tokio::sync::mpsc;

// Import debug from the log crate using :: prefix to disambiguate from our log module
use ::log::debug;

use crate::config::Config;
use crate::effect::{execute_effect, Effect};
use crate::gh::{comment, merge};
use crate::log::{LogSection, PrContext};
use crate::pr::Pr;
use crate::shortcuts::{Action, BootstrapResult};
use crate::state::*;
use crate::store::Store;
use crate::task::{start_task_worker, BackgroundTask};
use crate::theme::Theme;

mod config;
mod effect;
mod gh;
mod log;
mod merge_bot;
mod pr;
mod reducer;
mod shortcuts;
mod state;
mod store;
mod task;
mod theme;

pub struct App {
    // Redux store - centralized state management
    pub store: Store,
    // Communication channels
    pub action_tx: mpsc::UnboundedSender<Action>,
    pub task_tx: mpsc::UnboundedSender<BackgroundTask>,
    // Lazy-initialized octocrab client (created after .env is loaded)
    pub octocrab: Option<Octocrab>,
}

#[derive(Debug, Serialize, Deserialize, Eq, Clone, PartialEq)]
struct PersistedState {
    selected_repo: Repo,
}

pub fn initialize_panic_handler() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        shutdown().unwrap();
        original_hook(panic_info);
    }));
}

fn startup() -> Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stderr(), crossterm::terminal::EnterAlternateScreen)?;
    Ok(())
}

fn shutdown() -> Result<()> {
    crossterm::execute!(std::io::stderr(), crossterm::terminal::LeaveAlternateScreen)?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}

// Background task definitions moved to task.rs module

async fn update(app: &mut App, msg: Action) -> Result<Action> {
    // When shortcuts panel is open, remap navigation to shortcuts scrolling
    let msg = if app.store.state().ui.show_shortcuts {
        match msg {
            Action::NavigateToNextPr => Action::ScrollShortcutsDown,
            Action::NavigateToPreviousPr => Action::ScrollShortcutsUp,
            Action::ToggleShortcuts | Action::CloseLogPanel => msg,
            Action::Quit => Action::ToggleShortcuts,
            _ => {
                // Ignore all other actions when shortcuts panel is open
                return Ok(Action::None);
            }
        }
    } else {
        msg
    };

    // Pure Redux/Elm architecture: Dispatch action to reducers, get effects back
    let effects = app.store.dispatch(msg);

    // Execute effects returned by reducers
    for effect in effects {
        execute_effect(app, effect).await?;
    }

    Ok(Action::None)
}

fn start_event_handler(
    _app: &App,
    tx: mpsc::UnboundedSender<Action>,
) -> tokio::task::JoinHandle<()> {
    let tick_rate = std::time::Duration::from_millis(250);
    tokio::spawn(async move {
        loop {
            let action = if crossterm::event::poll(tick_rate).unwrap() {
                handle_events().unwrap_or(Action::None)
            } else {
                Action::None
            };

            if let Err(_) = tx.send(action) {
                break;
            }
        }
    })
}

async fn run() -> Result<()> {
    let mut t = Terminal::new(CrosstermBackend::new(std::io::stderr()))?;

    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    let (task_tx, task_rx) = mpsc::unbounded_channel();

    let mut app = App::new(action_tx.clone(), task_tx);

    let event_task = start_event_handler(&app, app.action_tx.clone());
    let worker_task = start_task_worker(task_rx, action_tx.clone());

    app.action_tx
        .send(Action::Bootstrap)
        .expect("Failed to send bootstrap action");

    loop {
        // Increment spinner frame for animation
        app.store.state_mut().ui.spinner_frame = (app.store.state().ui.spinner_frame + 1) % 10;

        t.draw(|f| {
            ui(f, &mut app);
        })?;

        // Use timeout to ensure UI updates even without actions (for spinner and progress bar)
        let action =
            tokio::time::timeout(std::time::Duration::from_millis(100), action_rx.recv()).await;

        match action {
            Ok(Some(action)) => {
                if let Err(err) = update(&mut app, action).await {
                    app.store.state_mut().repos.loading_state =
                        LoadingState::Error(err.to_string());
                    app.store.state_mut().ui.should_quit = true;
                    debug!("Error updating app: {}", err);
                }
            }
            Ok(None) => break, // Channel closed
            Err(_) => {
                // Timeout - continue to redraw (for spinner animation and progress updates)
                // Also step the merge bot if it's running
                if app.store.state().merge_bot.bot.is_running() {
                    if let Some(repo) = app.repo().cloned() {
                        let repo_data = app.get_current_repo_data();

                        // Process next PR in queue
                        if let Some(action) = app
                            .store
                            .state_mut()
                            .merge_bot
                            .bot
                            .process_next(&repo_data.prs)
                        {
                            use crate::merge_bot::MergeBotAction;
                            match action {
                                MergeBotAction::DispatchMerge(indices) => {
                                    // Dispatch merge action
                                    if let Ok(octocrab) = app.octocrab() {
                                        let _ = app.task_tx.send(BackgroundTask::Merge {
                                            repo: repo.clone(),
                                            prs: repo_data.prs.clone(),
                                            selected_indices: indices,
                                            octocrab,
                                        });
                                    }
                                    app.store.state_mut().task.status = Some(TaskStatus {
                                        message: app.store.state().merge_bot.bot.status_message(),
                                        status_type: TaskStatusType::Running,
                                    });
                                }
                                MergeBotAction::DispatchRebase(indices) => {
                                    // Dispatch rebase action
                                    if let Ok(octocrab) = app.octocrab() {
                                        let _ = app.task_tx.send(BackgroundTask::Rebase {
                                            repo: repo.clone(),
                                            prs: repo_data.prs.clone(),
                                            selected_indices: indices,
                                            octocrab,
                                        });
                                    }
                                    app.store.state_mut().task.status = Some(TaskStatus {
                                        message: app.store.state().merge_bot.bot.status_message(),
                                        status_type: TaskStatusType::Running,
                                    });
                                }
                                MergeBotAction::WaitForCI(_pr_number) => {
                                    // Just update status, CI waiting is now handled via polling
                                    app.store.state_mut().task.status = Some(TaskStatus {
                                        message: app.store.state().merge_bot.bot.status_message(),
                                        status_type: TaskStatusType::Running,
                                    });
                                }
                                MergeBotAction::PollMergeStatus(pr_number, is_checking_ci) => {
                                    // Dispatch polling task to check PR status
                                    // is_checking_ci determines sleep duration: 15s for CI, 2s for merge
                                    if let Ok(octocrab) = app.octocrab() {
                                        let _ =
                                            app.task_tx.send(BackgroundTask::PollPRMergeStatus {
                                                repo_index: app.store.state().repos.selected_repo,
                                                repo: repo.clone(),
                                                pr_number,
                                                octocrab,
                                                is_checking_ci,
                                            });
                                    }
                                    app.store.state_mut().task.status = Some(TaskStatus {
                                        message: app.store.state().merge_bot.bot.status_message(),
                                        status_type: TaskStatusType::Running,
                                    });
                                }
                                MergeBotAction::PrSkipped(_pr_number, _reason) => {
                                    // Update status and continue
                                    app.store.state_mut().task.status = Some(TaskStatus {
                                        message: app.store.state().merge_bot.bot.status_message(),
                                        status_type: TaskStatusType::Running,
                                    });
                                }
                                MergeBotAction::Completed => {
                                    app.store.state_mut().task.status = Some(TaskStatus {
                                        message: app.store.state().merge_bot.bot.status_message(),
                                        status_type: TaskStatusType::Success,
                                    });
                                    // Refresh the PR list
                                    if let Ok(octocrab) = app.octocrab() {
                                        let _ = app.task_tx.send(BackgroundTask::LoadSingleRepo {
                                            repo_index: app.store.state().repos.selected_repo,
                                            repo: repo.clone(),
                                            filter: app.store.state().repos.filter.clone(),
                                            octocrab,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if app.store.state().ui.should_quit {
            store_recent_repos(&app.store.state().repos.recent_repos)?;
            if let Some(repo) = app.repo().cloned() {
                let persisted_state = PersistedState {
                    selected_repo: repo,
                };
                store_persisted_state(&persisted_state)?;
            }
            break;
        }
    }

    event_task.abort();
    worker_task.abort();

    Ok(())
}

fn ui(f: &mut Frame, app: &mut App) {
    // Show bootstrap status if not completed
    if app.store.state().repos.bootstrap_state != BootstrapState::Completed {
        render_bootstrap_screen(f, app);
        return;
    }

    // If no repositories at all (shouldn't happen after bootstrap completes)
    if app.store.state().repos.recent_repos.is_empty() {
        f.render_widget(
            Paragraph::new(
                "No repositories configured. Add repositories to .recent-repositories.json",
            )
            .centered(),
            f.area(),
        );
        return;
    }

    // Split the layout: tabs on top, table in middle, action panel and status at bottom
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tabs
            Constraint::Min(0),    // Table (full width)
            Constraint::Length(3), // Action panel
            Constraint::Length(1), // Status line
        ])
        .split(f.area());

    // Table always takes full width
    let table_area = chunks[1];

    // Render tabs (always visible when there are repos)
    let tab_titles: Vec<Line> = app
        .store
        .state()
        .repos
        .recent_repos
        .iter()
        .enumerate()
        .map(|(i, repo)| {
            let number = if i < 9 {
                format!("{} ", i + 1)
            } else {
                String::new()
            };
            Line::from(format!("{}{}/{}", number, repo.org, repo.repo))
        })
        .collect();

    let tabs = Tabs::new(tab_titles)
        .block(Block::default().borders(Borders::ALL).title(format!(
            "Projects [Tab/1-9: switch, /: cycle] | Filter: {} [f: cycle]",
            app.store.state().repos.filter.label()
        )))
        .select(app.store.state().repos.selected_repo)
        .style(Style::default().fg(app.store.state().repos.colors.row_fg))
        .highlight_style(
            Style::default()
                .fg(app.store.state().repos.colors.selected_row_style_fg)
                .add_modifier(Modifier::BOLD)
                .bg(app.store.state().repos.colors.header_bg),
        );

    f.render_widget(tabs, chunks[0]);

    // Get the selected repo (should always exist if we have repos)
    let Some(selected_repo) = app.repo() else {
        f.render_widget(
            Paragraph::new("Error: Invalid repository selection").centered(),
            table_area,
        );
        return;
    };

    // Get the current repo data
    let repo_data = app.get_current_repo_data();

    // Format the loading state with refresh hint
    let status_text = match &repo_data.loading_state {
        LoadingState::Idle => "Idle [Ctrl+r to refresh]".to_string(),
        LoadingState::Loading => "Loading...".to_string(),
        LoadingState::Loaded => "Loaded [Ctrl+r to refresh]".to_string(),
        LoadingState::Error(err) => {
            // Truncate error if too long
            let err_short = if err.len() > 30 {
                format!("{}...", &err[..30])
            } else {
                err.clone()
            };
            format!("Error: {} [Ctrl+r to retry]", err_short)
        }
    };
    let loading_state = Line::from(status_text).right_aligned();

    let block = Block::default()
        .title(format!(
            "GitHub PRs: {}/{}@{}",
            &selected_repo.org, &selected_repo.repo, &selected_repo.branch
        ))
        .title(loading_state)
        .borders(Borders::ALL);

    let header_style = Style::default()
        .fg(app.store.state().repos.colors.header_fg)
        .bg(app.store.state().repos.colors.header_bg);

    let header_cells = ["#PR", "Description", "Author", "#Comments", "Status"]
        .iter()
        .map(|h| Cell::from(*h).style(header_style));

    let header = Row::new(header_cells)
        .style(Style::default().bg(app.store.state().theme.table_header_bg))
        .height(1);

    let selected_row_style = Style::default().add_modifier(Modifier::REVERSED).fg(app
        .store
        .state()
        .repos
        .colors
        .selected_row_style_fg);

    // Check if we should show a message instead of PRs
    if repo_data.prs.is_empty() {
        let message = match &repo_data.loading_state {
            LoadingState::Loading => "Loading pull requests...",
            LoadingState::Error(_err) => "Error loading data. Press Ctrl+r to retry.",
            _ => "No pull requests found matching filter",
        };

        let paragraph = Paragraph::new(message)
            .block(block)
            .style(Style::default().fg(app.store.state().repos.colors.row_fg))
            .alignment(ratatui::layout::Alignment::Center);

        f.render_widget(paragraph, table_area);
    } else {
        let rows = repo_data.prs.iter().enumerate().map(|(i, item)| {
            let color = match i % 2 {
                0 => app.store.state().repos.colors.normal_row_color,
                _ => app.store.state().repos.colors.alt_row_color,
            };
            let color = if repo_data.selected_prs.contains(&i) {
                app.store.state().repos.colors.selected_cell_style_fg
            } else {
                color
            };
            let row: Row = item.into();
            row.style(
                Style::new()
                    .fg(app.store.state().repos.colors.row_fg)
                    .bg(color),
            )
            .height(1)
        });

        let widths = [
            Constraint::Percentage(8),  // #PR
            Constraint::Percentage(50), // Description
            Constraint::Percentage(15), // Author
            Constraint::Percentage(10), // #Comments
            Constraint::Percentage(17), // Status (wider to show "✗ Build Failed" etc.)
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(block)
            .row_highlight_style(selected_row_style);

        // Get mutable reference to the current repo's table state
        let table_state = &mut app.get_current_repo_data_mut().table_state;
        f.render_stateful_widget(table, table_area, table_state);
    }

    // Render context-sensitive action panel at the bottom
    render_action_panel(f, app, chunks[2]);

    // Render status line at the very bottom
    render_status_line(f, app, chunks[3]);

    // Render log panel LAST if it's open - covers only the table area
    if let Some(ref panel) = app.store.state().log_panel.panel {
        crate::log::render_log_panel_card(f, panel, &app.store.state().theme, chunks[1]);
    }

    // Render shortcuts panel on top of everything if visible
    if app.store.state().ui.show_shortcuts {
        let max_scroll = crate::shortcuts::render_shortcuts_panel(
            f,
            chunks[1],
            app.store.state().ui.shortcuts_scroll,
            &app.store.state().theme,
        );
        app.store.state_mut().ui.shortcuts_max_scroll = max_scroll;
    }
}

/// Render the bottom action panel with context-sensitive shortcuts
fn render_action_panel(f: &mut Frame, app: &App, area: Rect) {
    let repo_data = app.get_current_repo_data();
    let selected_count = repo_data.selected_prs.len();

    let mut actions: Vec<(String, String, Color)> = Vec::new();

    // If log panel is open, show log panel shortcuts
    if app.store.state().log_panel.panel.is_some() {
        actions.push((
            "↑↓/jk".to_string(),
            "Scroll V".to_string(),
            tailwind::CYAN.c600,
        ));
        actions.push((
            "←→/h".to_string(),
            "Scroll H".to_string(),
            tailwind::CYAN.c600,
        ));
        actions.push((
            "n".to_string(),
            "Next Section".to_string(),
            tailwind::CYAN.c600,
        ));
        actions.push((
            "t".to_string(),
            if app
                .store
                .state()
                .log_panel
                .panel
                .as_ref()
                .map(|p| p.show_timestamps)
                .unwrap_or(false)
            {
                "Hide Timestamps".to_string()
            } else {
                "Show Timestamps".to_string()
            },
            tailwind::CYAN.c600,
        ));
        actions.push(("x/Esc".to_string(), "Close".to_string(), tailwind::RED.c600));
    } else if selected_count > 0 {
        // Highlight merge action when PRs are selected
        actions.push((
            "m".to_string(),
            format!("Merge ({})", selected_count),
            tailwind::GREEN.c700,
        ));

        // Show rebase action for manually selected PRs
        actions.push((
            "r".to_string(),
            format!("Rebase ({})", selected_count),
            tailwind::BLUE.c700,
        ));
    } else if !repo_data.prs.is_empty() {
        // When nothing selected, show how to select
        actions.push((
            "Space".to_string(),
            "Select".to_string(),
            tailwind::AMBER.c600,
        ));

        // Check if there are PRs that need rebase - show auto-rebase option
        let prs_needing_rebase = repo_data.prs.iter().filter(|pr| pr.needs_rebase).count();
        if prs_needing_rebase > 0 {
            actions.push((
                "r".to_string(),
                format!("Auto-rebase ({})", prs_needing_rebase),
                tailwind::YELLOW.c600,
            ));
        }
    }

    // Add Enter action when PR(s) are selected or focused
    if !repo_data.prs.is_empty() {
        if selected_count > 0 {
            actions.push((
                "Enter".to_string(),
                format!("Open in Browser ({})", selected_count),
                tailwind::PURPLE.c600,
            ));
        } else if let Some(selected_idx) = repo_data.table_state.selected() {
            actions.push((
                "Enter".to_string(),
                "Open in Browser".to_string(),
                tailwind::PURPLE.c600,
            ));

            // Add "l" action for viewing build logs
            if repo_data.prs.get(selected_idx).is_some() {
                actions.push((
                    "l".to_string(),
                    "View Build Logs".to_string(),
                    tailwind::ORANGE.c600,
                ));
                actions.push((
                    "i".to_string(),
                    "Open in IDE".to_string(),
                    tailwind::INDIGO.c600,
                ));
            }
        }
    }

    // Always add help shortcut at the end
    actions.push(("?".to_string(), "Help".to_string(), tailwind::SLATE.c600));

    // Helper function to create action spans
    let create_action_spans = |actions: &[(String, String, Color)]| -> Vec<Span> {
        let mut spans = Vec::new();
        for (i, (key, label, bg_color)) in actions.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }

            // Key part (highlighted)
            spans.push(Span::styled(
                format!(" {} ", key),
                Style::default()
                    .fg(app.store.state().theme.selected_fg)
                    .bg(*bg_color)
                    .add_modifier(Modifier::BOLD),
            ));

            // Label part
            spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(app.store.state().repos.colors.row_fg),
            ));
        }
        spans
    };

    // Render actions in full-width panel
    let action_spans = create_action_spans(&actions);
    let action_line = Line::from(action_spans);
    let action_paragraph = Paragraph::new(action_line)
        .block(Block::default().borders(Borders::ALL).title("Actions"))
        .alignment(ratatui::layout::Alignment::Left);
    f.render_widget(action_paragraph, area);
}

/// Render the status line showing background task progress
fn render_status_line(f: &mut Frame, app: &App, area: Rect) {
    if let Some(ref status) = app.store.state().task.status {
        let (icon, color) = match status.status_type {
            TaskStatusType::Running => ("⏳", app.store.state().theme.status_warning),
            TaskStatusType::Success => ("✓", app.store.state().theme.status_success),
            TaskStatusType::Error => ("✗", app.store.state().theme.status_error),
        };

        let status_text = format!(" {} {}", icon, status.message);
        let status_span = Span::styled(
            status_text,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        );

        let paragraph = Paragraph::new(Line::from(status_span))
            .style(Style::default().bg(app.store.state().repos.colors.buffer_bg));
        f.render_widget(paragraph, area);
    }
}

/// Render the fancy bootstrap loading screen
fn render_bootstrap_screen(f: &mut Frame, app: &App) {
    const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    let area = f.area();

    // Calculate a centered area for the bootstrap content
    let centered_area = {
        let width = 50.min(area.width);
        let height = 12.min(area.height);
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;
        Rect {
            x,
            y,
            width,
            height,
        }
    };

    // Clear background
    f.render_widget(
        Block::default().style(Style::default().bg(tailwind::SLATE.c950)),
        area,
    );

    // Render the centered content box
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(tailwind::BLUE.c500))
        .style(Style::default().bg(tailwind::SLATE.c900));

    f.render_widget(block, centered_area);

    // Split the centered area into sections for content
    let inner = centered_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title
            Constraint::Length(1), // Title underline
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // Spinner
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // Progress bar
            Constraint::Length(1), // Spacing
            Constraint::Min(2),    // Status message
        ])
        .split(inner);

    // Determine stage info
    let (stage_message, progress, is_error) = match &app.store.state().repos.bootstrap_state {
        BootstrapState::NotStarted => ("Initializing application...", 0, false),
        BootstrapState::LoadingRepositories => ("Loading repositories...", 25, false),
        BootstrapState::RestoringSession => ("Restoring session...", 50, false),
        BootstrapState::LoadingPRs => {
            // Calculate progress based on loaded repos
            let total_repos = app.store.state().repos.recent_repos.len().max(1);
            let loaded_repos = app
                .store
                .state()
                .repos
                .repo_data
                .values()
                .filter(|d| {
                    matches!(
                        d.loading_state,
                        LoadingState::Loaded | LoadingState::Error(_)
                    )
                })
                .count();
            let progress = 50 + ((loaded_repos * 50) / total_repos);
            (
                &format!(
                    "Loading pull requests from\n{} repositories...",
                    total_repos
                )[..],
                progress,
                false,
            )
        }
        BootstrapState::Error(err) => (&format!("Error: {}", err)[..], 0, true),
        BootstrapState::Completed => unreachable!(),
    };

    // Title
    let title = Paragraph::new("PR Bulk Review TUI")
        .style(
            Style::default()
                .fg(tailwind::BLUE.c400)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(title, chunks[0]);

    // Title underline
    let underline = Paragraph::new("─────────────────")
        .style(Style::default().fg(tailwind::BLUE.c600))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(underline, chunks[1]);

    // Spinner (animated)
    if !is_error {
        let spinner = SPINNER_FRAMES[app.store.state().ui.spinner_frame % SPINNER_FRAMES.len()];
        let spinner_text = format!("{} Loading...", spinner);
        let spinner_widget = Paragraph::new(spinner_text)
            .style(
                Style::default()
                    .fg(app.store.state().theme.status_warning)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(spinner_widget, chunks[3]);
    } else {
        let error_icon = Paragraph::new("✗ Error")
            .style(
                Style::default()
                    .fg(app.store.state().theme.status_error)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(error_icon, chunks[3]);
    }

    // Progress bar
    if !is_error {
        let bar_width = chunks[5].width.saturating_sub(10) as usize; // Reserve space for percentage
        let filled = (bar_width * progress) / 100;
        let empty = bar_width.saturating_sub(filled);

        let progress_bar = format!("{}{}  {}%", "▰".repeat(filled), "▱".repeat(empty), progress);

        let bar_widget = Paragraph::new(progress_bar)
            .style(Style::default().fg(app.store.state().theme.status_info))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(bar_widget, chunks[5]);
    }

    // Status message
    let message_style = if is_error {
        Style::default().fg(app.store.state().theme.status_error)
    } else {
        Style::default().fg(app.store.state().theme.text_secondary)
    };

    let message_widget = Paragraph::new(stage_message)
        .style(message_style)
        .alignment(ratatui::layout::Alignment::Center)
        .wrap(Wrap { trim: true });
    f.render_widget(message_widget, chunks[7]);
}

#[tokio::main]
async fn main() -> Result<()> {
    initialize_panic_handler();
    startup()?;
    run().await?;
    shutdown()?;
    Ok(())
}

impl App {
    fn new(
        action_tx: mpsc::UnboundedSender<Action>,
        task_tx: mpsc::UnboundedSender<BackgroundTask>,
    ) -> App {
        // Initialize Redux store with default state
        let theme = Theme::default();
        let initial_state = AppState {
            ui: UiState::default(),
            repos: ReposState {
                colors: TableColors::from_theme(&theme),
                ..ReposState::default()
            },
            log_panel: LogPanelState::default(),
            merge_bot: MergeBotState::default(),
            task: TaskState::default(),
            config: Config::load(),
            theme,
        };

        App {
            store: Store::new(initial_state),
            action_tx,
            task_tx,
            octocrab: None, // Initialized lazily during bootstrap after .env is loaded
        }
    }

    /// Get the current repo data (read-only)
    fn get_current_repo_data(&self) -> RepoData {
        self.store
            .state()
            .repos
            .repo_data
            .get(&self.store.state().repos.selected_repo)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the current repo data (mutable)
    fn get_current_repo_data_mut(&mut self) -> &mut RepoData {
        let selected_repo = self.store.state().repos.selected_repo;
        self.store
            .state_mut()
            .repos
            .repo_data
            .entry(selected_repo)
            .or_default()
    }

    /// Save current state to the repo data before switching tabs
    fn save_current_repo_state(&mut self) {
        let selected_repo = self.store.state().repos.selected_repo;
        let prs = self.store.state().repos.prs.clone();
        let table_state = self.store.state().repos.state.clone();
        let selected_prs = self.store.state().repos.selected_prs.clone();
        let loading_state = self.store.state().repos.loading_state.clone();

        let data = self
            .store
            .state_mut()
            .repos
            .repo_data
            .entry(selected_repo)
            .or_default();
        data.prs = prs;
        data.table_state = table_state;
        data.selected_prs = selected_prs;
        data.loading_state = loading_state;
    }

    /// Load state from repo data when switching tabs
    fn load_repo_state(&mut self) {
        let data = self.get_current_repo_data();
        self.store.state_mut().repos.prs = data.prs;
        self.store.state_mut().repos.state = data.table_state;
        self.store.state_mut().repos.selected_prs = data.selected_prs;
        self.store.state_mut().repos.loading_state = data.loading_state;
    }

    fn octocrab(&self) -> Result<Octocrab> {
        // Return cached octocrab instance (initialized during bootstrap)
        self.octocrab.clone().ok_or_else(|| {
            anyhow::anyhow!("Octocrab not initialized. This is a bug - octocrab should be initialized during bootstrap.")
        })
    }

    fn repo(&self) -> Option<&Repo> {
        self.store
            .state()
            .repos
            .recent_repos
            .get(self.store.state().repos.selected_repo)
    }

    async fn restore_session(&mut self, state: PersistedState) -> Result<()> {
        // Restore the selected repository from the persisted state
        if let Some(index) = self
            .store
            .state()
            .repos
            .recent_repos
            .iter()
            .position(|r| r == &state.selected_repo)
        {
            self.store.state_mut().repos.selected_repo = index;
        } else {
            // If the persisted repo is not found, default to first repo
            debug!("Persisted repository not found in recent repositories, defaulting to first");
            self.store.state_mut().repos.selected_repo = 0;
        }

        Ok(())
    }

    /// Fetch data from GitHub for the selected repository and filter
    async fn fetch_data(&mut self, repo: &Repo) -> Result<()> {
        self.store.state_mut().repos.loading_state = LoadingState::Loading;

        let octocrab = self.octocrab()?.clone();
        let repo = repo.clone();
        let filter = self.store.state().repos.filter.clone();

        let github_data =
            tokio::task::spawn(async move { fetch_github_data(&octocrab, &repo, &filter).await })
                .await??;
        self.store.state_mut().repos.prs = github_data;

        self.store.state_mut().repos.loading_state = LoadingState::Loaded;

        Ok(())
    }

    /// Move to the next PR in the list
    fn next(&mut self) {
        let repo_data = self.get_current_repo_data();
        let i = match repo_data.table_state.selected() {
            Some(i) => {
                if i < repo_data.prs.len() - 1 {
                    i + 1
                } else {
                    i
                }
            }
            None => 0,
        };

        // Update both the repo data state and the app state
        self.store.state_mut().repos.state.select(Some(i));
        let selected_repo = self.store.state().repos.selected_repo;
        let data = self
            .store
            .state_mut()
            .repos
            .repo_data
            .entry(selected_repo)
            .or_default();
        data.table_state.select(Some(i));
    }

    /// Move to the previous PR in the list
    fn previous(&mut self) {
        let repo_data = self.get_current_repo_data();
        let i = match repo_data.table_state.selected() {
            Some(i) => {
                if i > 0 {
                    i - 1
                } else {
                    i
                }
            }
            None => 0,
        };

        // Update both the repo data state and the app state
        self.store.state_mut().repos.state.select(Some(i));
        let selected_repo = self.store.state().repos.selected_repo;
        let data = self
            .store
            .state_mut()
            .repos
            .repo_data
            .entry(selected_repo)
            .or_default();
        data.table_state.select(Some(i));
    }

    /// Toggle the selection of the currently selected PR
    fn select_toggle(&mut self) {
        let repo_data = self.get_current_repo_data();
        let i = repo_data.table_state.selected().unwrap_or(0);

        // Update both the app state and repo data
        if self.store.state().repos.selected_prs.contains(&i) {
            self.store
                .state_mut()
                .repos
                .selected_prs
                .retain(|&x| x != i);
        } else {
            self.store.state_mut().repos.selected_prs.push(i);
        }

        let selected_repo = self.store.state().repos.selected_repo;
        let data = self
            .store
            .state_mut()
            .repos
            .repo_data
            .entry(selected_repo)
            .or_default();
        if data.selected_prs.contains(&i) {
            data.selected_prs.retain(|&x| x != i);
        } else {
            data.selected_prs.push(i);
        }
    }

    /// Select the next repo (cycle forward through tabs)
    fn select_next_repo(&mut self) {
        self.save_current_repo_state();
        self.store.state_mut().repos.selected_repo = (self.store.state().repos.selected_repo + 1)
            % self.store.state().repos.recent_repos.len();
        self.load_repo_state();
    }

    /// Select the previous repo (cycle backward through tabs)
    fn select_previous_repo(&mut self) {
        self.save_current_repo_state();
        self.store.state_mut().repos.selected_repo =
            if self.store.state_mut().repos.selected_repo == 0 {
                self.store.state().repos.recent_repos.len() - 1
            } else {
                self.store.state().repos.selected_repo - 1
            };
        self.load_repo_state();
    }

    /// Select a repo by index (for number key shortcuts)
    fn select_repo_by_index(&mut self, index: usize) {
        if index < self.store.state().repos.recent_repos.len() {
            self.save_current_repo_state();
            self.store.state_mut().repos.selected_repo = index;
            self.load_repo_state();
        }
    }

    /// Load data for all repositories in parallel on startup
    async fn load_all_repos(&mut self) -> Result<()> {
        let octocrab = self.octocrab()?;
        let filter = self.store.state().repos.filter.clone();
        let repos = self.store.state().repos.recent_repos.clone();

        // Set all repos to loading state
        for i in 0..repos.len() {
            let data = self.store.state_mut().repos.repo_data.entry(i).or_default();
            data.loading_state = LoadingState::Loading;
        }

        // Spawn tasks to fetch data for each repo in parallel
        let mut tasks = Vec::new();
        for (index, repo) in repos.iter().enumerate() {
            let octocrab = octocrab.clone();
            let repo = repo.clone();
            let filter = filter.clone();

            let task = tokio::spawn(async move {
                let result = fetch_github_data(&octocrab, &repo, &filter).await;
                (index, result)
            });
            tasks.push(task);
        }

        // Collect results as they complete
        for task in tasks {
            if let Ok((index, result)) = task.await {
                let data = self
                    .store
                    .state_mut()
                    .repos
                    .repo_data
                    .entry(index)
                    .or_default();
                match result {
                    Ok(prs) => {
                        data.prs = prs;
                        data.loading_state = LoadingState::Loaded;
                        if data.table_state.selected().is_none() && !data.prs.is_empty() {
                            data.table_state.select(Some(0));
                        }
                    }
                    Err(err) => {
                        data.loading_state = LoadingState::Error(err.to_string());
                    }
                }
            }
        }

        // Load the current repo state into the app
        self.load_repo_state();

        Ok(())
    }

    /// Refresh the current repository's data
    async fn refresh_current_repo(&mut self) -> Result<()> {
        let Some(repo) = self.repo().cloned() else {
            bail!("No repository selected");
        };

        // Set to loading state
        let selected_repo = self.store.state().repos.selected_repo;
        self.store.state_mut().repos.loading_state = LoadingState::Loading;
        let data = self
            .store
            .state_mut()
            .repos
            .repo_data
            .entry(selected_repo)
            .or_default();
        data.loading_state = LoadingState::Loading;

        self.fetch_data(&repo).await?;

        // Update the repo data cache
        let prs = self.store.state().repos.prs.clone();
        let loading_state = self.store.state().repos.loading_state.clone();
        let data = self
            .store
            .state_mut()
            .repos
            .repo_data
            .entry(selected_repo)
            .or_default();
        data.prs = prs;
        data.loading_state = loading_state;

        Ok(())
    }

    async fn select_repo(&mut self) -> Result<()> {
        let Some(repo) = self.repo().cloned() else {
            bail!("No repository selected");
        };
        debug!("Selecting repo: {:?}", repo);

        // This function is a placeholder for future implementation
        // It could be used to select a specific repo from a list or input
        self.store.state_mut().repos.selected_prs.clear();
        self.fetch_data(&repo).await?;
        self.store.state_mut().repos.state.select(Some(0));
        Ok(())
    }

    /// Exit the application
    fn exit(&mut self) -> Result<()> {
        bail!("Exiting the application")
    }

    /// Rebase the selected PRs
    async fn rebase(&mut self) -> Result<()> {
        // for all selected PRs, authored by `dependabot` we rebase by adding the commend `@dependabot rebase`

        let Some(repo) = self.repo() else {
            bail!("No repository selected for rebasing");
        };
        let octocrab = self.octocrab()?;
        for &pr_index in &self.store.state().repos.selected_prs {
            if let Some(pr) = self.store.state().repos.prs.get(pr_index) {
                if pr.author.starts_with("dependabot") {
                    debug!("Rebasing PR #{}", pr.number);

                    comment(&octocrab, repo, pr, "@dependabot rebase").await?;
                } else {
                    debug!("Skipping PR #{} authored by {}", pr.number, pr.author);
                }
            } else {
                debug!("No PR found at index {}", pr_index);
            }
        }
        debug!(
            "Rebasing selected PRs: {:?}",
            self.store.state().repos.selected_prs
        );

        Ok(())
    }

    /// Merge the selected PRs
    async fn merge(&mut self) -> Result<()> {
        let Some(repo) = self.repo() else {
            bail!("No repository selected for merging");
        };
        let octocrab = self.octocrab()?;
        let mut selected_prs = self.store.state().repos.selected_prs.clone();
        for &pr_index in self.store.state().repos.selected_prs.iter() {
            if let Some(pr) = self.store.state().repos.prs.get(pr_index) {
                debug!("Merging PR #{}", pr.number);
                match merge(&octocrab, repo, pr).await {
                    Ok(_) => {
                        debug!("Successfully merged PR #{}", pr.number);
                        selected_prs.retain(|&x| x != pr_index);
                    }
                    Err(err) => {
                        debug!("Failed to merge PR #{}: {}", pr.number, err);
                    }
                }
            } else {
                debug!("No PR found at index {}", pr_index);
            }
        }

        // todo: now clean up `self.store.state().repos.prs` by those that are not in `selected_prs` anymore,
        // there the index of the PRs is to take

        self.store.state_mut().repos.selected_prs = selected_prs;
        debug!(
            "Merging selected PRs: {:?}",
            self.store.state().repos.selected_prs
        );

        Ok(())
    }
}

pub async fn fetch_github_data<'a>(
    octocrab: &Octocrab,
    repo: &Repo,
    filter: &PrFilter,
) -> Result<Vec<Pr>> {
    let mut prs = Vec::new();
    let mut page_num = 1u32;
    const MAX_PRS: usize = 50;
    const PER_PAGE: u8 = 30;

    // Fetch pages until we have 50 PRs or run out of pages
    loop {
        let page = octocrab
            .pulls(&repo.org, &repo.repo)
            .list()
            .state(params::State::Open)
            .head(&repo.branch)
            .sort(params::pulls::Sort::Updated)
            .direction(params::Direction::Ascending)
            .per_page(PER_PAGE)
            .page(page_num)
            .send()
            .await?;

        let page_is_empty = page.items.is_empty();

        for pr in page.items.into_iter().filter(|pr| {
            pr.title
                .as_ref()
                .map(|t| filter.matches(t))
                .unwrap_or(false)
        }) {
            if prs.len() >= MAX_PRS {
                break;
            }
            let pr = Pr::from_pull_request(&pr, repo, &octocrab).await;
            prs.push(pr);
        }

        // Stop if we have enough PRs or if the page was empty
        if prs.len() >= MAX_PRS || page_is_empty {
            break;
        }

        page_num += 1;
    }

    Ok(prs)
}

fn handle_events() -> Result<Action> {
    Ok(match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key_event(key),
        _ => Action::None,
    })
}

fn handle_key_event(key: KeyEvent) -> Action {
    // Use the shortcuts module to find the action for this key
    crate::shortcuts::find_action_for_key(&key)
}

/// loading recent repositories from a local config file, that is just json file
fn loading_recent_repos() -> Result<Vec<Repo>> {
    let repos = if let Ok(recent_repos) = File::open(".recent-repositories.json") {
        let reader = BufReader::new(recent_repos);
        serde_json::from_reader(reader).context("Failed to parse recent repositories from file")?
    } else {
        debug!("No recent repositories file found, using default repositories");
        vec![
            Repo::new("cargo-generate", "cargo-generate", "main"),
            Repo::new("steganogram", "stegano-rs", "main"),
            Repo::new("rust-lang", "rust", "master"),
        ]
    };

    debug!("Loaded recent repositories: {:?}", repos);

    Ok(repos)
}

/// Storing recent repositories to a local json config file
fn store_recent_repos(repos: &[Repo]) -> Result<()> {
    let file = File::create(".recent-repositories.json")
        .context("Failed to create recent repositories file")?;
    serde_json::to_writer_pretty(file, &repos)
        .context("Failed to write recent repositories to file")?;

    debug!("Stored recent repositories: {:?}", repos);

    Ok(())
}

fn store_persisted_state(state: &PersistedState) -> Result<()> {
    let file = File::create(".session.json").context("Failed to create persisted state file")?;
    serde_json::to_writer_pretty(file, state).context("Failed to write persisted state to file")?;

    debug!("Stored persisted state: {:?}", state);

    Ok(())
}

fn load_persisted_state() -> Result<PersistedState> {
    let file = File::open(".session.json").context("Failed to open persisted state file")?;
    let reader = BufReader::new(file);
    let state: PersistedState =
        serde_json::from_reader(reader).context("Failed to parse persisted state from file")?;

    debug!("Loaded persisted state: {:?}", state);

    Ok(state)
}
