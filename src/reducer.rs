use crate::{effect::Effect, shortcuts::Action, state::*};

/// Root reducer that delegates to sub-reducers based on action type
/// Pure function: takes state and action, returns (new state, effects to perform)
pub fn reduce(mut state: AppState, action: &Action) -> (AppState, Vec<Effect>) {
    let mut effects = Vec::new();

    // Apply each sub-reducer and collect effects
    let (ui_state, ui_effects) = ui_reducer(state.ui, action);
    state.ui = ui_state;
    effects.extend(ui_effects);

    let (repos_state, repos_effects) = repos_reducer(state.repos, action, &state.config);
    state.repos = repos_state;
    effects.extend(repos_effects);

    let (log_panel_state, log_panel_effects) = log_panel_reducer(state.log_panel, action);
    state.log_panel = log_panel_state;
    effects.extend(log_panel_effects);

    let (merge_bot_state, merge_bot_effects) = merge_bot_reducer(state.merge_bot, action);
    state.merge_bot = merge_bot_state;
    effects.extend(merge_bot_effects);

    let (task_state, task_effects) = task_reducer(state.task, action);
    state.task = task_state;
    effects.extend(task_effects);

    (state, effects)
}

/// UI state reducer - handles UI-related actions
fn ui_reducer(mut state: UiState, action: &Action) -> (UiState, Vec<Effect>) {
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

    (state, vec![])
}

/// Repository and PR state reducer
/// ALL logic lives here - reducer returns effects to be performed
fn repos_reducer(
    mut state: ReposState,
    action: &Action,
    _config: &crate::config::Config,
) -> (ReposState, Vec<Effect>) {
    let mut effects = vec![];

    match action {
        // Bootstrap: Load repositories and session
        Action::Bootstrap => {
            state.bootstrap_state = BootstrapState::LoadingRepositories;
            // Effect: Load .env file if needed (checked by effect executor)
            effects.push(Effect::LoadEnvFile);
            // Effect: Load repositories from config file
            effects.push(Effect::LoadRepositories);
        }

        // Internal state update actions
        Action::SetBootstrapState(new_state) => {
            state.bootstrap_state = new_state.clone();
        }
        Action::SetLoadingState(new_state) => {
            state.loading_state = new_state.clone();
        }
        Action::SetReposLoading(indices) => {
            for &index in indices {
                let data = state.repo_data.entry(index).or_default();
                data.loading_state = LoadingState::Loading;
            }
        }

        // Repositories loaded - restore session and load PRs
        Action::BootstrapComplete(Ok(result)) => {
            state.recent_repos = result.repos.clone();
            state.selected_repo = result.selected_repo;
            state.bootstrap_state = BootstrapState::LoadingPRs;

            // Set all repos to loading
            for i in 0..result.repos.len() {
                let data = state.repo_data.entry(i).or_default();
                data.loading_state = LoadingState::Loading;
            }

            // Effect: Load PRs for all repos
            effects.push(Effect::LoadAllRepos {
                repos: result.repos.clone(),
                filter: state.filter.clone(),
            });
        }
        Action::BootstrapComplete(Err(err)) => {
            state.bootstrap_state = BootstrapState::Error(err.clone());
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
            if data.table_state.selected().is_none() && !data.prs.is_empty() {
                data.table_state.select(Some(0));
            }

            // Sync legacy fields if this is the selected repo
            if *repo_index == state.selected_repo {
                state.prs = prs.clone();
                state.loading_state = LoadingState::Loaded;
            }

            // Effect: Check merge status for loaded PRs
            if let Some(repo) = state.recent_repos.get(*repo_index).cloned() {
                let pr_numbers: Vec<usize> = prs.iter().map(|pr| pr.number).collect();
                effects.push(Effect::CheckMergeStatus {
                    repo_index: *repo_index,
                    repo,
                    pr_numbers,
                });
            }

            // Check if all repos are done loading
            let all_loaded = state.repo_data.len() == state.recent_repos.len()
                && state.repo_data.values().all(|d| {
                    matches!(
                        d.loading_state,
                        LoadingState::Loaded | LoadingState::Error(_)
                    )
                });

            // Effect: Dispatch bootstrap completion
            if all_loaded && state.bootstrap_state == BootstrapState::LoadingPRs {
                effects.push(Effect::batch(vec![
                    Effect::DispatchAction(Action::SetBootstrapState(
                        BootstrapState::Completed,
                    )),
                    Effect::DispatchAction(Action::SetTaskStatus(Some(TaskStatus {
                        message: "All repositories loaded successfully".to_string(),
                        status_type: TaskStatusType::Success,
                    }))),
                ]));
            }
        }
        Action::RepoDataLoaded(repo_index, Err(err)) => {
            let data = state.repo_data.entry(*repo_index).or_default();
            data.loading_state = LoadingState::Error(err.clone());
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
        Action::RefreshCurrentRepo => {
            // Effect: Reload current repository
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                effects.push(Effect::LoadSingleRepo {
                    repo_index: state.selected_repo,
                    repo,
                    filter: state.filter.clone(),
                });
            }
        }
        Action::Rebase => {
            // Effect: Perform rebase on selected PRs (or auto-rebase if none selected)
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                let prs_to_rebase: Vec<_> = if state.selected_prs.is_empty() {
                    // Auto-rebase: find first PR that needs rebase
                    state.prs.iter().filter(|pr| pr.needs_rebase).take(1).cloned().collect()
                } else {
                    // Rebase selected PRs
                    state.selected_prs.iter()
                        .filter_map(|&idx| state.prs.get(idx).cloned())
                        .collect()
                };

                if !prs_to_rebase.is_empty() {
                    effects.push(Effect::PerformRebase {
                        repo,
                        prs: prs_to_rebase,
                    });
                }
            }
        }
        Action::RerunFailedJobs => {
            // Effect: Rerun failed CI jobs for current or selected PRs
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                let pr_numbers: Vec<usize> = if state.selected_prs.is_empty() {
                    // Rerun for current PR only
                    state.state.selected()
                        .and_then(|idx| state.prs.get(idx))
                        .map(|pr| vec![pr.number])
                        .unwrap_or_default()
                } else {
                    // Rerun for all selected PRs
                    state.selected_prs.iter()
                        .filter_map(|&idx| state.prs.get(idx).map(|pr| pr.number))
                        .collect()
                };

                if !pr_numbers.is_empty() {
                    effects.push(Effect::RerunFailedJobs {
                        repo,
                        pr_numbers,
                    });
                }
            }
        }
        Action::MergeSelectedPrs => {
            // Effect: Merge selected PRs
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                let prs_to_merge: Vec<_> = state.selected_prs.iter()
                    .filter_map(|&idx| state.prs.get(idx).cloned())
                    .collect();

                if !prs_to_merge.is_empty() {
                    effects.push(Effect::PerformMerge {
                        repo,
                        prs: prs_to_merge,
                    });
                }
            }
        }
        Action::StartMergeBot => {
            // Effect: Start merge bot with selected PRs
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                let prs_to_process: Vec<_> = state.selected_prs.iter()
                    .filter_map(|&idx| state.prs.get(idx).cloned())
                    .collect();

                if !prs_to_process.is_empty() {
                    effects.push(Effect::StartMergeBot {
                        repo,
                        prs: prs_to_process,
                    });
                }
            }
        }
        Action::OpenCurrentPrInBrowser => {
            // Effect: Open current PR(s) in browser
            if let Some(repo) = state.recent_repos.get(state.selected_repo) {
                // If multiple PRs selected, open all of them
                let prs_to_open: Vec<usize> = if !state.selected_prs.is_empty() {
                    state.selected_prs.iter()
                        .filter_map(|&idx| state.prs.get(idx).map(|pr| pr.number))
                        .collect()
                } else if let Some(selected_idx) = state.state.selected() {
                    // Open just the current PR
                    state.prs.get(selected_idx).map(|pr| vec![pr.number]).unwrap_or_default()
                } else {
                    vec![]
                };

                for pr_number in prs_to_open {
                    let url = format!("https://github.com/{}/{}/pull/{}", repo.org, repo.repo, pr_number);
                    effects.push(Effect::OpenInBrowser { url });
                }
            }
        }
        Action::OpenBuildLogs => {
            // Effect: Load build logs for current PR
            if let Some(selected_idx) = state.state.selected() {
                if let Some(pr) = state.prs.get(selected_idx).cloned() {
                    if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                        effects.push(Effect::LoadBuildLogs { repo, pr });
                    }
                }
            }
        }
        Action::OpenInIDE => {
            // Effect: Open current PR in IDE
            if let Some(selected_idx) = state.state.selected() {
                if let Some(pr) = state.prs.get(selected_idx) {
                    if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                        effects.push(Effect::OpenInIDE {
                            repo,
                            pr_number: pr.number,
                        });
                    }
                }
            }
        }
        Action::SelectNextRepo => {
            if !state.recent_repos.is_empty() {
                state.selected_repo = (state.selected_repo + 1) % state.recent_repos.len();

                // Sync legacy fields with repo_data
                if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    state.prs = data.prs.clone();
                    state.state = data.table_state.clone();
                    state.selected_prs = data.selected_prs.clone();
                    state.loading_state = data.loading_state.clone();
                }
            }
        }
        Action::SelectPreviousRepo => {
            if !state.recent_repos.is_empty() {
                state.selected_repo = if state.selected_repo == 0 {
                    state.recent_repos.len() - 1
                } else {
                    state.selected_repo - 1
                };

                // Sync legacy fields with repo_data
                if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    state.prs = data.prs.clone();
                    state.state = data.table_state.clone();
                    state.selected_prs = data.selected_prs.clone();
                    state.loading_state = data.loading_state.clone();
                }
            }
        }
        _ => {}
    }

    (state, effects)
}

/// Log panel state reducer
fn log_panel_reducer(mut state: LogPanelState, action: &Action) -> (LogPanelState, Vec<Effect>) {
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

    (state, vec![])
}

/// Merge bot state reducer
fn merge_bot_reducer(mut state: MergeBotState, action: &Action) -> (MergeBotState, Vec<Effect>) {
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

    (state, vec![])
}

/// Task status reducer
fn task_reducer(mut state: TaskState, action: &Action) -> (TaskState, Vec<Effect>) {
    match action {
        // Internal state update action
        Action::SetTaskStatus(new_status) => {
            state.status = new_status.clone();
        }

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
        Action::RerunJobsComplete(result) => {
            state.status = Some(match result {
                Ok(_) => TaskStatus {
                    message: "CI jobs rerun successfully".to_string(),
                    status_type: TaskStatusType::Success,
                },
                Err(err) => TaskStatus {
                    message: format!("Failed to rerun CI jobs: {}", err),
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

    (state, vec![])
}
