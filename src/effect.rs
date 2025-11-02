/// Effect system for Redux architecture
/// Reducers return (State, Vec<Effect>) where Effects describe side effects to perform
/// The update() function executes these effects

use crate::{pr::Pr, state::Repo};
use octocrab::Octocrab;

/// Effects that reducers can request to be performed
#[derive(Debug, Clone)]
pub enum Effect {
    /// Load .env file if GITHUB_TOKEN is not set
    LoadEnvFile,

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
