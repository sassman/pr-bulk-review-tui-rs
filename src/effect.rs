/// Effect system for Redux architecture
/// Reducers return (State, Vec<Effect>) where Effects describe side effects to perform
/// The update() function executes these effects
// Import debug from the log crate using :: prefix
use ::log::debug;

use crate::{
    App, load_persisted_state, loading_recent_repos,
    log::PrContext,
    pr::Pr,
    shortcuts::{Action, BootstrapResult},
    state::{LoadingState, Repo, TaskStatus, TaskStatusType},
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
        repos: Vec<Repo>,
        filter: crate::state::PrFilter,
    },

    /// Trigger background task to load single repo
    LoadSingleRepo {
        repo_index: usize,
        repo: Repo,
        filter: crate::state::PrFilter,
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
    PerformRebase { repo: Repo, prs: Vec<Pr> },

    /// Perform merge operation
    PerformMerge { repo: Repo, prs: Vec<Pr> },

    /// Approve PRs with configured message
    ApprovePrs { repo: Repo, pr_numbers: Vec<usize>, approval_message: String },

    /// Open PR in browser
    OpenInBrowser { url: String },

    /// Open in IDE
    OpenInIDE { repo: Repo, pr_number: usize },

    /// Load build logs
    LoadBuildLogs { repo: Repo, pr: Pr },

    /// Start merge bot
    StartMergeBot { repo: Repo, prs: Vec<Pr> },

    /// Rerun failed CI jobs for PRs
    RerunFailedJobs { repo: Repo, pr_numbers: Vec<usize> },

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

    /// Add a new repository
    AddRepository(Repo),

    /// Save repositories to disk
    SaveRepositories(Vec<Repo>),

    /// Dispatch another action (for chaining)
    DispatchAction(crate::shortcuts::Action),

    /// Batch multiple effects
    Batch(Vec<Effect>),

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
                    let selected_repo = if let Ok(state) = load_persisted_state() {
                        if let Some(index) = repos.iter().position(|r| r == &state.selected_repo) {
                            index
                        } else {
                            0
                        }
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
            let num_repos = repos.len();
            follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Loading PRs from {} repositories...", num_repos),
                status_type: TaskStatusType::Running,
            })));

            let indices: Vec<usize> = (0..num_repos).collect();
            follow_up_actions.push(Action::SetReposLoading(indices));

            let _ = app.task_tx.send(BackgroundTask::LoadAllRepos {
                repos,
                filter,
                octocrab: app.octocrab()?,
            });
        }

        Effect::LoadSingleRepo {
            repo_index,
            repo,
            filter,
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
            });
        }

        Effect::CheckMergeStatus {
            repo_index,
            repo,
            pr_numbers,
        } => {
            // Trigger background merge status checks
            let _ = app.task_tx.send(BackgroundTask::CheckMergeStatus {
                repo_index,
                repo,
                pr_numbers,
                octocrab: app.octocrab()?,
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

        Effect::ApprovePrs { repo, pr_numbers, approval_message } => {
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

        Effect::OpenInBrowser { url } => {
            // Open URL in browser
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(&url).spawn();
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("cmd")
                .args(&["/C", "start", &url])
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
            // Start merge bot - update state directly
            let pr_data: Vec<(usize, usize)> = prs
                .iter()
                .enumerate()
                .map(|(idx, pr)| (pr.number, idx))
                .collect();

            app.store.state_mut().merge_bot.bot.start(pr_data);
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
            debug!("Starting {} monitoring for PR #{}", operation_name, pr_number);

            let _ = app.task_tx.send(BackgroundTask::MonitorOperation {
                repo_index,
                repo,
                pr_number,
                operation,
                octocrab: app.octocrab()?,
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
                // Add to repos list in state
                let mut new_repos = app.store.state().repos.recent_repos.clone();
                new_repos.push(repo.clone());
                let repo_index = new_repos.len() - 1;

                // Save to file
                if let Err(e) = crate::store_recent_repos(&new_repos) {
                    follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                        message: format!("Failed to save repository: {}", e),
                        status_type: TaskStatusType::Error,
                    })));
                    return Ok(follow_up_actions);
                }

                // Update state by mutating the store directly
                app.store.state_mut().repos.recent_repos = new_repos;

                // Initialize repo data for the new repo
                let data = app
                    .store
                    .state_mut()
                    .repos
                    .repo_data
                    .entry(repo_index)
                    .or_default();
                data.loading_state = LoadingState::Loading;

                // Show success message
                follow_up_actions.push(Action::SetTaskStatus(Some(TaskStatus {
                    message: format!("Added repository: {}/{}", repo.org, repo.repo),
                    status_type: TaskStatusType::Success,
                })));

                // Trigger loading PRs for the new repo
                let filter = app.store.state().repos.filter.clone();
                let _ = app.task_tx.send(BackgroundTask::LoadSingleRepo {
                    repo_index,
                    repo: repo.clone(),
                    filter,
                    octocrab: app.octocrab()?,
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

        Effect::LoadPersistedSession => {
            // This is handled as part of LoadRepositories
            // No-op here as it's done synchronously in that effect
        }
    }

    Ok(follow_up_actions)
}
