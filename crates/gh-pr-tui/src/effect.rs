/// Effect system for Redux architecture
/// Reducers return (State, Vec<Effect>) where Effects describe side effects to perform
/// The update() function executes these effects
// Import debug from the log crate using :: prefix
use ::log::debug;

use crate::{
    App,
    actions::{Action, BootstrapResult},
    load_persisted_state, loading_recent_repos,
    log::PrContext,
    pr::Pr,
    state::{Repo, TaskStatus, TaskStatusType},
    task::BackgroundTask,
};
use anyhow::Result;
use octocrab::Octocrab;
use std::env;

/// Effects that reducers can request to be performed
#[derive(Debug, Clone)]
pub enum Effect {
    /// Load .env file if GITHUB_TOKEN is not set
    LoadEnvFile,

    /// Initialize Octocrab client (must happen after LoadEnvFile)
    InitializeOctocrab,

    /// Load repositories from config file
    LoadRepositories,

    /// Load persisted session state
    LoadPersistedSession,

    /// Trigger background task to load all repos
    LoadAllRepos {
        repos: Vec<(usize, Repo)>, // (repo_index, repo) pairs
        filter: crate::state::PrFilter,
    },

    /// Trigger background task to load single repo
    LoadSingleRepo {
        repo_index: usize,
        repo: Repo,
        filter: crate::state::PrFilter,
        bypass_cache: bool, // True for user-triggered refresh, false for lazy loading
    },

    /// Trigger delayed repo reload (waits before reloading)
    DelayedRepoReload {
        repo_index: usize,
        delay_ms: u64,
    },

    /// Trigger background merge status checks
    CheckMergeStatus {
        repo_index: usize,
        repo: Repo,
        pr_numbers: Vec<usize>,
    },

    /// Trigger background rebase checks
    CheckRebaseStatus {
        repo_index: usize,
        repo: Repo,
        pr_numbers: Vec<usize>,
    },

    /// Trigger background comment count checks
    CheckCommentCounts {
        repo_index: usize,
        repo: Repo,
        pr_numbers: Vec<usize>,
    },

    /// Perform rebase operation
    PerformRebase {
        repo: Repo,
        prs: Vec<Pr>,
    },

    /// Perform merge operation
    PerformMerge {
        repo: Repo,
        prs: Vec<Pr>,
    },

    /// Approve PRs with configured message
    ApprovePrs {
        repo: Repo,
        pr_numbers: Vec<usize>,
        approval_message: String,
    },

    /// Close PRs with comment
    ClosePrs {
        comment: String,
    },

    /// Open PR in browser
    OpenInBrowser {
        url: String,
    },

    /// Open in IDE
    OpenInIDE {
        repo: Repo,
        pr_number: usize,
    },

    /// Load build logs
    LoadBuildLogs {
        repo: Repo,
        pr: Pr,
    },

    /// Start merge bot
    StartMergeBot {
        repo: Repo,
        prs: Vec<Pr>,
    },

    /// Rerun failed CI jobs for PRs
    RerunFailedJobs {
        repo: Repo,
        pr_numbers: Vec<usize>,
    },

    /// Enable auto-merge on PR and monitor until ready
    EnableAutoMerge {
        repo_index: usize,
        repo: Repo,
        pr_number: usize,
    },

    /// Start monitoring an operation (rebase/merge) for a PR
    StartOperationMonitoring {
        repo_index: usize,
        repo: Repo,
        pr_number: usize,
        operation: crate::state::OperationType,
    },

    /// Poll PR merge status (for merge bot)
    PollPRMergeStatus {
        repo_index: usize,
        repo: Repo,
        pr_number: usize,
        is_checking_ci: bool,
    },

    /// Add a new repository
    AddRepository(Repo),

    /// Save repositories to disk
    SaveRepositories(Vec<Repo>),

    /// Dispatch another action (for chaining)
    DispatchAction(crate::actions::Action),

    /// Batch multiple effects
    Batch(Vec<Effect>),

    /// Update command palette filtered commands based on current input
    UpdateCommandPaletteFilter,

    /// Cache management effects
    ClearCache,
    ShowCacheStats,
    InvalidateRepoCache(usize), // Invalidate cache for specific repo index

    /// No effect
    None,
}

impl Effect {
    /// Create a batch of effects
    pub fn batch(effects: Vec<Effect>) -> Self {
        Effect::Batch(effects)
    }

    /// Create no effect
    pub fn none() -> Self {
        Effect::None
    }
}
/// Execute an effect and return follow-up actions to dispatch
/// This maintains clean architecture by avoiding direct action dispatching from effects
pub async fn execute_effect(app: &mut App, effect: Effect) -> Result<Vec<Action>> {
    use crate::effect::Effect;

    let mut follow_up_actions = Vec::new();

    match effect {
        Effect::None => {}

        Effect::LoadEnvFile => {
            // Load .env file if GITHUB_TOKEN is not already set
            if std::env::var("GITHUB_TOKEN").is_err() {
                // Try to load .env file from current directory or parent directories
                match dotenvy::dotenv() {
                    Ok(path) => {
                        debug!("Loaded .env file from: {:?}", path);
                    }
                    Err(_) => {
                        // .env file not found or couldn't be loaded - not an error
                        // User might have GITHUB_TOKEN set via other means
                        debug!(".env file not found, will rely on environment variables");
                    }
                }
            }
        }

        Effect::InitializeOctocrab => {
            // Initialize octocrab client with GITHUB_TOKEN
            // This happens after LoadEnvFile, ensuring token is available
            match env::var("GITHUB_TOKEN") {
                Ok(token) => match Octocrab::builder().personal_token(token).build() {
                    Ok(client) => {
                        app.octocrab = Some(client);
                        debug!("Octocrab client initialized successfully");
                    }
                    Err(e) => {
                        debug!("Failed to initialize octocrab: {}", e);
                        follow_up_actions.push(Action::BootstrapComplete(Err(format!(
                            "Failed to initialize GitHub client: {}",
                            e
                        ))));
                        return Ok(follow_up_actions);
                    }
                },
                Err(_) => {
                    follow_up_actions.push(Action::BootstrapComplete(Err(
                        "GITHUB_TOKEN environment variable not set. Please set it or create a .env file.".to_string()
                    )));
                    return Ok(follow_up_actions);
                }
            }
        }

        Effect::LoadRepositories => {
            // Load repositories from config file
            match loading_recent_repos() {
                Ok(repos) => {
                    if repos.is_empty() {
                        follow_up_actions.push(Action::BootstrapComplete(Err(
                            "No repositories configured. Add repositories to .recent-repositories.json".to_string()
                        )));
                        return Ok(follow_up_actions);
                    }

                    // Restore session
                    let selected_repo: usize = if let Ok(state) = load_persisted_state() {
                        repos
                            .iter()
                            .position(|r| r == &state.selected_repo)
                            .unwrap_or_default()
                    } else {
                        0
                    };

                    // Return bootstrap complete action
                    let result = BootstrapResult {
                        repos,
                        selected_repo,
                    };
                    follow_up_actions.push(Action::BootstrapComplete(Ok(result)));
                }
                Err(err) => {
                    follow_up_actions.push(Action::BootstrapComplete(Err(err.to_string())));
                }
            }
        }

        Effect::LoadAllRepos { repos, filter } => {
            // Trigger background task to load all repos
            // Extract the repo indices from the (index, repo) pairs
            let indices: Vec<usize> = repos.iter().map(|(i, _)| *i).collect();
            follow_up_actions.push(Action::SetReposLoading(indices));

            // Don't show "Loading PRs from X repositories..." if we're in background loading mode
            // (individual repo status messages will be shown instead)

            let _ = app.task_tx.send(BackgroundTask::LoadAllRepos {
                repos,
                filter,
                octocrab: app.octocrab()?,
                cache: app.cache.clone(),
            });
        }

        Effect::LoadSingleRepo {
            repo_index,
            repo,
            filter,
            bypass_cache,
        } => {
            // Trigger background task to load single repo
            follow_up_actions.push(Action::SetReposLoading(vec![repo_index]));
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message: "Refreshing...".to_string(),
                status_type: TaskStatusType::Running,
            })));

            let _ = app.task_tx.send(BackgroundTask::LoadSingleRepo {
                repo_index,
                repo,
                filter,
                octocrab: app.octocrab()?,
                cache: app.cache.clone(),
                bypass_cache,
            });
        }

        Effect::DelayedRepoReload {
            repo_index,
            delay_ms,
        } => {
            // Trigger delayed repo reload using DelayedTask wrapper
            // Delayed reload is typically after operations (merge/rebase), so bypass cache
            if let Some(repo) = app
                .store
                .state()
                .repos
                .recent_repos
                .get(repo_index)
                .cloned()
            {
                let filter = app.store.state().repos.filter.clone();
                let _ = app.task_tx.send(BackgroundTask::DelayedTask {
                    task: Box::new(BackgroundTask::LoadSingleRepo {
                        repo_index,
                        repo,
                        filter,
                        octocrab: app.octocrab()?,
                        cache: app.cache.clone(),
                        bypass_cache: true, // Get fresh data after operations
                    }),
                    delay_ms,
                });
            }
        }

        Effect::CheckMergeStatus {
            repo_index,
            repo,
            pr_numbers,
        } => {
            // Trigger background merge status checks
            let _ = app.task_tx.send(BackgroundTask::DelayedTask {
                task: Box::new(BackgroundTask::CheckMergeStatus {
                    repo_index,
                    repo,
                    pr_numbers,
                    octocrab: app.octocrab()?,
                }),
                delay_ms: 500,
            });
        }

        Effect::CheckRebaseStatus { .. } => {
            // Note: CheckRebaseStatus is checked as part of CheckMergeStatus
            // This effect exists for future extensibility
            // For now, no-op as rebase status is determined from merge status
        }

        Effect::CheckCommentCounts {
            repo_index,
            repo,
            pr_numbers,
        } => {
            // Trigger background comment count checks
            let _ = app.task_tx.send(BackgroundTask::CheckCommentCounts {
                repo_index,
                repo,
                pr_numbers,
                octocrab: app.octocrab()?,
            });
        }

        Effect::PerformRebase { repo, prs } => {
            // Perform rebase operation
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Rebasing {} PR(s)...", prs.len()),
                status_type: TaskStatusType::Running,
            })));

            let selected_indices: Vec<usize> = (0..prs.len()).collect();
            let _ = app.task_tx.send(BackgroundTask::Rebase {
                repo,
                prs,
                selected_indices,
                octocrab: app.octocrab()?,
            });
        }

        Effect::PerformMerge { repo, prs } => {
            // Perform merge operation
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Merging {} PR(s)...", prs.len()),
                status_type: TaskStatusType::Running,
            })));

            let selected_indices: Vec<usize> = (0..prs.len()).collect();
            let _ = app.task_tx.send(BackgroundTask::Merge {
                repo,
                prs,
                selected_indices,
                octocrab: app.octocrab()?,
            });
        }

        Effect::ApprovePrs {
            repo,
            pr_numbers,
            approval_message,
        } => {
            // Approve PRs with configured message
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Approving {} PR(s)...", pr_numbers.len()),
                status_type: TaskStatusType::Running,
            })));

            let _ = app.task_tx.send(BackgroundTask::ApprovePrs {
                repo,
                pr_numbers,
                approval_message,
                octocrab: app.octocrab()?,
            });
        }

        Effect::ClosePrs { comment } => {
            // Close selected PRs with comment
            let state = app.store.state();
            let repo_index = state.repos.selected_repo;

            if let Some(repo) = state.repos.recent_repos.get(repo_index).cloned() {
                // Get selected PRs or current PR
                let has_selection = if let Some(data) = state.repos.repo_data.get(&repo_index) {
                    !data.selected_pr_numbers.is_empty()
                } else {
                    false
                };

                let (pr_numbers, prs): (Vec<usize>, Vec<crate::pr::Pr>) = if !has_selection {
                    // No selection - use current cursor PR
                    state
                        .repos
                        .state
                        .selected()
                        .and_then(|idx| state.repos.prs.get(idx).cloned())
                        .map(|pr| (vec![pr.number], vec![pr]))
                        .unwrap_or((vec![], vec![]))
                } else if let Some(data) = state.repos.repo_data.get(&repo_index) {
                    let selected_prs: Vec<_> = state
                        .repos
                        .prs
                        .iter()
                        .filter(|pr| {
                            data.selected_pr_numbers
                                .contains(&crate::state::PrNumber::from_pr(pr))
                        })
                        .cloned()
                        .collect();
                    let pr_numbers = selected_prs.iter().map(|pr| pr.number).collect();
                    (pr_numbers, selected_prs)
                } else {
                    (vec![], vec![])
                };

                if !pr_numbers.is_empty() {
                    follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                        message: format!("Closing {} PR(s)...", pr_numbers.len()),
                        status_type: TaskStatusType::Running,
                    })));

                    let _ = app.task_tx.send(BackgroundTask::ClosePrs {
                        repo,
                        pr_numbers,
                        prs,
                        comment,
                        octocrab: app.octocrab()?,
                    });
                }
            }
        }

        Effect::OpenInBrowser { url } => {
            // Open URL in browser
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(&url).spawn();
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", &url])
                .spawn();
        }

        Effect::OpenInIDE { repo, pr_number } => {
            // Open PR or main branch in IDE
            let message = if pr_number == 0 {
                "Opening main branch in IDE...".to_string()
            } else {
                format!("Opening PR #{} in IDE...", pr_number)
            };
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message,
                status_type: TaskStatusType::Running,
            })));

            let config = app.store.state().config.clone();
            let _ = app.task_tx.send(BackgroundTask::OpenPRInIDE {
                repo,
                pr_number,
                ide_command: config.ide_command,
                temp_dir: config.temp_dir,
            });
        }

        Effect::LoadBuildLogs { repo, pr } => {
            // Load build logs
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message: "Loading build logs...".to_string(),
                status_type: TaskStatusType::Running,
            })));

            let pr_context = PrContext {
                number: pr.number,
                title: pr.title.clone(),
                author: pr.author.clone(),
            };

            let _ = app.task_tx.send(BackgroundTask::FetchBuildLogs {
                repo,
                pr_number: pr.number,
                head_sha: "HEAD".to_string(), // Placeholder - will fetch in background task
                octocrab: app.octocrab()?,
                pr_context,
            });
        }

        Effect::StartMergeBot { prs, .. } => {
            // Start merge bot - dispatch action to reducer
            let pr_data: Vec<(usize, usize)> = prs
                .iter()
                .enumerate()
                .map(|(idx, pr)| (pr.number, idx))
                .collect();

            // Dispatch action to initialize bot (reducer handles state mutation)
            follow_up_actions.push(Action::StartMergeBotWithPrData(pr_data));
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Merge bot started with {} PR(s)", prs.len()),
                status_type: TaskStatusType::Success,
            })));
        }

        Effect::RerunFailedJobs { repo, pr_numbers } => {
            // Rerun failed CI jobs for PRs
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Rerunning failed CI jobs for {} PR(s)...", pr_numbers.len()),
                status_type: TaskStatusType::Running,
            })));

            let _ = app.task_tx.send(BackgroundTask::RerunFailedJobs {
                repo,
                pr_numbers,
                octocrab: app.octocrab()?,
            });
        }

        Effect::EnableAutoMerge {
            repo_index,
            repo,
            pr_number,
        } => {
            // Add PR to auto-merge queue
            follow_up_actions.push(Action::AddToAutoMergeQueue(repo_index, pr_number));

            // Show status message
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Enabling auto-merge for PR #{}...", pr_number),
                status_type: TaskStatusType::Running,
            })));

            // Send background task to enable auto-merge on GitHub
            let _ = app.task_tx.send(BackgroundTask::EnableAutoMerge {
                repo_index,
                repo,
                pr_number,
                octocrab: app.octocrab()?,
            });
        }

        Effect::StartOperationMonitoring {
            repo_index,
            repo,
            pr_number,
            operation,
        } => {
            // Send background task to monitor the operation
            let operation_name = match operation {
                crate::state::OperationType::Rebase => "Rebase",
                crate::state::OperationType::Merge => "Merge",
            };
            debug!(
                "Starting {} monitoring for PR #{}",
                operation_name, pr_number
            );

            let _ = app.task_tx.send(BackgroundTask::MonitorOperation {
                repo_index,
                repo,
                pr_number,
                operation,
                octocrab: app.octocrab()?,
            });
        }

        Effect::PollPRMergeStatus {
            repo_index,
            repo,
            pr_number,
            is_checking_ci,
        } => {
            // Poll PR to check if it's merged (for merge bot)
            let _ = app.task_tx.send(BackgroundTask::PollPRMergeStatus {
                repo_index,
                repo,
                pr_number,
                octocrab: app.octocrab()?,
                is_checking_ci,
            });
        }

        Effect::AddRepository(repo) => {
            // Check if repository already exists
            let repo_exists = app
                .store
                .state()
                .repos
                .recent_repos
                .iter()
                .any(|r| r.org == repo.org && r.repo == repo.repo && r.branch == repo.branch);

            if !repo_exists {
                // Calculate new repo index
                let repo_index = app.store.state().repos.recent_repos.len();

                // Build new repos list for saving
                let mut new_repos = app.store.state().repos.recent_repos.clone();
                new_repos.push(repo.clone());

                // Save to file first (effect side effect)
                if let Err(e) = crate::store_recent_repos(&new_repos) {
                    follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                        message: format!("Failed to save repository: {}", e),
                        status_type: TaskStatusType::Error,
                    })));
                    return Ok(follow_up_actions);
                }

                // Dispatch action to update state (reducer handles mutation)
                follow_up_actions.push(Action::RepositoryAdded {
                    repo_index,
                    repo: repo.clone(),
                });

                // Show success message
                follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                    message: format!("Added repository: {}/{}", repo.org, repo.repo),
                    status_type: TaskStatusType::Success,
                })));

                // Trigger loading PRs for the new repo (use cache for initial load)
                let filter = app.store.state().repos.filter.clone();
                let _ = app.task_tx.send(BackgroundTask::LoadSingleRepo {
                    repo_index,
                    repo: repo.clone(),
                    filter,
                    octocrab: app.octocrab()?,
                    cache: app.cache.clone(),
                    bypass_cache: false, // Use cache for initial load of newly added repo
                });
            } else {
                // Repository already exists
                follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                    message: format!("Repository {}/{} already exists", repo.org, repo.repo),
                    status_type: TaskStatusType::Error,
                })));
            }
        }

        Effect::SaveRepositories(repos) => {
            // Save repositories to disk
            if let Err(e) = crate::store_recent_repos(&repos) {
                follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                    message: format!("Failed to save repositories: {}", e),
                    status_type: TaskStatusType::Error,
                })));
            }
        }

        Effect::DispatchAction(action) => {
            // Return action for dispatching (for chaining)
            follow_up_actions.push(action);
        }

        Effect::Batch(effects) => {
            // Execute a batch of effects and collect all follow-up actions
            for effect in effects {
                let actions = Box::pin(execute_effect(app, effect)).await?;
                follow_up_actions.extend(actions);
            }
        }

        Effect::ClearCache => {
            let mut cache = app.cache.lock().unwrap();
            match cache.clear() {
                Ok(_) => {
                    follow_up_actions.push(crate::actions::Action::SetTaskStatus(Some(
                        crate::state::TaskStatus {
                            message: "Cache cleared successfully".to_string(),
                            status_type: crate::state::TaskStatusType::Success,
                        },
                    )));
                }
                Err(e) => {
                    follow_up_actions.push(crate::actions::Action::SetTaskStatus(Some(
                        crate::state::TaskStatus {
                            message: format!("Failed to clear cache: {}", e),
                            status_type: crate::state::TaskStatusType::Error,
                        },
                    )));
                }
            }
        }

        Effect::ShowCacheStats => {
            let cache = app.cache.lock().unwrap();
            let stats = cache.stats();
            let message = format!(
                "Cache: {} total ({} fresh, {} stale) | TTL: {}min",
                stats.total_entries,
                stats.fresh_entries,
                stats.stale_entries,
                stats.ttl_seconds / 60
            );
            follow_up_actions.push(crate::actions::Action::SetTaskStatus(Some(
                crate::state::TaskStatus {
                    message,
                    status_type: crate::state::TaskStatusType::Success,
                },
            )));
        }

        Effect::InvalidateRepoCache(repo_index) => {
            if let Some(repo) = app.store.state().repos.recent_repos.get(repo_index) {
                let mut cache = app.cache.lock().unwrap();
                let pattern = format!("/repos/{}/{}", repo.org, repo.repo);
                cache.invalidate_pattern(&pattern);
                follow_up_actions.push(crate::actions::Action::SetTaskStatus(Some(
                    crate::state::TaskStatus {
                        message: format!("Cache invalidated for {}/{}", repo.org, repo.repo),
                        status_type: crate::state::TaskStatusType::Success,
                    },
                )));
            }
        }

        Effect::UpdateCommandPaletteFilter => {
            // Filter commands based on current input
            if let Some(palette_state) = &app.store.state().ui.command_palette {
                use crate::command_palette_integration::ShortcutCommandProvider;
                use gh_pr_tui_command_palette::{CommandPalette, filter_commands};

                // Create command palette with providers
                let mut palette = CommandPalette::new();
                palette.register(Box::new(ShortcutCommandProvider));

                // Get all available commands for current state
                let all_commands = palette.all_commands(app.store.state());

                // Filter commands based on user input
                let filtered = filter_commands(&all_commands, &palette_state.input);

                // Update state with filtered results (dispatch action)
                use crate::actions::Action;
                follow_up_actions.push(Action::UpdateCommandPaletteResults(filtered));
            }
        }

        Effect::LoadPersistedSession => {
            // This is handled as part of LoadRepositories
            // No-op here as it's done synchronously in that effect
        }
    }

    Ok(follow_up_actions)
}
