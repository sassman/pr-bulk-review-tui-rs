use crate::{
    shortcuts::Action,
    state::*,
};

/// Root reducer that delegates to sub-reducers based on action type
/// Pure function: takes state and action, returns new state
pub fn reduce(mut state: AppState, action: &Action) -> AppState {
    // Apply each sub-reducer
    state.ui = ui_reducer(state.ui, action);
    state.repos = repos_reducer(state.repos, action);
    state.log_panel = log_panel_reducer(state.log_panel, action);
    state.merge_bot = merge_bot_reducer(state.merge_bot, action);
    state.task = task_reducer(state.task, action);

    state
}

/// UI state reducer - handles UI-related actions
fn ui_reducer(mut state: UiState, action: &Action) -> UiState {
    match action {
        Action::Quit => {
            state.should_quit = true;
        }
        Action::ToggleShortcuts => {
            state.show_shortcuts = !state.show_shortcuts;
        }
        Action::ScrollShortcutsUp => {
            state.shortcuts_scroll = state.shortcuts_scroll.saturating_sub(1);
        }
        Action::ScrollShortcutsDown => {
            if state.shortcuts_scroll < state.shortcuts_max_scroll {
                state.shortcuts_scroll += 1;
            }
        }
        Action::CloseLogPanel => {
            // Close shortcuts panel first if open
            if state.show_shortcuts {
                state.show_shortcuts = false;
            }
        }
        _ => {}
    }

    state
}

/// Repository and PR state reducer
fn repos_reducer(mut state: ReposState, action: &Action) -> ReposState {
    match action {
        Action::BootstrapComplete(Ok(result)) => {
            state.recent_repos = result.repos.clone();
            state.selected_repo = result.selected_repo;
            state.bootstrap_state = BootstrapState::LoadingPRs;
        }
        Action::BootstrapComplete(Err(_)) => {
            // Error handled elsewhere
        }
        Action::SelectRepoByIndex(index) => {
            if *index < state.recent_repos.len() {
                state.selected_repo = *index;

                // Sync legacy fields with repo_data
                if let Some(data) = state.repo_data.get(index) {
                    state.prs = data.prs.clone();
                    state.state = data.table_state.clone();
                    state.selected_prs = data.selected_prs.clone();
                    state.loading_state = data.loading_state.clone();
                }
            }
        }
        Action::RepoDataLoaded(repo_index, Ok(prs)) => {
            let data = state.repo_data.entry(*repo_index).or_default();
            data.prs = prs.clone();
            data.loading_state = LoadingState::Loaded;

            // Sync legacy fields if this is the selected repo
            if *repo_index == state.selected_repo {
                state.prs = prs.clone();
                state.loading_state = LoadingState::Loaded;
            }
        }
        Action::RepoDataLoaded(_, Err(_)) => {
            // Error handled elsewhere
        }
        Action::CycleFilter => {
            state.filter = state.filter.next();
        }
        Action::NavigateToNextPr => {
            let i = match state.state.selected() {
                Some(i) => {
                    if i >= state.prs.len().saturating_sub(1) {
                        0
                    } else {
                        i + 1
                    }
                }
                None => 0,
            };
            state.state.select(Some(i));

            // Sync to repo_data
            if let Some(data) = state.repo_data.get_mut(&state.selected_repo) {
                data.table_state.select(Some(i));
            }
        }
        Action::NavigateToPreviousPr => {
            let i = match state.state.selected() {
                Some(i) => {
                    if i == 0 {
                        state.prs.len().saturating_sub(1)
                    } else {
                        i - 1
                    }
                }
                None => 0,
            };
            state.state.select(Some(i));

            // Sync to repo_data
            if let Some(data) = state.repo_data.get_mut(&state.selected_repo) {
                data.table_state.select(Some(i));
            }
        }
        Action::TogglePrSelection => {
            if let Some(selected) = state.state.selected() {
                if state.selected_prs.contains(&selected) {
                    state.selected_prs.retain(|&i| i != selected);
                } else {
                    state.selected_prs.push(selected);
                }
                state.selected_prs.sort_unstable();

                // Sync to repo_data
                if let Some(data) = state.repo_data.get_mut(&state.selected_repo) {
                    data.selected_prs = state.selected_prs.clone();
                }
            }
        }
        Action::MergeStatusUpdated(repo_index, pr_number, status) => {
            // Update PR status in repo_data
            if let Some(data) = state.repo_data.get_mut(repo_index) {
                if let Some(pr) = data.prs.iter_mut().find(|p| p.number == *pr_number) {
                    pr.mergeable = *status;
                }
            }

            // Sync legacy fields if this is the selected repo
            if *repo_index == state.selected_repo {
                if let Some(pr) = state.prs.iter_mut().find(|p| p.number == *pr_number) {
                    pr.mergeable = *status;
                }
            }
        }
        Action::RebaseStatusUpdated(repo_index, pr_number, needs_rebase) => {
            // Update PR rebase status in repo_data
            if let Some(data) = state.repo_data.get_mut(repo_index) {
                if let Some(pr) = data.prs.iter_mut().find(|p| p.number == *pr_number) {
                    pr.needs_rebase = *needs_rebase;
                }
            }

            // Sync legacy fields if this is the selected repo
            if *repo_index == state.selected_repo {
                if let Some(pr) = state.prs.iter_mut().find(|p| p.number == *pr_number) {
                    pr.needs_rebase = *needs_rebase;
                }
            }
        }
        Action::MergeComplete(Ok(_)) => {
            // Clear selections after successful merge (only if not in merge bot)
            state.selected_prs.clear();
            if let Some(data) = state.repo_data.get_mut(&state.selected_repo) {
                data.selected_prs.clear();
            }
        }
        _ => {}
    }

    state
}

/// Log panel state reducer
fn log_panel_reducer(mut state: LogPanelState, action: &Action) -> LogPanelState {
    match action {
        Action::BuildLogsLoaded(sections, pr_context) => {
            state.panel = Some(crate::log::LogPanel {
                log_sections: sections.clone(),
                scroll_offset: 0,
                current_section: 0,
                horizontal_scroll: 0,
                pr_context: pr_context.clone(),
                show_timestamps: false,
            });
        }
        Action::CloseLogPanel => {
            state.panel = None;
        }
        Action::ScrollLogPanelUp => {
            if let Some(ref mut panel) = state.panel {
                panel.scroll_offset = panel.scroll_offset.saturating_sub(1);
            }
        }
        Action::ScrollLogPanelDown => {
            if let Some(ref mut panel) = state.panel {
                panel.scroll_offset = panel.scroll_offset.saturating_add(1);
            }
        }
        Action::ScrollLogPanelLeft => {
            if let Some(ref mut panel) = state.panel {
                panel.horizontal_scroll = panel.horizontal_scroll.saturating_sub(1);
            }
        }
        Action::ScrollLogPanelRight => {
            if let Some(ref mut panel) = state.panel {
                panel.horizontal_scroll = panel.horizontal_scroll.saturating_add(1);
            }
        }
        Action::NextLogSection => {
            if let Some(ref mut panel) = state.panel {
                if panel.current_section < panel.log_sections.len().saturating_sub(1) {
                    panel.current_section += 1;
                    panel.scroll_offset = 0;
                }
            }
        }
        Action::ToggleTimestamps => {
            if let Some(ref mut panel) = state.panel {
                panel.show_timestamps = !panel.show_timestamps;
            }
        }
        _ => {}
    }

    state
}

/// Merge bot state reducer
fn merge_bot_reducer(mut state: MergeBotState, action: &Action) -> MergeBotState {
    match action {
        Action::StartMergeBot => {
            // Note: actual bot starting logic with PR data happens in the effect handler
            // This just ensures the state is ready
        }
        Action::MergeStatusUpdated(_repo_index, pr_number, status) => {
            if state.bot.is_running() {
                state.bot.handle_status_update(*pr_number, *status);
            }
        }
        Action::RebaseComplete(result) => {
            if state.bot.is_running() {
                state.bot.handle_rebase_complete(result.is_ok());
            }
        }
        Action::MergeComplete(result) => {
            if state.bot.is_running() {
                state.bot.handle_merge_complete(result.is_ok());
            }
        }
        Action::PRMergedConfirmed(_repo_index, pr_number, is_merged) => {
            if state.bot.is_running() {
                state.bot.handle_pr_merged_confirmed(*pr_number, *is_merged);
            }
        }
        _ => {}
    }

    state
}

/// Task status reducer
fn task_reducer(mut state: TaskState, action: &Action) -> TaskState {
    match action {
        Action::RefreshCurrentRepo => {
            state.status = Some(TaskStatus {
                message: "Refreshing...".to_string(),
                status_type: TaskStatusType::Running,
            });
        }
        Action::RebaseComplete(result) => {
            state.status = Some(match result {
                Ok(_) => TaskStatus {
                    message: "Rebase completed successfully".to_string(),
                    status_type: TaskStatusType::Success,
                },
                Err(err) => TaskStatus {
                    message: format!("Rebase failed: {}", err),
                    status_type: TaskStatusType::Error,
                },
            });
        }
        Action::MergeComplete(result) => {
            state.status = Some(match result {
                Ok(_) => TaskStatus {
                    message: "Merge completed successfully".to_string(),
                    status_type: TaskStatusType::Success,
                },
                Err(err) => TaskStatus {
                    message: format!("Merge failed: {}", err),
                    status_type: TaskStatusType::Error,
                },
            });
        }
        Action::IDEOpenComplete(result) => {
            state.status = Some(match result {
                Ok(_) => TaskStatus {
                    message: "IDE opened successfully".to_string(),
                    status_type: TaskStatusType::Success,
                },
                Err(err) => TaskStatus {
                    message: format!("Failed to open IDE: {}", err),
                    status_type: TaskStatusType::Error,
                },
            });
        }
        _ => {}
    }

    state
}
