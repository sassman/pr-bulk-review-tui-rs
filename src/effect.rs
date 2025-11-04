/// Effect system for Redux architecture
/// Reducers return (State, Vec<Effect>) where Effects describe side effects to perform
/// The update() function executes these effects

// Import debug from the log crate using :: prefix
use ::log::debug;

use crate::{
    loading_recent_repos, load_persisted_state,
    log::PrContext,
    pr::Pr,
    shortcuts::{Action, BootstrapResult},
    state::{Repo, TaskStatus, TaskStatusType},
    task::BackgroundTask,
    App,
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
pub async fn execute_effect(app: &mut App, effect: Effect) -> Result<()> {
    use crate::effect::Effect;

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
                        let _ = app.action_tx.send(Action::BootstrapComplete(Err(format!(
                            "Failed to initialize GitHub client: {}",
                            e
                        ))));
                        return Ok(());
                    }
                },
                Err(_) => {
                    let _ = app.action_tx.send(Action::BootstrapComplete(Err(
                        "GITHUB_TOKEN environment variable not set. Please set it or create a .env file.".to_string()
                    )));
                    return Ok(());
                }
            }
        }

        Effect::LoadRepositories => {
            // Load repositories from config file
            match loading_recent_repos() {
                Ok(repos) => {
                    if repos.is_empty() {
                        let _ = app.action_tx.send(Action::BootstrapComplete(Err(
                            "No repositories configured. Add repositories to .recent-repositories.json".to_string()
                        )));
                        return Ok(());
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

                    // Dispatch bootstrap complete
                    let result = BootstrapResult {
                        repos,
                        selected_repo,
                    };
                    let _ = app.action_tx.send(Action::BootstrapComplete(Ok(result)));
                }
                Err(err) => {
                    let _ = app
                        .action_tx
                        .send(Action::BootstrapComplete(Err(err.to_string())));
                }
            }
        }

        Effect::LoadAllRepos { repos, filter } => {
            // Trigger background task to load all repos
            let num_repos = repos.len();
            let _ = app.action_tx.send(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Loading PRs from {} repositories...", num_repos),
                status_type: TaskStatusType::Running,
            })));

            let indices: Vec<usize> = (0..num_repos).collect();
            let _ = app.action_tx.send(Action::SetReposLoading(indices));

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
            let _ = app
                .action_tx
                .send(Action::SetReposLoading(vec![repo_index]));
            let _ = app.action_tx.send(Action::SetTaskStatus(Some(TaskStatus {
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

        Effect::PerformRebase { repo, prs } => {
            // Perform rebase operation
            let _ = app.action_tx.send(Action::SetTaskStatus(Some(TaskStatus {
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
            let _ = app.action_tx.send(Action::SetTaskStatus(Some(TaskStatus {
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
            // Open PR in IDE
            let _ = app.action_tx.send(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Opening PR #{} in IDE...", pr_number),
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
            let _ = app.action_tx.send(Action::SetTaskStatus(Some(TaskStatus {
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
            let _ = app.action_tx.send(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Merge bot started with {} PR(s)", prs.len()),
                status_type: TaskStatusType::Success,
            })));
        }

        Effect::RerunFailedJobs { repo, pr_numbers } => {
            // Rerun failed CI jobs for PRs
            let _ = app.action_tx.send(Action::SetTaskStatus(Some(TaskStatus {
                message: format!("Rerunning failed CI jobs for {} PR(s)...", pr_numbers.len()),
                status_type: TaskStatusType::Running,
            })));

            let _ = app.task_tx.send(BackgroundTask::RerunFailedJobs {
                repo,
                pr_numbers,
                octocrab: app.octocrab()?,
            });
        }

        Effect::DispatchAction(action) => {
            // Dispatch another action (for chaining)
            let _ = app.action_tx.send(action);
        }

        Effect::Batch(effects) => {
            // Execute a batch of effects
            for effect in effects {
                Box::pin(execute_effect(app, effect)).await?;
            }
        }

        Effect::LoadPersistedSession => {
            // This is handled as part of LoadRepositories
            // No-op here as it's done synchronously in that effect
        }
    }

    Ok(())
}

