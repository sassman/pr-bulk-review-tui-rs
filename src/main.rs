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
use crate::gh::{comment, merge};
use crate::log::{LogPanel, LogSection, PrContext};
use crate::pr::Pr;
use crate::shortcuts::Action;
use crate::state::*;
use crate::store::Store;
use crate::theme::Theme;

mod config;
mod gh;
mod log;
mod merge_bot;
mod pr;
mod reducer;
mod shortcuts;
mod state;
mod store;
mod theme;

const PALETTES: [tailwind::Palette; 4] = [
    tailwind::BLUE,
    tailwind::EMERALD,
    tailwind::INDIGO,
    tailwind::RED,
];

// TableColors moved to state.rs

// Types moved to state.rs - keeping only App and PersistedState here
// Note: TableColors is now defined in state.rs

struct App {
    // Redux store - centralized state management
    store: Store,
    // Communication channels
    action_tx: mpsc::UnboundedSender<Action>,
    task_tx: mpsc::UnboundedSender<BackgroundTask>,
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

// Background task definitions
enum BackgroundTask {
    LoadAllRepos {
        repos: Vec<Repo>,
        filter: PrFilter,
        octocrab: Octocrab,
    },
    LoadSingleRepo {
        repo_index: usize,
        repo: Repo,
        filter: PrFilter,
        octocrab: Octocrab,
    },
    CheckMergeStatus {
        repo_index: usize,
        repo: Repo,
        pr_numbers: Vec<usize>,
        octocrab: Octocrab,
    },
    Rebase {
        repo: Repo,
        prs: Vec<Pr>,
        selected_indices: Vec<usize>,
        octocrab: Octocrab,
    },
    Merge {
        repo: Repo,
        prs: Vec<Pr>,
        selected_indices: Vec<usize>,
        octocrab: Octocrab,
    },
    FetchBuildLogs {
        repo: Repo,
        pr_number: usize,
        head_sha: String,
        octocrab: Octocrab,
        pr_context: PrContext,
    },
    OpenPRInIDE {
        repo: Repo,
        pr_number: usize,
        ide_command: String,
        temp_dir: String,
    },
    /// Poll a PR to check if it's actually merged (for merge bot)
    PollPRMergeStatus {
        repo_index: usize,
        repo: Repo,
        pr_number: usize,
        octocrab: Octocrab,
        is_checking_ci: bool, // If true, use longer sleep (15s) for CI checks
    },
}

async fn update(app: &mut App, msg: Action) -> Result<Action> {
    // When shortcuts panel is open, remap navigation to shortcuts scrolling
    let msg = if app.store.state().ui.show_shortcuts {
        match msg {
            Action::NavigateToNextPr => Action::ScrollShortcutsDown,
            Action::NavigateToPreviousPr => Action::ScrollShortcutsUp,
            Action::ToggleShortcuts | Action::CloseLogPanel | Action::Quit => msg,
            _ => {
                // Ignore all other actions when shortcuts panel is open
                return Ok(Action::None);
            }
        }
    } else {
        msg
    };

    match msg {
        Action::Quit => {
            // Use Redux dispatch for simple state updates
            app.store.dispatch(Action::Quit);
        }

        Action::Bootstrap => {
            // Stage 1: Loading repositories (fast, synchronous)
            app.store.state_mut().repos.bootstrap_state = BootstrapState::LoadingRepositories;

            match loading_recent_repos() {
                Ok(repos) => {
                    app.store.state_mut().repos.recent_repos = repos;

                    if app.store.state().repos.recent_repos.is_empty() {
                        app.store.state_mut().repos.bootstrap_state = BootstrapState::Error(
                            "No repositories configured. Add repositories to .recent-repositories.json".to_string()
                        );
                        return Ok(Action::None);
                    }
                }
                Err(err) => {
                    app.store.state_mut().repos.bootstrap_state =
                        BootstrapState::Error(format!("Failed to load repositories: {}", err));
                    return Ok(Action::None);
                }
            }

            // Stage 2: Restoring session (fast, synchronous)
            app.store.state_mut().repos.bootstrap_state = BootstrapState::RestoringSession;

            let restored = if let Ok(state) = load_persisted_state() {
                if let Err(err) = app.restore_session(state).await {
                    debug!("Failed to restore session: {}", err);
                    false
                } else {
                    true
                }
            } else {
                false
            };

            if !restored {
                app.store.state_mut().repos.selected_repo = 0;
            }

            // Stage 3: Loading PRs (slow, move to background)
            app.store.state_mut().repos.bootstrap_state = BootstrapState::LoadingPRs;

            // Set status
            app.store.state_mut().task.status = Some(TaskStatus {
                message: format!(
                    "Loading PRs from {} repositories...",
                    app.store.state().repos.recent_repos.len()
                ),
                status_type: TaskStatusType::Running,
            });

            // Set all repos to loading state
            for i in 0..app.store.state().repos.recent_repos.len() {
                let data = app.store.state_mut().repos.repo_data.entry(i).or_default();
                data.loading_state = LoadingState::Loading;
            }

            // Trigger background loading
            let _ = app.task_tx.send(BackgroundTask::LoadAllRepos {
                repos: app.store.state().repos.recent_repos.clone(),
                filter: app.store.state().repos.filter.clone(),
                octocrab: app.octocrab()?,
            });
        }

        Action::RepoDataLoaded(index, result) => {
            // Handle completion of background repo loading
            let data = app.store.state_mut().repos.repo_data.entry(index).or_default();
            match result {
                Ok(prs) => {
                    data.prs = prs.clone();
                    data.loading_state = LoadingState::Loaded;
                    if data.table_state.selected().is_none() && !data.prs.is_empty() {
                        data.table_state.select(Some(0));
                    }

                    // Trigger background merge status checks for this repo
                    if let Some(repo) = app.store.state().repos.recent_repos.get(index).cloned() {
                        let pr_numbers: Vec<usize> = prs.iter().map(|pr| pr.number).collect();
                        let _ = app.task_tx.send(BackgroundTask::CheckMergeStatus {
                            repo_index: index,
                            repo,
                            pr_numbers,
                            octocrab: app.octocrab().unwrap_or_else(|_| {
                                // Fallback - shouldn't happen
                                Octocrab::default()
                            }),
                        });
                    }
                }
                Err(err) => {
                    data.loading_state = LoadingState::Error(err);
                }
            }

            // Load current repo state if this is the selected repo
            if index == app.store.state().repos.selected_repo {
                app.load_repo_state();
            }

            // Check if all repos are done loading
            let all_loaded = app.store.state().repos.repo_data.len() == app.store.state().repos.recent_repos.len()
                && app.store.state().repos.repo_data.values().all(|d| {
                    matches!(
                        d.loading_state,
                        LoadingState::Loaded | LoadingState::Error(_)
                    )
                });

            if all_loaded && app.store.state_mut().repos.bootstrap_state == BootstrapState::LoadingPRs {
                app.store.state_mut().repos.bootstrap_state = BootstrapState::Completed;
                // Clear loading status and show success
                app.store.state_mut().task.status = Some(TaskStatus {
                    message: "All repositories loaded successfully".to_string(),
                    status_type: TaskStatusType::Success,
                });
            }
        }

        Action::MergeStatusUpdated(repo_index, pr_number, status) => {
            // Update the merge status for a specific PR
            if let Some(data) = app.store.state_mut().repos.repo_data.get_mut(&repo_index) {
                if let Some(pr) = data.prs.iter_mut().find(|pr| pr.number == pr_number) {
                    pr.mergeable = status;
                }
            }

            // If this is the current repo, update app.store.state().repos.prs too
            if repo_index == app.store.state().repos.selected_repo {
                if let Some(pr) = app.store.state_mut().repos.prs.iter_mut().find(|pr| pr.number == pr_number) {
                    pr.mergeable = status;
                }
            }

            // Notify merge bot if it's running
            if app.store.state().merge_bot.bot.is_running() {
                app.store.state_mut().merge_bot.bot.handle_status_update(pr_number, status);
            }
        }

        Action::RebaseStatusUpdated(repo_index, pr_number, needs_rebase) => {
            // Update the rebase status for a specific PR
            if let Some(data) = app.store.state_mut().repos.repo_data.get_mut(&repo_index) {
                if let Some(pr) = data.prs.iter_mut().find(|pr| pr.number == pr_number) {
                    pr.needs_rebase = needs_rebase;
                }
            }

            // If this is the current repo, update app.store.state().repos.prs too
            if repo_index == app.store.state().repos.selected_repo {
                if let Some(pr) = app.store.state_mut().repos.prs.iter_mut().find(|pr| pr.number == pr_number) {
                    pr.needs_rebase = needs_rebase;
                }
            }
        }

        Action::RefreshCurrentRepo => {
            // Trigger background refresh
            if let Some(repo) = app.repo().cloned() {
                let selected_repo = app.store.state().repos.selected_repo;
                let filter = app.store.state().repos.filter.clone();

                app.store.state_mut().repos.loading_state = LoadingState::Loading;
                let data = app.store.state_mut().repos.repo_data.entry(selected_repo).or_default();
                data.loading_state = LoadingState::Loading;

                app.store.state_mut().task.status = Some(TaskStatus {
                    message: format!("Refreshing {}/{}...", repo.org, repo.repo),
                    status_type: TaskStatusType::Running,
                });

                let _ = app.task_tx.send(BackgroundTask::LoadSingleRepo {
                    repo_index: selected_repo,
                    repo,
                    filter,
                    octocrab: app.octocrab()?,
                });
            }
        }

        Action::CycleFilter => {
            // Cycle to next filter and reload current repo
            let next_filter = app.store.state().repos.filter.next();
            app.store.state_mut().repos.filter = next_filter.clone();

            let filter_label = next_filter.label();
            app.store.state_mut().task.status = Some(TaskStatus {
                message: format!("Filtering by: {}", filter_label),
                status_type: TaskStatusType::Running,
            });

            if let Some(repo) = app.repo().cloned() {
                let selected_repo = app.store.state().repos.selected_repo;

                app.store.state_mut().repos.loading_state = LoadingState::Loading;
                let data = app.store.state_mut().repos.repo_data.entry(selected_repo).or_default();
                data.loading_state = LoadingState::Loading;

                let _ = app.task_tx.send(BackgroundTask::LoadSingleRepo {
                    repo_index: selected_repo,
                    repo,
                    filter: next_filter,
                    octocrab: app.octocrab()?,
                });
            }
        }

        Action::Rebase => {
            // If user has manually selected PRs, rebase those
            // Otherwise, auto-select all PRs that need rebase
            if let Some(repo) = app.repo().cloned() {
                let (selected_indices, prs_clone) = {
                    let repo_data = app.get_current_repo_data_mut();

                    // Check if user has manually selected PRs
                    let selected_indices = if !repo_data.selected_prs.is_empty() {
                        // Use manual selection
                        repo_data.selected_prs.clone()
                    } else {
                        // Auto-select all PRs that need rebase
                        let prs_needing_rebase: Vec<usize> = repo_data
                            .prs
                            .iter()
                            .enumerate()
                            .filter(|(_, pr)| pr.needs_rebase)
                            .map(|(idx, _)| idx)
                            .collect();

                        if prs_needing_rebase.is_empty() {
                            debug!("No PRs selected and no PRs need rebase");
                            return Ok(Action::None);
                        }

                        // Update selection to PRs needing rebase for visual feedback
                        repo_data.selected_prs = prs_needing_rebase.clone();
                        prs_needing_rebase
                    };

                    (selected_indices, repo_data.prs.clone())
                };

                // Set status to show rebase is starting
                app.store.state_mut().task.status = Some(TaskStatus {
                    message: format!("Rebasing {} PR(s)...", selected_indices.len()),
                    status_type: TaskStatusType::Running,
                });

                let _ = app.task_tx.send(BackgroundTask::Rebase {
                    repo,
                    prs: prs_clone,
                    selected_indices,
                    octocrab: app.octocrab()?,
                });
            }
        }

        Action::RebaseComplete(result) => {
            let success = result.is_ok();

            // Notify merge bot if it's running
            if app.store.state().merge_bot.bot.is_running() {
                app.store.state_mut().merge_bot.bot.handle_rebase_complete(success);
            }

            match result {
                Ok(_) => {
                    debug!("Rebase completed successfully");
                    if !app.store.state().merge_bot.bot.is_running() {
                        app.store.state_mut().task.status = Some(TaskStatus {
                            message: "Rebase completed successfully".to_string(),
                            status_type: TaskStatusType::Success,
                        });
                    }
                }
                Err(err) => {
                    debug!("Rebase failed: {}", err);
                    if !app.store.state().merge_bot.bot.is_running() {
                        app.store.state_mut().task.status = Some(TaskStatus {
                            message: format!("Rebase failed: {}", err),
                            status_type: TaskStatusType::Error,
                        });
                    }
                }
            }
        },

        Action::MergeSelectedPrs => {
            // Trigger background merge
            if let Some(repo) = app.repo().cloned() {
                let repo_data = app.get_current_repo_data();
                let selected_count = repo_data.selected_prs.len();

                app.store.state_mut().task.status = Some(TaskStatus {
                    message: format!("Merging {} PR(s)...", selected_count),
                    status_type: TaskStatusType::Running,
                });

                let _ = app.task_tx.send(BackgroundTask::Merge {
                    repo,
                    prs: repo_data.prs.clone(),
                    selected_indices: repo_data.selected_prs.clone(),
                    octocrab: app.octocrab()?,
                });
            }
        }

        Action::StartMergeBot => {
            // Start the merge bot with selected PRs
            let repo_data = app.get_current_repo_data();

            if repo_data.selected_prs.is_empty() {
                app.store.state_mut().task.status = Some(TaskStatus {
                    message: "No PRs selected for merge bot".to_string(),
                    status_type: TaskStatusType::Error,
                });
            } else {
                // Get PR numbers and indices
                let pr_data: Vec<(usize, usize)> = repo_data
                    .selected_prs
                    .iter()
                    .filter_map(|&idx| {
                        repo_data.prs.get(idx).map(|pr| (pr.number, idx))
                    })
                    .collect();

                app.store.state_mut().merge_bot.bot.start(pr_data);
                app.store.state_mut().task.status = Some(TaskStatus {
                    message: format!("Merge bot started with {} PR(s)", repo_data.selected_prs.len()),
                    status_type: TaskStatusType::Running,
                });
            }
        }

        Action::MergeComplete(result) => {
            let success = result.is_ok();

            // Notify merge bot if it's running
            if app.store.state().merge_bot.bot.is_running() {
                app.store.state_mut().merge_bot.bot.handle_merge_complete(success);
            }

            match result {
                Ok(_) => {
                    debug!("Merge completed successfully");
                    if !app.store.state().merge_bot.bot.is_running() {
                        let selected_repo = app.store.state().repos.selected_repo;

                        app.store.state_mut().task.status = Some(TaskStatus {
                            message: "Merge completed successfully".to_string(),
                            status_type: TaskStatusType::Success,
                        });
                        // Clear selections after successful merge (only if not in merge bot)
                        app.store.state_mut().repos.selected_prs.clear();
                        let data = app.store.state_mut().repos.repo_data.entry(selected_repo).or_default();
                        data.selected_prs.clear();
                    }
                }
                Err(err) => {
                    debug!("Merge failed: {}", err);
                    if !app.store.state().merge_bot.bot.is_running() {
                        app.store.state_mut().task.status = Some(TaskStatus {
                            message: format!("Merge failed: {}", err),
                            status_type: TaskStatusType::Error,
                        });
                    }
                }
            }
        }

        Action::OpenCurrentPrInBrowser => {
            if let Some(repo) = app.repo() {
                let repo_data = app.get_current_repo_data();

                // If multiple PRs are selected, open all of them
                if !repo_data.selected_prs.is_empty() {
                    for &idx in &repo_data.selected_prs {
                        if let Some(pr) = repo_data.prs.get(idx) {
                            let url = format!(
                                "https://github.com/{}/{}/pull/{}",
                                repo.org, repo.repo, pr.number
                            );
                            #[cfg(target_os = "macos")]
                            let _ = std::process::Command::new("open").arg(&url).spawn();
                            #[cfg(target_os = "linux")]
                            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                            #[cfg(target_os = "windows")]
                            let _ = std::process::Command::new("cmd")
                                .args(&["/C", "start", &url])
                                .spawn();
                        }
                    }
                } else if let Some(selected_idx) = repo_data.table_state.selected() {
                    // If no multi-selection, open the currently focused PR
                    if let Some(pr) = repo_data.prs.get(selected_idx) {
                        let url = format!(
                            "https://github.com/{}/{}/pull/{}",
                            repo.org, repo.repo, pr.number
                        );
                        #[cfg(target_os = "macos")]
                        let _ = std::process::Command::new("open").arg(&url).spawn();
                        #[cfg(target_os = "linux")]
                        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                        #[cfg(target_os = "windows")]
                        let _ = std::process::Command::new("cmd")
                            .args(&["/C", "start", &url])
                            .spawn();
                    }
                }
            }
        }

        Action::OpenBuildLogs => {
            // Open logs for any PR - we'll check for failures in the background task
            if let Some(repo) = app.repo().cloned() {
                let repo_data = app.get_current_repo_data();
                if let Some(selected_idx) = repo_data.table_state.selected() {
                    if let Some(pr) = repo_data.prs.get(selected_idx) {
                        app.store.state_mut().task.status = Some(TaskStatus {
                            message: "Loading build logs...".to_string(),
                            status_type: TaskStatusType::Running,
                        });

                        let pr_context = PrContext {
                            number: pr.number,
                            title: pr.title.clone(),
                            author: pr.author.clone(),
                        };

                        let _ = app.task_tx.send(BackgroundTask::FetchBuildLogs {
                            repo,
                            pr_number: pr.number,
                            head_sha: "HEAD".to_string(), // Placeholder - will fetch in background task
                            octocrab: app.octocrab().unwrap_or_else(|_| Octocrab::default()),
                            pr_context,
                        });
                    }
                }
            }
        }

        Action::OpenInIDE => {
            // Open PR in configured IDE
            if let Some(repo) = app.repo().cloned() {
                let repo_data = app.get_current_repo_data();
                if let Some(selected_idx) = repo_data.table_state.selected() {
                    if let Some(pr) = repo_data.prs.get(selected_idx) {
                        app.store.state_mut().task.status = Some(TaskStatus {
                            message: format!("Opening PR #{} in IDE...", pr.number),
                            status_type: TaskStatusType::Running,
                        });

                        let _ = app.task_tx.send(BackgroundTask::OpenPRInIDE {
                            repo,
                            pr_number: pr.number,
                            ide_command: app.store.state().config.ide_command.clone(),
                            temp_dir: app.store.state().config.temp_dir.clone(),
                        });
                    }
                }
            }
        }

        Action::IDEOpenComplete(result) => {
            app.store.state_mut().task.status = Some(match result {
                Ok(()) => TaskStatus {
                    message: "IDE opened successfully".to_string(),
                    status_type: TaskStatusType::Success,
                },
                Err(err) => TaskStatus {
                    message: format!("Failed to open IDE: {}", err),
                    status_type: TaskStatusType::Error,
                },
            });
        }

        Action::PRMergedConfirmed(repo_index, pr_number, is_merged) => {
            // Notify merge bot if it's running
            if app.store.state().merge_bot.bot.is_running() && repo_index == app.store.state().repos.selected_repo {
                app.store.state_mut().merge_bot.bot.handle_pr_merged_confirmed(pr_number, is_merged);
            }
        }

        Action::CloseLogPanel => {
            // Context-aware: close shortcuts panel first if open, otherwise close log panel
            if app.store.state().ui.show_shortcuts {
                app.store.state_mut().ui.show_shortcuts = false;
            } else {
                app.store.state_mut().log_panel.panel = None;
            }
        }

        Action::ToggleShortcuts => {
            app.store.state_mut().ui.show_shortcuts = !app.store.state().ui.show_shortcuts;
            // Reset scroll when opening
            if app.store.state().ui.show_shortcuts {
                app.store.state_mut().ui.shortcuts_scroll = 0;
            }
        }

        Action::ScrollShortcutsUp => {
            app.store.state_mut().ui.shortcuts_scroll = app.store.state().ui.shortcuts_scroll.saturating_sub(1);
        }

        Action::ScrollShortcutsDown => {
            // Only increment if we haven't reached the bottom
            // max_scroll is calculated during rendering based on actual content/viewport size
            if app.store.state().ui.shortcuts_scroll < app.store.state().ui.shortcuts_max_scroll {
                app.store.state_mut().ui.shortcuts_scroll += 1;
            }
        }

        Action::NextLogSection => {
            if let Some(ref mut panel) = app.store.state_mut().log_panel.panel {
                if panel.current_section + 1 < panel.log_sections.len() {
                    panel.current_section += 1;
                    // Jump to the start of this section
                    panel.scroll_offset = panel.log_sections[..panel.current_section]
                        .iter()
                        .map(|s| s.error_lines.len() + 3) // +3 for header and separator
                        .sum();
                }
            }
        }

        Action::ToggleTimestamps => {
            if let Some(ref mut panel) = app.store.state_mut().log_panel.panel {
                panel.show_timestamps = !panel.show_timestamps;
            }
        }

        Action::BuildLogsLoaded(log_sections, pr_context) => {
            if !log_sections.is_empty() {
                let section_count = log_sections.len();
                app.store.state_mut().log_panel.panel = Some(LogPanel {
                    log_sections,
                    scroll_offset: 0,
                    current_section: 0,
                    horizontal_scroll: 0,
                    pr_context,
                    show_timestamps: false,
                });
                app.store.state_mut().task.status = Some(TaskStatus {
                    message: format!("Build logs loaded ({} check run(s))", section_count),
                    status_type: TaskStatusType::Success,
                });
            } else {
                app.store.state_mut().task.status = Some(TaskStatus {
                    message: "No check runs found for this PR".to_string(),
                    status_type: TaskStatusType::Error,
                });
            }
        }

        Action::SelectNextRepo => {
            app.select_next_repo();
        }
        Action::SelectPreviousRepo => {
            app.select_previous_repo();
        }
        Action::SelectRepoByIndex(index) => {
            app.select_repo_by_index(index);
        }
        Action::TogglePrSelection => {
            app.select_toggle();
            // Move to next PR after toggling selection for easier bulk selection
            app.next();
        }
        Action::NavigateToNextPr => {
            // If log panel is open, scroll down instead
            if app.store.state().log_panel.panel.is_some() {
                if let Some(ref mut panel) = app.store.state_mut().log_panel.panel {
                    panel.scroll_offset = panel.scroll_offset.saturating_add(1);
                }
            } else {
                app.next();
            }
        }
        Action::NavigateToPreviousPr => {
            // If log panel is open, scroll up instead
            if app.store.state().log_panel.panel.is_some() {
                if let Some(ref mut panel) = app.store.state_mut().log_panel.panel {
                    panel.scroll_offset = panel.scroll_offset.saturating_sub(1);
                }
            } else {
                app.previous();
            }
        }
        Action::ScrollLogPanelLeft => {
            // Only handle if log panel is open
            if let Some(ref mut panel) = app.store.state_mut().log_panel.panel {
                panel.horizontal_scroll = panel.horizontal_scroll.saturating_sub(5);
            }
        }
        Action::ScrollLogPanelRight => {
            // Only handle if log panel is open
            if let Some(ref mut panel) = app.store.state_mut().log_panel.panel {
                panel.horizontal_scroll = panel.horizontal_scroll.saturating_add(5);
            }
        }

        _ => {}
    };
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

/// Background task worker that processes heavy operations without blocking UI
fn start_task_worker(
    mut task_rx: mpsc::UnboundedReceiver<BackgroundTask>,
    action_tx: mpsc::UnboundedSender<Action>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(task) = task_rx.recv().await {
            match task {
                BackgroundTask::LoadAllRepos {
                    repos,
                    filter,
                    octocrab,
                } => {
                    // Spawn parallel tasks for each repo
                    let mut tasks = Vec::new();
                    for (index, repo) in repos.iter().enumerate() {
                        let octocrab = octocrab.clone();
                        let repo = repo.clone();
                        let filter = filter.clone();

                        let task = tokio::spawn(async move {
                            let result = fetch_github_data(&octocrab, &repo, &filter)
                                .await
                                .map_err(|e| e.to_string());
                            (index, result)
                        });
                        tasks.push(task);
                    }

                    // Collect results and send back to UI thread
                    for task in tasks {
                        if let Ok((index, result)) = task.await {
                            let _ = action_tx.send(Action::RepoDataLoaded(index, result));
                        }
                    }
                }
                BackgroundTask::LoadSingleRepo {
                    repo_index,
                    repo,
                    filter,
                    octocrab,
                } => {
                    let result = fetch_github_data(&octocrab, &repo, &filter)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = action_tx.send(Action::RepoDataLoaded(repo_index, result));
                }
                BackgroundTask::CheckMergeStatus {
                    repo_index,
                    repo,
                    pr_numbers,
                    octocrab,
                } => {
                    // Check merge status for each PR in parallel
                    let mut tasks = Vec::new();
                    for pr_number in pr_numbers {
                        let octocrab = octocrab.clone();
                        let repo = repo.clone();
                        let action_tx = action_tx.clone();

                        let task = tokio::spawn(async move {
                            use crate::pr::MergeableStatus;

                            // Fetch detailed PR info to get mergeable status and rebase status
                            match octocrab
                                .pulls(&repo.org, &repo.repo)
                                .get(pr_number as u64)
                                .await
                            {
                                Ok(pr_detail) => {
                                    // Check if PR needs rebase (Behind state means PR is behind base branch)
                                    let needs_rebase =
                                        if let Some(ref state) = pr_detail.mergeable_state {
                                            matches!(
                                                state,
                                                octocrab::models::pulls::MergeableState::Behind
                                            )
                                        } else {
                                            false
                                        };

                                    // Check CI/build status by fetching check runs
                                    let head_sha = pr_detail.head.sha.clone();

                                    // Use the REST API directly to get check runs
                                    let check_runs_url = format!(
                                        "/repos/{}/{}/commits/{}/check-runs",
                                        repo.org, repo.repo, head_sha
                                    );

                                    #[derive(Debug, serde::Deserialize)]
                                    struct CheckRunsResponse {
                                        check_runs: Vec<CheckRun>,
                                    }

                                    #[derive(Debug, serde::Deserialize)]
                                    struct CheckRun {
                                        status: String,
                                        conclusion: Option<String>,
                                    }

                                    let (ci_failed, ci_in_progress) = match octocrab
                                        .get::<CheckRunsResponse, _, ()>(
                                            &check_runs_url,
                                            None::<&()>,
                                        )
                                        .await
                                    {
                                        Ok(response) => {
                                            // Check if any check run failed
                                            let failed = response.check_runs.iter().any(|check| {
                                                check.status == "completed"
                                                    && (check.conclusion.as_deref()
                                                        == Some("failure")
                                                        || check.conclusion.as_deref()
                                                            == Some("cancelled")
                                                        || check.conclusion.as_deref()
                                                            == Some("timed_out"))
                                            });
                                            // Check if any check run is still in progress
                                            let in_progress =
                                                response.check_runs.iter().any(|check| {
                                                    check.status == "queued"
                                                        || check.status == "in_progress"
                                                });
                                            (failed, in_progress)
                                        }
                                        Err(_) => {
                                            // Fallback: use mergeable_state "unstable" as indicator
                                            let failed = if let Some(ref state) =
                                                pr_detail.mergeable_state
                                            {
                                                matches!(
                                                    state,
                                                    octocrab::models::pulls::MergeableState::Unstable
                                                )
                                            } else {
                                                false
                                            };
                                            (failed, false)
                                        }
                                    };

                                    // Determine final status with priority:
                                    // 1. Conflicted (mergeable=false && dirty)
                                    // 2. BuildFailed (CI checks failed)
                                    // 3. Checking (CI checks in progress)
                                    // 4. NeedsRebase (branch is behind)
                                    // 5. Blocked (other blocking reasons)
                                    // 6. Ready (all good!)
                                    let status = match pr_detail.mergeable {
                                        Some(false) => {
                                            // Not mergeable - check why
                                            if let Some(ref state) = pr_detail.mergeable_state {
                                                match state {
                                                    octocrab::models::pulls::MergeableState::Dirty => MergeableStatus::Conflicted,
                                                    octocrab::models::pulls::MergeableState::Blocked => {
                                                        if ci_failed {
                                                            MergeableStatus::BuildFailed
                                                        } else if ci_in_progress {
                                                            MergeableStatus::BuildInProgress
                                                        } else {
                                                            MergeableStatus::Blocked
                                                        }
                                                    }
                                                    _ => MergeableStatus::Blocked,
                                                }
                                            } else {
                                                MergeableStatus::Conflicted
                                            }
                                        }
                                        Some(true) => {
                                            // Mergeable, but check for other issues
                                            if ci_failed {
                                                MergeableStatus::BuildFailed
                                            } else if ci_in_progress {
                                                MergeableStatus::BuildInProgress
                                            } else if needs_rebase {
                                                MergeableStatus::NeedsRebase
                                            } else {
                                                MergeableStatus::Ready
                                            }
                                        }
                                        None => {
                                            // mergeable status unknown - check if CI is running
                                            if ci_in_progress {
                                                MergeableStatus::BuildInProgress
                                            } else {
                                                MergeableStatus::Unknown
                                            }
                                        }
                                    };

                                    let _ = action_tx.send(Action::MergeStatusUpdated(
                                        repo_index, pr_number, status,
                                    ));
                                    let _ = action_tx.send(Action::RebaseStatusUpdated(
                                        repo_index,
                                        pr_number,
                                        needs_rebase,
                                    ));
                                }
                                Err(_) => {
                                    // Failed to fetch, keep as unknown
                                }
                            }
                        });
                        tasks.push(task);
                    }

                    // Wait for all checks to complete
                    for task in tasks {
                        let _ = task.await;
                    }
                }
                BackgroundTask::Rebase {
                    repo,
                    prs,
                    selected_indices,
                    octocrab,
                } => {
                    use crate::pr::MergeableStatus;

                    let mut success = true;
                    for &idx in &selected_indices {
                        if let Some(pr) = prs.get(idx) {
                            // For dependabot PRs, use comment-based rebase
                            if pr.author.starts_with("dependabot") {
                                // If PR has conflicts, use "@dependabot recreate" to rebuild the PR
                                // Otherwise use "@dependabot rebase" for normal rebase
                                let comment_text = if pr.mergeable == MergeableStatus::Conflicted {
                                    "@dependabot recreate"
                                } else {
                                    "@dependabot rebase"
                                };

                                if let Err(_) = comment(&octocrab, &repo, pr, comment_text).await {
                                    success = false;
                                }
                            } else {
                                // For regular PRs, use GitHub's update_branch API
                                // This performs a rebase/merge to bring the PR branch up to date with base
                                let update_result = octocrab
                                    .pulls(&repo.org, &repo.repo)
                                    .update_branch(pr.number as u64)
                                    .await;

                                if update_result.is_err() {
                                    success = false;
                                }
                            }
                        }
                    }
                    let result = if success {
                        Ok(())
                    } else {
                        Err("Some rebases failed".to_string())
                    };
                    let _ = action_tx.send(Action::RebaseComplete(result));
                }
                BackgroundTask::Merge {
                    repo,
                    prs,
                    selected_indices,
                    octocrab,
                } => {
                    let mut success = true;
                    for &idx in &selected_indices {
                        if let Some(pr) = prs.get(idx) {
                            if let Err(_) = merge(&octocrab, &repo, pr).await {
                                success = false;
                            }
                        }
                    }
                    let result = if success {
                        Ok(())
                    } else {
                        Err("Some merges failed".to_string())
                    };
                    let _ = action_tx.send(Action::MergeComplete(result));
                }
                BackgroundTask::FetchBuildLogs {
                    repo,
                    pr_number,
                    head_sha: _,
                    octocrab,
                    pr_context,
                } => {
                    // First, get the PR details to get the actual head SHA
                    let pr_details = match octocrab
                        .pulls(&repo.org, &repo.repo)
                        .get(pr_number as u64)
                        .await
                    {
                        Ok(pr) => pr,
                        Err(_) => {
                            let _ = action_tx.send(Action::BuildLogsLoaded(vec![], pr_context));
                            return;
                        }
                    };

                    let head_sha = pr_details.head.sha.clone();

                    // Get workflow runs for this commit using the REST API directly
                    let url = format!(
                        "/repos/{}/{}/actions/runs?head_sha={}",
                        repo.org, repo.repo, head_sha
                    );

                    #[derive(Debug, serde::Deserialize)]
                    struct WorkflowRunsResponse {
                        workflow_runs: Vec<octocrab::models::workflows::Run>,
                    }

                    let workflow_runs: WorkflowRunsResponse =
                        match octocrab.get(&url, None::<&()>).await {
                            Ok(runs) => runs,
                            Err(_) => {
                                let _ = action_tx.send(Action::BuildLogsLoaded(vec![], pr_context));
                                return;
                            }
                        };

                    let mut log_sections = Vec::new();

                    // Process each workflow run and download its logs
                    for workflow_run in workflow_runs.workflow_runs {
                        let conclusion_str =
                            workflow_run.conclusion.as_deref().unwrap_or("in_progress");
                        let workflow_name = workflow_run.name.clone();

                        // Skip successful runs unless there are no failures
                        let is_failed = matches!(
                            conclusion_str,
                            "failure" | "timed_out" | "action_required" | "cancelled"
                        );

                        if !is_failed && !log_sections.is_empty() {
                            continue;
                        }

                        let mut metadata_lines = Vec::new();
                        metadata_lines.push(format!("Workflow: {}", workflow_name));
                        metadata_lines.push(format!("Run ID: {}", workflow_run.id));
                        metadata_lines.push(format!("Run URL: {}", workflow_run.html_url));
                        metadata_lines.push(format!("Conclusion: {}", conclusion_str));
                        metadata_lines.push(format!("Started: {}", workflow_run.created_at));
                        metadata_lines.push(format!("Updated: {}", workflow_run.updated_at));
                        metadata_lines.push("".to_string());

                        // Fetch jobs for this workflow run to get job IDs and URLs
                        let jobs_url = format!(
                            "/repos/{}/{}/actions/runs/{}/jobs",
                            repo.org, repo.repo, workflow_run.id
                        );

                        #[derive(Debug, serde::Deserialize)]
                        struct JobsResponse {
                            jobs: Vec<WorkflowJob>,
                        }

                        #[derive(Debug, serde::Deserialize)]
                        struct WorkflowJob {
                            id: u64,
                            name: String,
                            html_url: String,
                        }

                        let jobs_response: Result<JobsResponse, _> =
                            octocrab.get(&jobs_url, None::<&()>).await;

                        // Try to download the workflow run logs (they come as a zip file)
                        match octocrab
                            .actions()
                            .download_workflow_run_logs(
                                &repo.org,
                                &repo.repo,
                                workflow_run.id.into(),
                            )
                            .await
                        {
                            Ok(log_data) => {
                                // The log_data is a zip file as bytes
                                // We need to extract and parse it
                                match crate::log::parse_workflow_logs_zip(&log_data) {
                                    Ok(job_logs) => {
                                        // Process each job's logs separately
                                        for job_log in job_logs {
                                            // Try to find matching job URL by name
                                            let job_url = if let Ok(ref jobs) = jobs_response {
                                                jobs.jobs
                                                    .iter()
                                                    .find(|j| job_log.job_name.contains(&j.name))
                                                    .map(|j| j.html_url.clone())
                                            } else {
                                                None
                                            };

                                            let mut job_metadata = metadata_lines.clone();
                                            job_metadata.push(format!("Job: {}", job_log.job_name));
                                            if let Some(url) = &job_url {
                                                job_metadata.push(format!("Job URL: {}", url));
                                            }
                                            job_metadata.push("".to_string());

                                            // Try to extract error context from this job's logs
                                            let full_log_text = job_log.content.join("\n");
                                            let error_context = crate::log::extract_error_context(
                                                &full_log_text,
                                                &job_log.job_name,
                                            );

                                            if !error_context.is_empty() {
                                                // We found specific errors - create error context section
                                                let mut error_lines = job_metadata.clone();
                                                error_lines.push("Error Context:".to_string());
                                                error_lines.push("".to_string());
                                                error_lines.extend(error_context);

                                                log_sections.push(LogSection {
                                                    step_name: format!(
                                                        "{} / {} - Errors",
                                                        workflow_name, job_log.job_name
                                                    ),
                                                    error_lines,
                                                    has_extracted_errors: true,
                                                });

                                                // Also create full log section (will be sorted to bottom)
                                                let mut full_lines = job_metadata.clone();
                                                full_lines.push("Full Job Logs:".to_string());
                                                full_lines.push("".to_string());
                                                full_lines.extend(job_log.content);

                                                log_sections.push(LogSection {
                                                    step_name: format!(
                                                        "{} / {} - Full Log",
                                                        workflow_name, job_log.job_name
                                                    ),
                                                    error_lines: full_lines,
                                                    has_extracted_errors: false,
                                                });
                                            } else {
                                                // No specific errors found - just show full log
                                                let mut full_lines = job_metadata;
                                                full_lines.push("Job Logs:".to_string());
                                                full_lines.push("".to_string());
                                                full_lines.extend(job_log.content);

                                                log_sections.push(LogSection {
                                                    step_name: format!(
                                                        "{} / {}",
                                                        workflow_name, job_log.job_name
                                                    ),
                                                    error_lines: full_lines,
                                                    has_extracted_errors: false,
                                                });
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        let mut error_lines = metadata_lines;
                                        error_lines.push(format!("Failed to parse logs: {}", err));
                                        error_lines.push("".to_string());
                                        error_lines.push(format!(
                                            "View logs at: {}",
                                            workflow_run.html_url
                                        ));

                                        log_sections.push(LogSection {
                                            step_name: format!(
                                                "{} [{}]",
                                                workflow_name, conclusion_str
                                            ),
                                            error_lines,
                                            has_extracted_errors: false,
                                        });
                                    }
                                }
                            }
                            Err(_) => {
                                let mut error_lines = metadata_lines;
                                error_lines.push("Unable to download logs via API".to_string());
                                error_lines.push(
                                    "This may require authentication or the logs may have expired."
                                        .to_string(),
                                );
                                error_lines.push("".to_string());
                                error_lines
                                    .push(format!("View logs at: {}", workflow_run.html_url));

                                log_sections.push(LogSection {
                                    step_name: format!("{} [{}]", workflow_name, conclusion_str),
                                    error_lines,
                                    has_extracted_errors: false,
                                });
                            }
                        }
                    }

                    // Sort sections: error contexts first, full logs last
                    log_sections.sort_by_key(|section| !section.has_extracted_errors);

                    // If we didn't find any workflow runs, add a helpful message
                    if log_sections.is_empty() {
                        log_sections.push(LogSection {
                            step_name: "No Workflow Runs Found".to_string(),
                            error_lines: vec![
                                "This PR doesn't have any GitHub Actions workflow runs.".to_string(),
                                "".to_string(),
                                "This could mean:".to_string(),
                                "- No GitHub Actions workflows configured for this repository".to_string(),
                                "- Workflows haven't been triggered yet for this commit".to_string(),
                                "- CI/CD is using a different system (CircleCI, Travis, Jenkins, etc.)".to_string(),
                                "".to_string(),
                                "Try opening the PR in browser (press Enter) to check for other CI systems.".to_string(),
                            ],
                            has_extracted_errors: false,
                        });
                    }

                    let _ = action_tx.send(Action::BuildLogsLoaded(log_sections, pr_context));
                }

                BackgroundTask::OpenPRInIDE {
                    repo,
                    pr_number,
                    ide_command,
                    temp_dir,
                } => {
                    use std::process::Command;

                    // Create temp directory if it doesn't exist
                    if let Err(err) = std::fs::create_dir_all(&temp_dir) {
                        let _ = action_tx.send(Action::IDEOpenComplete(Err(format!(
                            "Failed to create temp directory: {}",
                            err
                        ))));
                        return;
                    }

                    // Create unique directory for this PR
                    let pr_dir = PathBuf::from(&temp_dir)
                        .join(format!("{}-{}-pr-{}", repo.org, repo.repo, pr_number));

                    // Remove existing directory if present
                    if pr_dir.exists() {
                        if let Err(err) = std::fs::remove_dir_all(&pr_dir) {
                            let _ = action_tx.send(Action::IDEOpenComplete(Err(format!(
                                "Failed to remove existing directory: {}",
                                err
                            ))));
                            return;
                        }
                    }

                    // Clone the repository using gh repo clone (uses SSH by default)
                    let clone_output = Command::new("gh")
                        .args(&[
                            "repo",
                            "clone",
                            &format!("{}/{}", repo.org, repo.repo),
                            &pr_dir.to_string_lossy(),
                        ])
                        .output();

                    if let Err(err) = clone_output {
                        let _ = action_tx.send(Action::IDEOpenComplete(Err(format!(
                            "Failed to run gh repo clone: {}",
                            err
                        ))));
                        return;
                    }

                    let clone_output = clone_output.unwrap();
                    if !clone_output.status.success() {
                        let stderr = String::from_utf8_lossy(&clone_output.stderr);
                        let _ = action_tx.send(Action::IDEOpenComplete(Err(format!(
                            "gh repo clone failed: {}",
                            stderr
                        ))));
                        return;
                    }

                    // Now checkout the PR using gh pr checkout
                    let checkout_output = Command::new("gh")
                        .args(&["pr", "checkout", &pr_number.to_string()])
                        .current_dir(&pr_dir)
                        .output();

                    if let Err(err) = checkout_output {
                        let _ = action_tx.send(Action::IDEOpenComplete(Err(format!(
                            "Failed to run gh pr checkout: {}",
                            err
                        ))));
                        return;
                    }

                    let checkout_output = checkout_output.unwrap();
                    if !checkout_output.status.success() {
                        let stderr = String::from_utf8_lossy(&checkout_output.stderr);
                        let _ = action_tx.send(Action::IDEOpenComplete(Err(format!(
                            "gh pr checkout failed: {}",
                            stderr
                        ))));
                        return;
                    }

                    // Set origin URL to SSH (gh checkout doesn't do this)
                    let ssh_url = format!("git@github.com:{}/{}.git", repo.org, repo.repo);
                    let set_url_output = Command::new("git")
                        .args(&["remote", "set-url", "origin", &ssh_url])
                        .current_dir(&pr_dir)
                        .output();

                    if let Err(err) = set_url_output {
                        let _ = action_tx.send(Action::IDEOpenComplete(Err(format!(
                            "Failed to set SSH origin URL: {}",
                            err
                        ))));
                        return;
                    }

                    let set_url_output = set_url_output.unwrap();
                    if !set_url_output.status.success() {
                        let stderr = String::from_utf8_lossy(&set_url_output.stderr);
                        let _ = action_tx.send(Action::IDEOpenComplete(Err(format!(
                            "Failed to set SSH origin URL: {}",
                            stderr
                        ))));
                        return;
                    }

                    // Open in IDE
                    let ide_output = Command::new(&ide_command).arg(&pr_dir).spawn();

                    match ide_output {
                        Ok(_) => {
                            let _ = action_tx.send(Action::IDEOpenComplete(Ok(())));
                        }
                        Err(err) => {
                            let _ = action_tx.send(Action::IDEOpenComplete(Err(format!(
                                "Failed to open IDE '{}': {}",
                                ide_command, err
                            ))));
                        }
                    }
                }
                BackgroundTask::PollPRMergeStatus {
                    repo_index,
                    repo,
                    pr_number,
                    octocrab,
                    is_checking_ci,
                } => {
                    // Poll the PR to check status
                    // Wait before polling to give GitHub time to process
                    // Use longer sleep (15s) when checking CI, shorter (2s) for merge confirmation
                    let sleep_duration = if is_checking_ci {
                        tokio::time::Duration::from_secs(15) // CI can take 4-10 minutes
                    } else {
                        tokio::time::Duration::from_secs(2) // Merge is usually quick
                    };
                    tokio::time::sleep(sleep_duration).await;

                    match octocrab
                        .pulls(&repo.org, &repo.repo)
                        .get(pr_number as u64)
                        .await
                    {
                        Ok(pr_detail) => {
                            if is_checking_ci {
                                // When checking CI, use GitHub's mergeable field which considers branch protection
                                // This properly handles PRs with failed non-required checks
                                use crate::pr::MergeableStatus;

                                // Check if PR needs rebase
                                let needs_rebase = if let Some(ref state) = pr_detail.mergeable_state {
                                    matches!(state, octocrab::models::pulls::MergeableState::Behind)
                                } else {
                                    false
                                };

                                // Check CI/build status
                                let head_sha = pr_detail.head.sha.clone();
                                let check_runs_url = format!(
                                    "/repos/{}/{}/commits/{}/check-runs",
                                    repo.org, repo.repo, head_sha
                                );

                                #[derive(Debug, serde::Deserialize)]
                                struct CheckRunsResponse {
                                    check_runs: Vec<CheckRun>,
                                }

                                #[derive(Debug, serde::Deserialize)]
                                struct CheckRun {
                                    status: String,
                                    conclusion: Option<String>,
                                }

                                let (ci_failed, ci_in_progress) = match octocrab.get::<CheckRunsResponse, _, ()>(&check_runs_url, None::<&()>).await {
                                    Ok(response) => {
                                        let failed = response.check_runs.iter().any(|check| {
                                            check.status == "completed" && matches!(
                                                check.conclusion.as_deref(),
                                                Some("failure") | Some("cancelled") | Some("timed_out")
                                            )
                                        });
                                        let in_progress = response.check_runs.iter().any(|check| {
                                            check.status == "queued" || check.status == "in_progress"
                                        });
                                        (failed, in_progress)
                                    }
                                    Err(_) => {
                                        // Fallback: use mergeable_state as indicator
                                        let failed = if let Some(ref state) = pr_detail.mergeable_state {
                                            matches!(state, octocrab::models::pulls::MergeableState::Unstable)
                                        } else {
                                            false
                                        };
                                        (failed, false)
                                    }
                                };

                                // Determine status using GitHub's mergeable field
                                // This properly handles required vs optional check failures
                                let status = match pr_detail.mergeable {
                                    Some(false) => {
                                        // Not mergeable - check why
                                        if let Some(ref state) = pr_detail.mergeable_state {
                                            match state {
                                                octocrab::models::pulls::MergeableState::Dirty => MergeableStatus::Conflicted,
                                                octocrab::models::pulls::MergeableState::Blocked => {
                                                    if ci_failed {
                                                        MergeableStatus::BuildFailed
                                                    } else if ci_in_progress {
                                                        MergeableStatus::BuildInProgress
                                                    } else {
                                                        MergeableStatus::Blocked
                                                    }
                                                }
                                                _ => MergeableStatus::Blocked,
                                            }
                                        } else {
                                            MergeableStatus::Conflicted
                                        }
                                    }
                                    Some(true) => {
                                        // PR is mergeable according to GitHub (required checks passed)
                                        // Even if some non-required checks failed, we can merge
                                        if ci_in_progress {
                                            MergeableStatus::BuildInProgress
                                        } else if needs_rebase {
                                            MergeableStatus::NeedsRebase
                                        } else {
                                            MergeableStatus::Ready
                                        }
                                    }
                                    None => {
                                        // mergeable status unknown - check if CI is running
                                        if ci_in_progress {
                                            MergeableStatus::BuildInProgress
                                        } else {
                                            MergeableStatus::Unknown
                                        }
                                    }
                                };

                                let _ = action_tx.send(Action::MergeStatusUpdated(repo_index, pr_number, status));
                            } else {
                                // When checking merge confirmation, just check if PR is merged
                                let is_merged = pr_detail.merged_at.is_some();
                                let _ = action_tx.send(Action::PRMergedConfirmed(repo_index, pr_number, is_merged));
                            }
                        }
                        Err(_) => {
                            if is_checking_ci {
                                // Can't fetch PR, send unknown status
                                let _ = action_tx.send(Action::MergeStatusUpdated(repo_index, pr_number, crate::pr::MergeableStatus::Unknown));
                            } else {
                                // Can't fetch PR, assume not merged yet
                                let _ = action_tx.send(Action::PRMergedConfirmed(repo_index, pr_number, false));
                            }
                        }
                    }
                }
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
                    app.store.state_mut().repos.loading_state = LoadingState::Error(err.to_string());
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
                        if let Some(action) = app.store.state_mut().merge_bot.bot.process_next(&repo_data.prs) {
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
                                        let _ = app.task_tx.send(BackgroundTask::PollPRMergeStatus {
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
        .store.state().repos.recent_repos
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
        .style(Style::default().bg(Color::Blue))
        .height(1);

    let selected_row_style = Style::default()
        .add_modifier(Modifier::REVERSED)
        .fg(app.store.state().repos.colors.selected_row_style_fg);

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
            row.style(Style::new().fg(app.store.state().repos.colors.row_fg).bg(color))
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
        crate::log::render_log_panel_card(f, panel, &app.store.state().repos.colors, chunks[1]);
    }

    // Render shortcuts panel on top of everything if visible
    if app.store.state().ui.show_shortcuts {
        let max_scroll =
            crate::shortcuts::render_shortcuts_panel(f, chunks[1], app.store.state().ui.shortcuts_scroll, &app.store.state().theme);
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
                .store.state().log_panel.panel
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
                    .fg(Color::White)
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
            TaskStatusType::Running => ("⏳", Color::Yellow),
            TaskStatusType::Success => ("✓", Color::Green),
            TaskStatusType::Error => ("✗", Color::Red),
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
                .store.state().repos.repo_data
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
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(spinner_widget, chunks[3]);
    } else {
        let error_icon = Paragraph::new("✗ Error")
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
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
            .style(Style::default().fg(tailwind::BLUE.c400))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(bar_widget, chunks[5]);
    }

    // Status message
    let message_style = if is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(tailwind::SLATE.c300)
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
        let initial_state = AppState {
            ui: UiState::default(),
            repos: ReposState {
                colors: TableColors::new(&PALETTES[0]),
                ..ReposState::default()
            },
            log_panel: LogPanelState::default(),
            merge_bot: MergeBotState::default(),
            task: TaskState::default(),
            config: Config::load(),
            theme: Theme::default(),
        };

        App {
            store: Store::new(initial_state),
            action_tx,
            task_tx,
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

        let data = self.store
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
        Ok(Octocrab::builder()
            .personal_token(
                env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN environment variable must be set"),
            )
            .build()?)
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
        let data = self.store.state_mut().repos.repo_data.entry(selected_repo).or_default();
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
        let data = self.store.state_mut().repos.repo_data.entry(selected_repo).or_default();
        data.table_state.select(Some(i));
    }

    /// Toggle the selection of the currently selected PR
    fn select_toggle(&mut self) {
        let repo_data = self.get_current_repo_data();
        let i = repo_data.table_state.selected().unwrap_or(0);

        // Update both the app state and repo data
        if self.store.state().repos.selected_prs.contains(&i) {
            self.store.state_mut().repos.selected_prs.retain(|&x| x != i);
        } else {
            self.store.state_mut().repos.selected_prs.push(i);
        }

        let selected_repo = self.store.state().repos.selected_repo;
        let data = self.store.state_mut().repos.repo_data.entry(selected_repo).or_default();
        if data.selected_prs.contains(&i) {
            data.selected_prs.retain(|&x| x != i);
        } else {
            data.selected_prs.push(i);
        }
    }

    /// Select the next repo (cycle forward through tabs)
    fn select_next_repo(&mut self) {
        self.save_current_repo_state();
        self.store.state_mut().repos.selected_repo = (self.store.state().repos.selected_repo + 1) % self.store.state().repos.recent_repos.len();
        self.load_repo_state();
    }

    /// Select the previous repo (cycle backward through tabs)
    fn select_previous_repo(&mut self) {
        self.save_current_repo_state();
        self.store.state_mut().repos.selected_repo = if self.store.state_mut().repos.selected_repo == 0 {
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
                let data = self.store.state_mut().repos.repo_data.entry(index).or_default();
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
        let data = self.store.state_mut().repos.repo_data.entry(selected_repo).or_default();
        data.loading_state = LoadingState::Loading;

        self.fetch_data(&repo).await?;

        // Update the repo data cache
        let prs = self.store.state().repos.prs.clone();
        let loading_state = self.store.state().repos.loading_state.clone();
        let data = self.store.state_mut().repos.repo_data.entry(selected_repo).or_default();
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
        debug!("Rebasing selected PRs: {:?}", self.store.state().repos.selected_prs);

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
        debug!("Merging selected PRs: {:?}", self.store.state().repos.selected_prs);

        Ok(())
    }
}

async fn fetch_github_data<'a>(
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
