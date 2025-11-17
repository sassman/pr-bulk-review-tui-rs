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

    let (debug_console_state, debug_console_effects) = debug_console_reducer(state.debug_console, action);
    state.debug_console = debug_console_state;
    effects.extend(debug_console_effects);

    (state, effects)
}

/// UI state reducer - handles UI-related actions
fn ui_reducer(mut state: UiState, action: &Action) -> (UiState, Vec<Effect>) {
    match action {
        Action::Quit => {
            state.should_quit = true;
        }
        Action::TickSpinner => {
            // Increment spinner frame for animation (0-9 cycle)
            state.spinner_frame = (state.spinner_frame + 1) % 10;
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
        Action::ShowAddRepoPopup => {
            state.show_add_repo = true;
            state.add_repo_form = AddRepoForm::default();
        }
        Action::HideAddRepoPopup => {
            state.show_add_repo = false;
            state.add_repo_form = AddRepoForm::default();
        }
        Action::AddRepoFormInput(ch) => {
            // Handle paste detection for GitHub URLs
            let input_str = ch.to_string();
            if input_str.contains("github.com") || state.add_repo_form.org.contains("github.com") {
                // Likely a URL paste, try to parse it
                let url_text = format!("{}{}", state.add_repo_form.org, input_str);
                if let Some((org, repo, branch)) = parse_github_url(&url_text) {
                    state.add_repo_form.org = org;
                    state.add_repo_form.repo = repo;
                    state.add_repo_form.branch = branch;
                    return (state, vec![]);
                }
            }

            // Normal character input to current field
            match state.add_repo_form.focused_field {
                AddRepoField::Org => state.add_repo_form.org.push(*ch),
                AddRepoField::Repo => state.add_repo_form.repo.push(*ch),
                AddRepoField::Branch => state.add_repo_form.branch.push(*ch),
            }
        }
        Action::AddRepoFormBackspace => match state.add_repo_form.focused_field {
            AddRepoField::Org => {
                state.add_repo_form.org.pop();
            }
            AddRepoField::Repo => {
                state.add_repo_form.repo.pop();
            }
            AddRepoField::Branch => {
                state.add_repo_form.branch.pop();
            }
        },
        Action::AddRepoFormNextField => {
            state.add_repo_form.focused_field = match state.add_repo_form.focused_field {
                AddRepoField::Org => AddRepoField::Repo,
                AddRepoField::Repo => AddRepoField::Branch,
                AddRepoField::Branch => AddRepoField::Org,
            };
        }
        Action::AddRepoFormSubmit => {
            // Validate and add repository
            if !state.add_repo_form.org.is_empty() && !state.add_repo_form.repo.is_empty() {
                let branch = if state.add_repo_form.branch.is_empty() {
                    "main".to_string()
                } else {
                    state.add_repo_form.branch.clone()
                };

                let new_repo = crate::state::Repo {
                    org: state.add_repo_form.org.clone(),
                    repo: state.add_repo_form.repo.clone(),
                    branch,
                };

                // Return effect to add the repository
                let effects = vec![Effect::AddRepository(new_repo)];
                state.show_add_repo = false;
                state.add_repo_form = AddRepoForm::default();
                return (state, effects);
            }
        }
        _ => {}
    }

    (state, vec![])
}

/// Parse GitHub URL into (org, repo, branch)
/// Supports formats:
/// - https://github.com/org/repo
/// - https://github.com/org/repo.git
/// - https://github.com/org/repo/tree/branch
/// - github.com/org/repo
fn parse_github_url(url: &str) -> Option<(String, String, String)> {
    let url = url.trim();

    // Remove protocol if present
    let url = url.strip_prefix("https://").unwrap_or(url);
    let url = url.strip_prefix("http://").unwrap_or(url);

    // Remove github.com prefix
    let url = url
        .strip_prefix("github.com/")
        .or_else(|| url.strip_prefix("www.github.com/"))?;

    // Split by '/'
    let parts: Vec<&str> = url.split('/').collect();

    if parts.len() >= 2 {
        let org = parts[0].to_string();
        // Remove .git suffix if present
        let mut repo = parts[1].to_string();
        if repo.ends_with(".git") {
            repo = repo.strip_suffix(".git").unwrap().to_string();
        }

        let branch = if parts.len() >= 4 && parts[2] == "tree" {
            parts[3].to_string()
        } else {
            "main".to_string()
        };

        Some((org, repo, branch))
    } else {
        None
    }
}

/// Repository and PR state reducer
/// ALL logic lives here - reducer returns effects to be performed
fn repos_reducer(
    mut state: ReposState,
    action: &Action,
    config: &crate::config::Config,
) -> (ReposState, Vec<Effect>) {
    let mut effects = vec![];

    match action {
        // Bootstrap: Load repositories and session
        Action::Bootstrap => {
            state.bootstrap_state = BootstrapState::LoadingRepositories;
            // Effect: Load .env file if needed (checked by effect executor)
            effects.push(Effect::LoadEnvFile);
            // Effect: Initialize octocrab client (must happen after .env is loaded)
            effects.push(Effect::InitializeOctocrab);
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
        Action::RepoLoadingStarted(repo_index) => {
            // Mark repo as loading (request in flight)
            let data = state.repo_data.entry(*repo_index).or_default();
            data.loading_state = LoadingState::Loading;
        }
        Action::DeleteCurrentRepo => {
            // Delete the currently selected repository
            if !state.recent_repos.is_empty() {
                let selected_idx = state.selected_repo;

                // Remove the repo from the list
                state.recent_repos.remove(selected_idx);

                // Remove its data
                state.repo_data.remove(&selected_idx);

                // Rebuild repo_data with updated indices
                let mut new_repo_data = std::collections::HashMap::new();
                for (old_idx, data) in state.repo_data.iter() {
                    let new_idx = if *old_idx > selected_idx {
                        old_idx - 1
                    } else {
                        *old_idx
                    };
                    new_repo_data.insert(new_idx, data.clone());
                }
                state.repo_data = new_repo_data;

                // Adjust selected repo index
                if state.recent_repos.is_empty() {
                    state.selected_repo = 0;
                    state.prs.clear();
                    state.loading_state = LoadingState::Idle;
                    state.state.select(None);
                } else if selected_idx >= state.recent_repos.len() {
                    // Was last repo, select the new last one
                    state.selected_repo = state.recent_repos.len() - 1;
                    // Sync legacy fields with new selection
                    if let Some(data) = state.repo_data.get(&state.selected_repo) {
                        state.prs = data.prs.clone();
                        state.state = data.table_state.clone();
                        state.loading_state = data.loading_state.clone();
                    }
                } else {
                    // Sync legacy fields with current selection
                    if let Some(data) = state.repo_data.get(&state.selected_repo) {
                        state.prs = data.prs.clone();
                        state.state = data.table_state.clone();
                        state.loading_state = data.loading_state.clone();
                    }
                }

                // Effect: Save updated repository list to file
                effects.push(Effect::SaveRepositories(state.recent_repos.clone()));

                // Show status message
                effects.push(Effect::DispatchAction(Action::SetTaskStatus(Some(
                    crate::state::TaskStatus {
                        message: "Repository deleted".to_string(),
                        status_type: crate::state::TaskStatusType::Success,
                    }
                ))));
            }
        }
        Action::SelectRepoByIndex(index) => {
            if *index < state.recent_repos.len() {
                state.selected_repo = *index;

                // Sync legacy fields with repo_data
                if let Some(data) = state.repo_data.get(index) {
                    state.prs = data.prs.clone();
                    state.state = data.table_state.clone();
                    state.loading_state = data.loading_state.clone();
                }
            }
        }
        Action::RepoDataLoaded(repo_index, Ok(prs)) => {
            let data = state.repo_data.entry(*repo_index).or_default();
            data.prs = prs.clone();
            data.loading_state = LoadingState::Loaded;

            // Update table selection based on PR list
            if data.prs.is_empty() {
                // Clear selection and selected PRs when no PRs
                data.table_state.select(None);
                data.selected_pr_numbers.clear();
            } else if data.table_state.selected().is_none() {
                // Select first row if nothing selected
                data.table_state.select(Some(0));
            }

            // Validate selected_pr_numbers - remove PRs that no longer exist
            // This is critical after filtering or when PRs are closed/merged
            let current_pr_numbers: std::collections::HashSet<_> =
                data.prs.iter().map(|pr| PrNumber::from_pr(pr)).collect();
            data.selected_pr_numbers.retain(|num| current_pr_numbers.contains(num));

            // Sync legacy fields if this is the selected repo
            if *repo_index == state.selected_repo {
                state.prs = prs.clone();
                state.state = data.table_state.clone();
                state.loading_state = LoadingState::Loaded;
            }

            // Effect: Check merge status for loaded PRs
            if let Some(repo) = state.recent_repos.get(*repo_index).cloned() {
                let pr_numbers: Vec<usize> = prs.iter().map(|pr| pr.number).collect();
                effects.push(Effect::CheckMergeStatus {
                    repo_index: *repo_index,
                    repo: repo.clone(),
                    pr_numbers: pr_numbers.clone(),
                });
                // Effect: Check comment counts for loaded PRs
                effects.push(Effect::CheckCommentCounts {
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
                    Effect::DispatchAction(Action::SetBootstrapState(BootstrapState::Completed)),
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

            // Reload current repository with new filter
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                effects.push(Effect::LoadSingleRepo {
                    repo_index: state.selected_repo,
                    repo,
                    filter: state.filter.clone(),
                });
            }
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
                if selected < state.prs.len() {
                    let pr_number = PrNumber::from_pr(&state.prs[selected]);

                    // Update type-safe PR number-based selection (stable across filtering)
                    if let Some(data) = state.repo_data.get_mut(&state.selected_repo) {
                        if data.selected_pr_numbers.contains(&pr_number) {
                            data.selected_pr_numbers.remove(&pr_number);
                        } else {
                            data.selected_pr_numbers.insert(pr_number);
                        }
                    }

                    // Automatically advance to next PR if not on the last row
                    if selected < state.prs.len().saturating_sub(1) {
                        effects.push(Effect::DispatchAction(Action::NavigateToNextPr));
                    }
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

            // If status is BuildInProgress, start monitoring the build
            if *status == crate::pr::MergeableStatus::BuildInProgress {
                if let Some(repo) = state.recent_repos.get(*repo_index).cloned() {
                    // First dispatch action to update state immediately
                    effects.push(Effect::DispatchAction(Action::StartOperationMonitor(
                        *repo_index,
                        *pr_number,
                        crate::state::OperationType::Rebase,
                    )));
                    // Then start background monitoring
                    effects.push(Effect::StartOperationMonitoring {
                        repo_index: *repo_index,
                        repo,
                        pr_number: *pr_number,
                        operation: crate::state::OperationType::Rebase,
                    });
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
        Action::CommentCountUpdated(repo_index, pr_number, comment_count) => {
            // Update PR comment count in repo_data
            if let Some(data) = state.repo_data.get_mut(repo_index) {
                if let Some(pr) = data.prs.iter_mut().find(|p| p.number == *pr_number) {
                    pr.no_comments = *comment_count;
                }
            }

            // Sync legacy fields if this is the selected repo
            if *repo_index == state.selected_repo {
                if let Some(pr) = state.prs.iter_mut().find(|p| p.number == *pr_number) {
                    pr.no_comments = *comment_count;
                }
            }
        }
        Action::MergeComplete(Ok(_)) => {
            // Clear selections after successful merge (only if not in merge bot)
            if let Some(data) = state.repo_data.get_mut(&state.selected_repo) {
                data.selected_pr_numbers.clear();
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
        Action::ReloadRepo(repo_index) => {
            // Effect: Reload specific repository (e.g., after PR merged)
            if let Some(repo) = state.recent_repos.get(*repo_index).cloned() {
                effects.push(Effect::LoadSingleRepo {
                    repo_index: *repo_index,
                    repo,
                    filter: state.filter.clone(),
                });
            }
        }
        Action::Rebase => {
            // Effect: Perform rebase on selected PRs, or current PR if none selected
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                // Use PR numbers for stable selection
                let has_selection = if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    !data.selected_pr_numbers.is_empty()
                } else {
                    false
                };

                let prs_to_rebase: Vec<_> = if !has_selection {
                    // No selection - use current cursor PR
                    state
                        .state
                        .selected()
                        .and_then(|idx| state.prs.get(idx).cloned())
                        .map(|pr| vec![pr])
                        .unwrap_or_default()
                } else if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    // Rebase selected PRs using PR numbers (stable across filtering)
                    state
                        .prs
                        .iter()
                        .filter(|pr| data.selected_pr_numbers.contains(&PrNumber::from_pr(pr)))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };

                if !prs_to_rebase.is_empty() {
                    // Start monitoring for each PR being rebased
                    let repo_index = state.selected_repo;
                    for pr in &prs_to_rebase {
                        // First dispatch action to update state immediately
                        effects.push(Effect::DispatchAction(Action::StartOperationMonitor(
                            repo_index,
                            pr.number,
                            crate::state::OperationType::Rebase,
                        )));
                        // Then start background monitoring
                        effects.push(Effect::StartOperationMonitoring {
                            repo_index,
                            repo: repo.clone(),
                            pr_number: pr.number,
                            operation: crate::state::OperationType::Rebase,
                        });
                    }

                    effects.push(Effect::PerformRebase {
                        repo,
                        prs: prs_to_rebase,
                    });

                    // Clear selection after starting rebase (if there was a selection)
                    if has_selection {
                        if let Some(data) = state.repo_data.get_mut(&state.selected_repo) {
                            data.selected_pr_numbers.clear();
                        }
                    }
                }
            }
        }
        Action::RerunFailedJobs => {
            // Effect: Rerun failed CI jobs for current or selected PRs
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                // Use PR numbers for stable selection
                let has_selection = if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    !data.selected_pr_numbers.is_empty()
                } else {
                    false
                };

                let pr_numbers: Vec<usize> = if !has_selection {
                    // Rerun for current PR only
                    state
                        .state
                        .selected()
                        .and_then(|idx| state.prs.get(idx))
                        .map(|pr| vec![pr.number])
                        .unwrap_or_default()
                } else if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    // Rerun for selected PRs using PR numbers (stable)
                    state
                        .prs
                        .iter()
                        .filter(|pr| data.selected_pr_numbers.contains(&PrNumber::from_pr(pr)))
                        .map(|pr| pr.number)
                        .collect()
                } else {
                    Vec::new()
                };

                if !pr_numbers.is_empty() {
                    effects.push(Effect::RerunFailedJobs { repo, pr_numbers });
                }
            }
        }
        Action::ApprovePrs => {
            // Effect: Approve selected PRs or current PR with configured message
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                // Use PR numbers for stable selection
                let has_selection = if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    !data.selected_pr_numbers.is_empty()
                } else {
                    false
                };

                let pr_numbers: Vec<usize> = if !has_selection {
                    // No selection - use current cursor PR
                    state
                        .state
                        .selected()
                        .and_then(|idx| state.prs.get(idx))
                        .map(|pr| vec![pr.number])
                        .unwrap_or_default()
                } else if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    state
                        .prs
                        .iter()
                        .filter(|pr| data.selected_pr_numbers.contains(&PrNumber::from_pr(pr)))
                        .map(|pr| pr.number)
                        .collect()
                } else {
                    Vec::new()
                };

                if !pr_numbers.is_empty() {
                    effects.push(Effect::ApprovePrs {
                        repo,
                        pr_numbers,
                        approval_message: config.approval_message.clone(),
                    });
                }
            }
        }
        Action::MergeSelectedPrs => {
            // Effect: Merge selected PRs or current PR, or enable auto-merge if building
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                // Use PR numbers for stable selection
                let has_selection = if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    !data.selected_pr_numbers.is_empty()
                } else {
                    false
                };

                let selected_prs: Vec<_> = if !has_selection {
                    // No selection - use current cursor PR
                    state
                        .state
                        .selected()
                        .and_then(|idx| state.prs.get(idx).cloned())
                        .map(|pr| vec![pr])
                        .unwrap_or_default()
                } else if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    state
                        .prs
                        .iter()
                        .filter(|pr| data.selected_pr_numbers.contains(&PrNumber::from_pr(pr)))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };

                if !selected_prs.is_empty() {
                    // Separate PRs by status: ready to merge vs building
                    let mut prs_to_merge = Vec::new();
                    let mut prs_to_auto_merge = Vec::new();

                    for pr in selected_prs {
                        match pr.mergeable {
                            crate::pr::MergeableStatus::BuildInProgress => {
                                prs_to_auto_merge.push(pr);
                            }
                            _ => {
                                prs_to_merge.push(pr);
                            }
                        }
                    }

                    // Merge ready PRs directly
                    if !prs_to_merge.is_empty() {
                        // Start monitoring for each PR being merged
                        let repo_index = state.selected_repo;
                        for pr in &prs_to_merge {
                            // First dispatch action to update state immediately
                            effects.push(Effect::DispatchAction(Action::StartOperationMonitor(
                                repo_index,
                                pr.number,
                                crate::state::OperationType::Merge,
                            )));
                            // Then start background monitoring
                            effects.push(Effect::StartOperationMonitoring {
                                repo_index,
                                repo: repo.clone(),
                                pr_number: pr.number,
                                operation: crate::state::OperationType::Merge,
                            });
                        }

                        effects.push(Effect::PerformMerge {
                            repo: repo.clone(),
                            prs: prs_to_merge,
                        });
                    }

                    // Enable auto-merge for building PRs
                    for pr in prs_to_auto_merge {
                        effects.push(Effect::EnableAutoMerge {
                            repo_index: state.selected_repo,
                            repo: repo.clone(),
                            pr_number: pr.number,
                        });
                    }

                    // Clear selection after starting merge operations (if there was a selection)
                    if has_selection {
                        if let Some(data) = state.repo_data.get_mut(&state.selected_repo) {
                            data.selected_pr_numbers.clear();
                        }
                    }
                }
            }
        }
        Action::StartMergeBot => {
            // Effect: Start merge bot with selected PRs
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                // Use PR numbers for stable selection
                let prs_to_process: Vec<_> = if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    state
                        .prs
                        .iter()
                        .filter(|pr| data.selected_pr_numbers.contains(&PrNumber::from_pr(pr)))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };

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
                // If multiple PRs selected, open all of them using PR numbers (stable)
                let has_selection = if let Some(data) = state.repo_data.get(&state.selected_repo) {
                    !data.selected_pr_numbers.is_empty()
                } else {
                    false
                };

                let prs_to_open: Vec<usize> = if has_selection {
                    if let Some(data) = state.repo_data.get(&state.selected_repo) {
                        state
                            .prs
                            .iter()
                            .filter(|pr| data.selected_pr_numbers.contains(&PrNumber::from_pr(pr)))
                            .map(|pr| pr.number)
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else if let Some(selected_idx) = state.state.selected() {
                    // Open just the current PR
                    state
                        .prs
                        .get(selected_idx)
                        .map(|pr| vec![pr.number])
                        .unwrap_or_default()
                } else {
                    vec![]
                };

                for pr_number in prs_to_open {
                    let url = format!(
                        "https://github.com/{}/{}/pull/{}",
                        repo.org, repo.repo, pr_number
                    );
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
            // Effect: Open current PR in IDE, or main branch if no PR selected
            if let Some(repo) = state.recent_repos.get(state.selected_repo).cloned() {
                if let Some(selected_idx) = state.state.selected() {
                    if let Some(pr) = state.prs.get(selected_idx) {
                        // Open the selected PR
                        effects.push(Effect::OpenInIDE {
                            repo,
                            pr_number: pr.number,
                        });
                    }
                } else {
                    // No PR selected (empty list) - open main branch
                    // Use pr_number = 0 as a special marker for main branch
                    effects.push(Effect::OpenInIDE { repo, pr_number: 0 });
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
                    state.loading_state = data.loading_state.clone();
                }
            }
        }
        Action::StartOperationMonitor(repo_index, pr_number, operation) => {
            // Add PR to operation monitor queue and set initial state
            if let Some(data) = state.repo_data.get_mut(repo_index) {
                // Check if already in queue
                if !data
                    .operation_monitor_queue
                    .iter()
                    .any(|op| op.pr_number == *pr_number)
                {
                    // Set the PR status to Rebasing or Merging immediately
                    let status = match operation {
                        crate::state::OperationType::Rebase => {
                            crate::pr::MergeableStatus::Rebasing
                        }
                        crate::state::OperationType::Merge => {
                            crate::pr::MergeableStatus::Merging
                        }
                    };

                    // Update PR status in repo_data
                    if let Some(pr) = data.prs.iter_mut().find(|p| p.number == *pr_number) {
                        pr.mergeable = status;
                    }

                    // Also sync to legacy fields if this is the selected repo
                    if *repo_index == state.selected_repo {
                        if let Some(pr) = state.prs.iter_mut().find(|p| p.number == *pr_number) {
                            pr.mergeable = status;
                        }
                    }

                    // Add to monitoring queue
                    data.operation_monitor_queue
                        .push(crate::state::OperationMonitor {
                            pr_number: *pr_number,
                            operation: *operation,
                            started_at: std::time::Instant::now(),
                            check_count: 0,
                            last_head_sha: None,
                        });
                }
            }
        }
        Action::RemoveFromOperationMonitor(repo_index, pr_number) => {
            // Remove PR from operation monitor queue
            if let Some(data) = state.repo_data.get_mut(repo_index) {
                data.operation_monitor_queue
                    .retain(|op| op.pr_number != *pr_number);
            }
        }
        Action::OperationMonitorCheck(repo_index, pr_number) => {
            // Periodic status check for operation monitor
            // This will be handled by the background task which will dispatch
            // MergeStatusUpdated actions based on GitHub API responses
            // For now, just increment check count
            if let Some(data) = state.repo_data.get_mut(repo_index) {
                if let Some(monitor) = data
                    .operation_monitor_queue
                    .iter_mut()
                    .find(|op| op.pr_number == *pr_number)
                {
                    monitor.check_count += 1;

                    // Timeout after 40 checks (20 minutes at 30s intervals)
                    if monitor.check_count >= 40 {
                        // Remove from queue - timeout reached
                        data.operation_monitor_queue
                            .retain(|op| op.pr_number != *pr_number);
                        effects.push(Effect::DispatchAction(Action::SetTaskStatus(Some(
                            crate::state::TaskStatus {
                                message: format!(
                                    "Operation monitor timeout for PR #{}",
                                    pr_number
                                ),
                                status_type: crate::state::TaskStatusType::Error,
                            },
                        ))));
                    }
                }
            }
        }
        Action::AddToAutoMergeQueue(repo_index, pr_number) => {
            // Add PR to auto-merge queue
            if let Some(data) = state.repo_data.get_mut(repo_index) {
                // Check if already in queue
                if !data
                    .auto_merge_queue
                    .iter()
                    .any(|pr| pr.pr_number == *pr_number)
                {
                    data.auto_merge_queue.push(crate::state::AutoMergePR {
                        pr_number: *pr_number,
                        started_at: std::time::Instant::now(),
                        check_count: 0,
                    });
                }
            }
        }
        Action::RemoveFromAutoMergeQueue(repo_index, pr_number) => {
            // Remove PR from auto-merge queue
            if let Some(data) = state.repo_data.get_mut(repo_index) {
                data.auto_merge_queue
                    .retain(|pr| pr.pr_number != *pr_number);
            }
        }
        Action::AutoMergeStatusCheck(repo_index, pr_number) => {
            // Periodic status check for auto-merge PR
            if let Some(data) = state.repo_data.get_mut(repo_index) {
                if let Some(auto_pr) = data
                    .auto_merge_queue
                    .iter_mut()
                    .find(|pr| pr.pr_number == *pr_number)
                {
                    auto_pr.check_count += 1;

                    // Check if we've exceeded the time limit (20 minutes = 20 checks at 1 min intervals)
                    if auto_pr.check_count >= 20 {
                        // Remove from queue - timeout reached
                        data.auto_merge_queue
                            .retain(|pr| pr.pr_number != *pr_number);
                        effects.push(Effect::DispatchAction(Action::SetTaskStatus(Some(
                            crate::state::TaskStatus {
                                message: format!("Auto-merge timeout for PR #{}", pr_number),
                                status_type: crate::state::TaskStatusType::Error,
                            },
                        ))));
                    } else {
                        // Check PR status
                        if let Some(repo) = state.recent_repos.get(*repo_index).cloned() {
                            // Find the PR to check its status
                            if let Some(pr) = data.prs.iter().find(|p| p.number == *pr_number) {
                                match pr.mergeable {
                                    crate::pr::MergeableStatus::Ready => {
                                        // PR is ready - trigger merge
                                        effects.push(Effect::PerformMerge {
                                            repo: repo.clone(),
                                            prs: vec![pr.clone()],
                                        });
                                        // Remove from queue
                                        data.auto_merge_queue.retain(|p| p.pr_number != *pr_number);
                                    }
                                    crate::pr::MergeableStatus::BuildFailed => {
                                        // Build failed - stop monitoring
                                        data.auto_merge_queue.retain(|p| p.pr_number != *pr_number);
                                        effects.push(Effect::DispatchAction(
                                            Action::SetTaskStatus(Some(crate::state::TaskStatus {
                                                message: format!(
                                                    "Auto-merge stopped: PR #{} build failed",
                                                    pr_number
                                                ),
                                                status_type: crate::state::TaskStatusType::Error,
                                            })),
                                        ));
                                    }
                                    crate::pr::MergeableStatus::NeedsRebase => {
                                        // Needs rebase - stop monitoring
                                        data.auto_merge_queue.retain(|p| p.pr_number != *pr_number);
                                        effects.push(Effect::DispatchAction(
                                            Action::SetTaskStatus(Some(crate::state::TaskStatus {
                                                message: format!(
                                                    "Auto-merge stopped: PR #{} needs rebase",
                                                    pr_number
                                                ),
                                                status_type: crate::state::TaskStatusType::Error,
                                            })),
                                        ));
                                    }
                                    crate::pr::MergeableStatus::BuildInProgress => {
                                        // Still building - schedule next check
                                        // This will be handled by the background task
                                    }
                                    _ => {
                                        // Unknown or conflicted - continue monitoring
                                    }
                                }
                            }
                        }
                    }
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
        Action::BuildLogsLoaded(jobs, pr_context) => {
            // Create master-detail log panel from job logs
            state.panel = Some(crate::log::create_log_panel_from_jobs(
                jobs.clone(),
                pr_context.clone(),
            ));
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
        Action::PageLogPanelDown => {
            if let Some(ref mut panel) = state.panel {
                // Page down by viewport_height - 1 (keep one line of context)
                let page_size = panel.viewport_height.saturating_sub(1).max(1);
                panel.scroll_offset = panel.scroll_offset.saturating_add(page_size);
            }
        }
        Action::ScrollLogPanelLeft => {
            if let Some(ref mut panel) = state.panel {
                // Scroll left by 5 characters for better UX
                panel.horizontal_scroll = panel.horizontal_scroll.saturating_sub(5);
            }
        }
        Action::ScrollLogPanelRight => {
            if let Some(ref mut panel) = state.panel {
                // Scroll right by 5 characters for better UX
                panel.horizontal_scroll = panel.horizontal_scroll.saturating_add(5);
            }
        }
        Action::NextLogSection => {
            if let Some(ref mut panel) = state.panel {
                panel.find_next_error();
            }
        }
        Action::PrevLogSection => {
            if let Some(ref mut panel) = state.panel {
                // Find previous error - move up until we find a node with errors
                // For now, just navigate up
                panel.navigate_up();
            }
        }
        Action::ToggleTimestamps => {
            if let Some(ref mut panel) = state.panel {
                panel.show_timestamps = !panel.show_timestamps;
            }
        }
        Action::UpdateLogPanelViewport(height) => {
            if let Some(ref mut panel) = state.panel {
                panel.viewport_height = *height;
            }
        }
        // Tree navigation
        Action::SelectNextJob => {
            if let Some(ref mut panel) = state.panel {
                panel.navigate_down();
            }
        }
        Action::SelectPrevJob => {
            if let Some(ref mut panel) = state.panel {
                panel.navigate_up();
            }
        }
        Action::ToggleTreeNode => {
            if let Some(ref mut panel) = state.panel {
                panel.toggle_at_cursor();
            }
        }
        Action::FocusJobList => {
            // No-op in tree view - unified view has no separate focus
        }
        Action::FocusLogViewer => {
            // No-op in tree view - unified view has no separate focus
        }
        // Step navigation
        Action::NextStep => {
            if let Some(ref mut panel) = state.panel {
                panel.navigate_down();
            }
        }
        Action::PrevStep => {
            if let Some(ref mut panel) = state.panel {
                panel.navigate_up();
            }
        }
        // Error navigation
        Action::NextError => {
            if let Some(ref mut panel) = state.panel {
                panel.find_next_error();
            }
        }
        Action::PrevError => {
            if let Some(ref mut panel) = state.panel {
                panel.find_prev_error();
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
        Action::ApprovalComplete(result) => {
            state.status = Some(match result {
                Ok(_) => TaskStatus {
                    message: "PR(s) approved successfully".to_string(),
                    status_type: TaskStatusType::Success,
                },
                Err(err) => TaskStatus {
                    message: format!("Failed to approve PR(s): {}", err),
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

/// Debug console state reducer - handles debug console actions
fn debug_console_reducer(mut state: DebugConsoleState, action: &Action) -> (DebugConsoleState, Vec<Effect>) {
    match action {
        Action::ToggleDebugConsole => {
            state.is_open = !state.is_open;
            // Reset scroll when opening
            if state.is_open {
                state.scroll_offset = 0;
            }
        }
        Action::ScrollDebugConsoleUp => {
            state.scroll_offset = state.scroll_offset.saturating_sub(1);
            // Disable auto-scroll when manually scrolling
            state.auto_scroll = false;
        }
        Action::ScrollDebugConsoleDown => {
            state.scroll_offset = state.scroll_offset.saturating_add(1);
            // Disable auto-scroll when manually scrolling
            state.auto_scroll = false;
        }
        Action::PageDebugConsoleDown => {
            // Page down by viewport_height - 1 (keep one line of context)
            let page_size = state.viewport_height.saturating_sub(1).max(1);
            state.scroll_offset = state.scroll_offset.saturating_add(page_size);
            // Disable auto-scroll when manually scrolling
            state.auto_scroll = false;
        }
        Action::ToggleDebugAutoScroll => {
            state.auto_scroll = !state.auto_scroll;
        }
        Action::ClearDebugLogs => {
            if let Ok(mut logs) = state.logs.lock() {
                logs.clear();
            }
            state.scroll_offset = 0;
        }
        Action::UpdateDebugConsoleViewport(height) => {
            state.viewport_height = *height;
        }
        _ => {}
    }

    (state, vec![])
}
