use anyhow::{Context, Result};
use octocrab::{Octocrab, params};
use ratatui::{
    crossterm::{
        self,
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    },
    layout::Margin,
    prelude::*,
    style::palette::tailwind,
    widgets::*,
};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::BufReader, sync::{Arc, Mutex}};
use tokio::sync::mpsc;

// Import debug from the log crate using :: prefix to disambiguate from our log module
use ::log::debug;

use crate::config::Config;
use crate::effect::execute_effect;
use crate::pr::Pr;
use crate::shortcuts::Action;
use crate::state::*;
use crate::store::Store;
use crate::task::{BackgroundTask, TaskResult, start_task_worker};
use crate::theme::Theme;

mod config;
mod effect;
mod gh;
mod log;
mod log_capture;
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

async fn update(app: &mut App, msg: Action) -> Result<Action> {
    // When add repo popup is open, handle popup-specific actions
    let msg = if app.store.state().ui.show_add_repo {
        match msg {
            // Allow these actions in the popup
            Action::HideAddRepoPopup
            | Action::AddRepoFormInput(_)
            | Action::AddRepoFormBackspace
            | Action::AddRepoFormNextField
            | Action::AddRepoFormSubmit => msg,
            Action::Quit => Action::HideAddRepoPopup,
            _ => {
                // Ignore all other actions when popup is open
                return Ok(Action::None);
            }
        }
    } else if app.store.state().ui.show_shortcuts {
        // When shortcuts panel is open, remap navigation to shortcuts scrolling
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

    // Execute effects returned by reducers and dispatch follow-up actions
    for effect in effects {
        let follow_up_actions = execute_effect(app, effect).await?;

        // Dispatch all follow-up actions returned from the effect
        for action in follow_up_actions {
            // Recursively process each follow-up action
            // This creates a chain: Action → Effects → Follow-up Actions → More Effects...
            let nested_effects = app.store.dispatch(action);
            for nested_effect in nested_effects {
                let nested_actions = execute_effect(app, nested_effect).await?;
                // Continue dispatching nested actions
                for nested_action in nested_actions {
                    let _ = app.action_tx.send(nested_action);
                }
            }
        }
    }

    Ok(Action::None)
}

fn start_event_handler(
    app: &App,
    tx: mpsc::UnboundedSender<Action>,
) -> (tokio::task::JoinHandle<()>, Arc<Mutex<bool>>) {
    let tick_rate = std::time::Duration::from_millis(250);
    // Clone the shared popup state flag for the event loop
    let show_add_repo_shared = app.store.state().ui.show_add_repo_shared.clone();
    // Clone the pending key state for two-key combinations
    let pending_key_shared = app.store.state().ui.pending_key.clone();
    // Clone the shared log panel state for the event loop
    let log_panel_open_shared = app.store.state().log_panel.log_panel_open_shared.clone();
    let log_panel_open = log_panel_open_shared.clone();
    let job_list_focused_shared = app.store.state().log_panel.job_list_focused_shared.clone();
    let job_list_focused = job_list_focused_shared.clone();
    // Create shared debug console state for event loop
    let debug_console_open_shared = Arc::new(Mutex::new(false));
    let debug_console_open = debug_console_open_shared.clone();

    let handle = tokio::spawn(async move {
        loop {
            let action = if crossterm::event::poll(tick_rate).unwrap() {
                let show_add_repo = *show_add_repo_shared.lock().unwrap();
                let log_panel_open_val = *log_panel_open.lock().unwrap();
                let job_list_focused_val = *job_list_focused.lock().unwrap();
                let console_open = *debug_console_open.lock().unwrap();
                handle_events(show_add_repo, log_panel_open_val, job_list_focused_val, console_open, &pending_key_shared).unwrap_or(Action::None)
            } else {
                Action::None
            };

            if let Err(_) = tx.send(action) {
                break;
            }
        }
    });

    (handle, debug_console_open_shared)
}

/// Convert TaskResult to Action - the single place where task results become actions
fn result_to_action(result: TaskResult) -> Action {
    match result {
        TaskResult::RepoLoadingStarted(idx) => Action::RepoLoadingStarted(idx),
        TaskResult::RepoDataLoaded(idx, data) => Action::RepoDataLoaded(idx, data),
        TaskResult::MergeStatusUpdated(idx, pr_num, status) => {
            Action::MergeStatusUpdated(idx, pr_num, status)
        }
        TaskResult::RebaseStatusUpdated(idx, pr_num, needs_rebase) => {
            Action::RebaseStatusUpdated(idx, pr_num, needs_rebase)
        }
        TaskResult::CommentCountUpdated(idx, pr_num, count) => {
            Action::CommentCountUpdated(idx, pr_num, count)
        }
        TaskResult::RebaseComplete(res) => Action::RebaseComplete(res),
        TaskResult::MergeComplete(res) => Action::MergeComplete(res),
        TaskResult::RerunJobsComplete(res) => Action::RerunJobsComplete(res),
        TaskResult::ApprovalComplete(res) => Action::ApprovalComplete(res),
        TaskResult::BuildLogsLoaded(sections, ctx) => Action::BuildLogsLoaded(sections, ctx),
        TaskResult::IDEOpenComplete(res) => Action::IDEOpenComplete(res),
        TaskResult::PRMergedConfirmed(idx, pr_num, merged) => {
            Action::PRMergedConfirmed(idx, pr_num, merged)
        }
        TaskResult::TaskStatusUpdate(status) => Action::SetTaskStatus(status),
        TaskResult::AutoMergeStatusCheck(idx, pr_num) => Action::AutoMergeStatusCheck(idx, pr_num),
        TaskResult::RemoveFromAutoMergeQueue(idx, pr_num) => {
            Action::RemoveFromAutoMergeQueue(idx, pr_num)
        }
        TaskResult::OperationMonitorCheck(idx, pr_num) => {
            Action::OperationMonitorCheck(idx, pr_num)
        }
        TaskResult::RemoveFromOperationMonitor(idx, pr_num) => {
            Action::RemoveFromOperationMonitor(idx, pr_num)
        }
        TaskResult::RepoNeedsReload(idx) => {
            Action::ReloadRepo(idx)
        }
    }
}

async fn run_with_log_buffer(log_buffer: log_capture::LogBuffer) -> Result<()> {
    let mut t = Terminal::new(CrosstermBackend::new(std::io::stderr()))?;

    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    let (task_tx, task_rx) = mpsc::unbounded_channel();
    let (result_tx, mut result_rx) = mpsc::unbounded_channel(); // New result channel

    let mut app = App::new(action_tx.clone(), task_tx, log_buffer);

    let (event_task, debug_console_shared) = start_event_handler(&app, app.action_tx.clone());
    let worker_task = start_task_worker(task_rx, result_tx);

    app.action_tx
        .send(Action::Bootstrap)
        .expect("Failed to send bootstrap action");

    loop {
        // Sync the shared popup state for event handler
        *app.store.state().ui.show_add_repo_shared.lock().unwrap() =
            app.store.state().ui.show_add_repo;

        // Sync the shared debug console state for event handler
        *debug_console_shared.lock().unwrap() =
            app.store.state().debug_console.is_open;

        // Sync the shared log panel state for event handler
        *app.store.state().log_panel.log_panel_open_shared.lock().unwrap() =
            app.store.state().log_panel.panel.is_some();

        t.draw(|f| {
            ui(f, &mut app);
        })?;

        // Use tokio::select! to handle both actions and task results
        // Prioritize results over actions to show incremental progress
        let maybe_action = tokio::time::timeout(std::time::Duration::from_millis(100), async {
            tokio::select! {
                biased;  // Check in order: results first, then actions
                Some(result) = result_rx.recv() => {
                    // Convert task result to action (prioritized for smooth progress updates)
                    Some(result_to_action(result))
                }
                Some(action) = action_rx.recv() => Some(action),
                else => None
            }
        })
        .await;

        match maybe_action {
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
                // Timeout - tick spinner animation (maintains clean architecture without blocking progress)
                let _ = app.action_tx.send(Action::TickSpinner);
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

    // Active/focused row style - use theme colors instead of REVERSED modifier
    // to avoid text becoming invisible when row is both selected and focused
    let selected_row_style = Style::default()
        .bg(app.store.state().theme.active_bg)
        .fg(app.store.state().theme.active_fg);

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
            // Use theme color for selected rows (Space key)
            // Now using type-safe PR numbers for stable selection across filtering/reloading
            let color = if repo_data.selected_pr_numbers.contains(&PrNumber::from_pr(item)) {
                app.store.state().theme.selected_bg
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
        let viewport_height = crate::log::render_log_panel_card(f, panel, &app.store.state().theme, chunks[1]);
        // Update viewport height for page down scrolling
        app.store.dispatch(Action::UpdateLogPanelViewport(viewport_height));
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

    // Render add repo popup on top of everything if visible
    if app.store.state().ui.show_add_repo {
        render_add_repo_popup(
            f,
            chunks[1],
            &app.store.state().ui.add_repo_form,
            &app.store.state().theme,
        );
    }

    // Render debug console (Quake-style drop-down) if visible
    if app.store.state().debug_console.is_open {
        let viewport_height = render_debug_console(f, f.area(), app);
        // Update viewport height for page down scrolling
        app.store.dispatch(Action::UpdateDebugConsoleViewport(viewport_height));
    }
}

/// Render the add repository popup as a centered floating window
fn render_add_repo_popup(f: &mut Frame, area: Rect, form: &AddRepoForm, theme: &Theme) {
    use ratatui::widgets::{Clear, Wrap};

    // Calculate centered area (60% width, 50% height)
    let popup_width = (area.width * 60 / 100).min(70);
    let popup_height = 14; // Fixed height for the form
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

    // Render border and title
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Add New Repository ")
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

    // Calculate inner area
    let inner = popup_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    // Build form content
    let mut text_lines = Vec::new();

    // Instructions
    text_lines.push(Line::from(vec![Span::styled(
        "Enter GitHub URL or fill in the fields manually:",
        Style::default().fg(theme.text_secondary),
    )]));
    text_lines.push(Line::from(""));

    // Organization field
    let org_focused = form.focused_field == AddRepoField::Org;
    text_lines.push(Line::from(vec![
        Span::styled(
            if org_focused { "> " } else { "  " },
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Organization: ",
            Style::default()
                .fg(if org_focused {
                    theme.active_fg
                } else {
                    theme.text_primary
                })
                .add_modifier(if org_focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            &form.org,
            Style::default()
                .fg(if org_focused {
                    theme.active_fg
                } else {
                    theme.text_primary
                })
                .bg(if org_focused {
                    theme.active_bg
                } else {
                    theme.bg_panel
                }),
        ),
    ]));

    // Repository field
    let repo_focused = form.focused_field == AddRepoField::Repo;
    text_lines.push(Line::from(vec![
        Span::styled(
            if repo_focused { "> " } else { "  " },
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Repository:   ",
            Style::default()
                .fg(if repo_focused {
                    theme.active_fg
                } else {
                    theme.text_primary
                })
                .add_modifier(if repo_focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            &form.repo,
            Style::default()
                .fg(if repo_focused {
                    theme.active_fg
                } else {
                    theme.text_primary
                })
                .bg(if repo_focused {
                    theme.active_bg
                } else {
                    theme.bg_panel
                }),
        ),
    ]));

    // Branch field
    let branch_focused = form.focused_field == AddRepoField::Branch;
    let branch_display = if form.branch.is_empty() {
        "main (default)"
    } else {
        &form.branch
    };
    text_lines.push(Line::from(vec![
        Span::styled(
            if branch_focused { "> " } else { "  " },
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Branch:       ",
            Style::default()
                .fg(if branch_focused {
                    theme.active_fg
                } else {
                    theme.text_primary
                })
                .add_modifier(if branch_focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            branch_display,
            Style::default()
                .fg(if branch_focused {
                    theme.active_fg
                } else {
                    theme.text_muted
                })
                .bg(if branch_focused {
                    theme.active_bg
                } else {
                    theme.bg_panel
                }),
        ),
    ]));

    text_lines.push(Line::from(""));
    text_lines.push(Line::from(""));

    // Footer with shortcuts
    text_lines.push(Line::from(vec![
        Span::styled(
            "Tab",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" navigate  ", Style::default().fg(theme.text_muted)),
        Span::styled(
            "Enter",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" add  ", Style::default().fg(theme.text_muted)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" cancel", Style::default().fg(theme.text_muted)),
    ]));

    // Render content
    let paragraph = Paragraph::new(text_lines)
        .wrap(Wrap { trim: false })
        .style(Style::default().bg(theme.bg_panel));

    f.render_widget(paragraph, inner);
}

/// Render the bottom action panel with context-sensitive shortcuts
fn render_action_panel(f: &mut Frame, app: &App, area: Rect) {
    let repo_data = app.get_current_repo_data();
    let selected_count = repo_data.selected_pr_numbers.len();

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

        // Show approval action for selected PRs
        actions.push((
            "a".to_string(),
            format!("Approve ({})", selected_count),
            tailwind::EMERALD.c600,
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
                actions.push((
                    "a".to_string(),
                    "Approve".to_string(),
                    tailwind::EMERALD.c600,
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
            TaskStatusType::Warning => ("⚠", app.store.state().theme.status_warning),
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
            // Calculate progress with half-credit for loading repos
            let total_repos = app.store.state().repos.recent_repos.len().max(1);

            let (loading_count, loaded_count) = app
                .store
                .state()
                .repos
                .repo_data
                .values()
                .fold((0, 0), |(loading, loaded), d| {
                    match d.loading_state {
                        LoadingState::Loading => (loading + 1, loaded),
                        LoadingState::Loaded | LoadingState::Error(_) => (loading, loaded + 1),
                        _ => (loading, loaded),
                    }
                });

            // Progress: loaded repos count as 1.0, loading repos count as 0.5
            // For 5 repos: start loading #1 = 10%, #1 done = 20%, start #2 = 30%, etc.
            let progress = ((loaded_count * 100) + (loading_count * 50)) / total_repos;

            (
                &format!(
                    "Loading pull requests...\n[{}/{}] repositories",
                    loaded_count,
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

/// Render the debug console as a Quake-style drop-down panel
/// Returns the visible viewport height for page down scrolling
fn render_debug_console(f: &mut Frame, area: Rect, app: &App) -> usize {
    use ratatui::widgets::{Clear, List, ListItem};

    let console_state = &app.store.state().debug_console;
    let theme = &app.store.state().theme;

    // Calculate console height based on percentage
    let console_height = (area.height * console_state.height_percent) / 100;
    let console_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: console_height.min(area.height),
    };

    // Clear the area
    f.render_widget(Clear, console_area);

    // Read logs from buffer
    let logs = console_state.logs.lock().unwrap();
    let log_count = logs.len();

    // Calculate visible range based on scroll offset
    let visible_height = console_height.saturating_sub(3) as usize; // Subtract border and header
    let total_logs = logs.len();

    let scroll_offset = if console_state.auto_scroll {
        // Auto-scroll: show most recent logs
        total_logs.saturating_sub(visible_height)
    } else {
        // Manual scroll: use scroll_offset
        console_state.scroll_offset.min(total_logs.saturating_sub(visible_height))
    };

    // Convert logs to ListItems with color coding
    let log_items: Vec<ListItem> = logs
        .iter()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|entry| {
            use ::log::Level;

            let level_color = match entry.level {
                Level::Error => theme.status_error,
                Level::Warn => theme.status_warning,
                Level::Info => theme.text_primary,
                Level::Debug => theme.text_secondary,
                Level::Trace => theme.text_muted,
            };

            let timestamp = entry.timestamp.format("%H:%M:%S%.3f");
            let level_str = format!("{:5}", entry.level.to_string().to_uppercase());
            let target_short = if entry.target.len() > 20 {
                format!("{}...", &entry.target[..17])
            } else {
                format!("{:20}", entry.target)
            };

            let text = format!(
                "{} {} {} {}",
                timestamp,
                level_str,
                target_short,
                entry.message
            );

            ListItem::new(text).style(Style::default().fg(level_color))
        })
        .collect();

    // Create the list widget
    let logs_list = List::new(log_items)
        .block(
            Block::bordered()
                .title(format!(
                    " Debug Console ({}/{}) {} ",
                    scroll_offset + visible_height.min(total_logs),
                    log_count,
                    if console_state.auto_scroll { "[AUTO]" } else { "[MANUAL]" }
                ))
                .title_bottom(" `~` Close | j/k Scroll | a Auto-scroll | c Clear ")
                .border_style(Style::default().fg(theme.accent_primary))
                .style(Style::default().bg(theme.bg_secondary))
        );

    f.render_widget(logs_list, console_area);

    visible_height
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize debug console logger before anything else
    let log_buffer = log_capture::init_logger();

    initialize_panic_handler();
    startup()?;
    run_with_log_buffer(log_buffer).await?;
    shutdown()?;
    Ok(())
}

impl App {
    fn new(
        action_tx: mpsc::UnboundedSender<Action>,
        task_tx: mpsc::UnboundedSender<BackgroundTask>,
        log_buffer: log_capture::LogBuffer,
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
            debug_console: DebugConsoleState {
                logs: log_buffer,
                ..DebugConsoleState::default()
            },
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

    // Sort by PR number (descending) for stable, predictable ordering
    prs.sort_by(|a, b| b.number.cmp(&a.number));

    Ok(prs)
}

fn handle_events(
    show_add_repo: bool,
    log_panel_open: bool,
    job_list_focused: bool,
    debug_console_open: bool,
    pending_key_shared: &std::sync::Arc<std::sync::Mutex<Option<crate::state::PendingKeyPress>>>,
) -> Result<Action> {
    Ok(match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            handle_key_event(key, show_add_repo, log_panel_open, job_list_focused, debug_console_open, pending_key_shared)
        }
        _ => Action::None,
    })
}

fn handle_key_event(
    key: KeyEvent,
    show_add_repo: bool,
    log_panel_open: bool,
    job_list_focused: bool,
    debug_console_open: bool,
    pending_key_shared: &std::sync::Arc<std::sync::Mutex<Option<crate::state::PendingKeyPress>>>,
) -> Action {
    // Handle add repo popup keys first if popup is open
    if show_add_repo {
        match key.code {
            KeyCode::Esc => return Action::HideAddRepoPopup,
            KeyCode::Enter => return Action::AddRepoFormSubmit,
            KeyCode::Tab => return Action::AddRepoFormNextField,
            KeyCode::Backspace => return Action::AddRepoFormBackspace,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::AddRepoFormInput(c);
            }
            _ => return Action::None,
        }
    }

    // Handle log panel keys if panel is open (before general shortcuts)
    if log_panel_open {
        match key.code {
            // Close panel (x or Esc)
            KeyCode::Char('x') | KeyCode::Esc => {
                return Action::CloseLogPanel;
            }
            // Tab: Switch focus between job list and log viewer
            KeyCode::Tab => {
                return if job_list_focused {
                    Action::FocusLogViewer
                } else {
                    Action::FocusJobList
                };
            }
            // j/k: Tree navigation (move through visible nodes)
            KeyCode::Char('j') | KeyCode::Down => {
                return Action::SelectNextJob; // Navigates down in tree
            }
            KeyCode::Char('k') | KeyCode::Up => {
                return Action::SelectPrevJob; // Navigates up in tree
            }
            // Page down (space)
            KeyCode::Char(' ') => {
                return Action::PageLogPanelDown;
            }
            // Horizontal scrolling (h/l or left/right)
            KeyCode::Char('h') | KeyCode::Left => {
                return Action::ScrollLogPanelLeft;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                return Action::ScrollLogPanelRight;
            }
            // Error navigation (n/p) - jump to next/previous error
            KeyCode::Char('n') => {
                return Action::NextError;
            }
            KeyCode::Char('p') => {
                return Action::PrevError;
            }
            // Toggle timestamps
            KeyCode::Char('t') => {
                return Action::ToggleTimestamps;
            }
            // Enter: Toggle tree node expand/collapse
            KeyCode::Enter => {
                return Action::ToggleTreeNode;
            }
            _ => return Action::None,
        }
    }

    // Handle debug console keys if console is open (before general shortcuts)
    if debug_console_open {
        match key.code {
            // Toggle console (backtick/tilde or Esc to close)
            KeyCode::Char('`') | KeyCode::Char('~') | KeyCode::Esc => {
                return Action::ToggleDebugConsole;
            }
            // Scroll up/down
            KeyCode::Char('j') | KeyCode::Down => {
                return Action::ScrollDebugConsoleDown;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                return Action::ScrollDebugConsoleUp;
            }
            // Page down (space)
            KeyCode::Char(' ') => {
                return Action::PageDebugConsoleDown;
            }
            // Toggle auto-scroll
            KeyCode::Char('a') => {
                return Action::ToggleDebugAutoScroll;
            }
            // Clear logs
            KeyCode::Char('c') => {
                return Action::ClearDebugLogs;
            }
            _ => return Action::None,
        }
    }

    // Use the shortcuts module to find the action for this key (with two-key support)
    let pending_guard = pending_key_shared.lock().unwrap();
    let (action, should_clear, new_pending_char) =
        crate::shortcuts::find_action_for_key_with_pending(&key, pending_guard.as_ref());

    // Update pending key state
    drop(pending_guard);
    let mut pending_guard = pending_key_shared.lock().unwrap();
    if should_clear {
        *pending_guard = None;
    }
    if let Some(pending_char) = new_pending_char {
        *pending_guard = Some(crate::state::PendingKeyPress {
            key: pending_char,
            timestamp: std::time::Instant::now(),
        });
    }
    drop(pending_guard);

    action
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
