use anyhow::{Context, Result, bail};
use log::debug;
use octocrab::{Octocrab, params};
use ratatui::{
    crossterm::{
        self,
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    },
    prelude::*,
    style::palette::tailwind,
    widgets::*,
};
use serde::{Deserialize, Serialize};
use std::{env, fs::File, io::BufReader};
use tokio::sync::mpsc;

use crate::gh::{comment, merge};
use crate::pr::Pr;

mod gh;
mod pr;

const PALETTES: [tailwind::Palette; 4] = [
    tailwind::BLUE,
    tailwind::EMERALD,
    tailwind::INDIGO,
    tailwind::RED,
];

struct TableColors {
    buffer_bg: Color,
    header_bg: Color,
    header_fg: Color,
    row_fg: Color,
    selected_row_style_fg: Color,
    selected_column_style_fg: Color,
    selected_cell_style_fg: Color,
    normal_row_color: Color,
    alt_row_color: Color,
    footer_border_color: Color,
}

impl TableColors {
    const fn new(color: &tailwind::Palette) -> Self {
        Self {
            buffer_bg: tailwind::SLATE.c950,
            header_bg: color.c900,
            header_fg: tailwind::SLATE.c200,
            row_fg: tailwind::SLATE.c200,
            selected_row_style_fg: color.c400,
            selected_column_style_fg: color.c400,
            selected_cell_style_fg: color.c600,
            normal_row_color: tailwind::SLATE.c950,
            alt_row_color: tailwind::SLATE.c900,
            footer_border_color: color.c400,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum LoadingState {
    #[default]
    Idle,
    Loading,
    Loaded,
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum BootstrapState {
    #[default]
    NotStarted,
    LoadingRepositories,
    RestoringSession,
    LoadingPRs,
    Completed,
    Error(String),
}

struct App {
    action_tx: mpsc::UnboundedSender<Action>,
    task_tx: mpsc::UnboundedSender<BackgroundTask>,
    should_quit: bool,
    state: TableState,
    prs: Vec<Pr>,
    recent_repos: Vec<Repo>,
    selected_repo: usize,
    filter: PrFilter,
    selected_prs: Vec<usize>,
    colors: TableColors,
    loading_state: LoadingState,
    bootstrap_state: BootstrapState,
    // Tabbed view: store PRs and state for each repo
    repo_data: std::collections::HashMap<usize, RepoData>,
    // Background task status
    task_status: Option<TaskStatus>,
}

#[derive(Debug, Clone)]
struct TaskStatus {
    message: String,
    status_type: TaskStatusType,
}

#[derive(Debug, Clone, PartialEq)]
enum TaskStatusType {
    Running,   // ⏳ In progress
    Success,   // ✓ Completed successfully
    Error,     // ✗ Failed
}

#[derive(Debug, Clone, Default)]
struct RepoData {
    prs: Vec<Pr>,
    table_state: TableState,
    selected_prs: Vec<usize>,
    loading_state: LoadingState,
}

#[derive(Debug, Serialize, Deserialize, Eq, Clone, PartialEq)]
struct PersistedState {
    selected_repo: Repo,
}

#[derive(Debug, Serialize, Deserialize, Eq, Clone, PartialEq)]
struct Repo {
    org: String,
    repo: String,
    branch: String,
}

#[derive(Debug, Serialize, Deserialize, Eq, Clone, PartialEq)]
enum PrFilter {
    None,      // Show all PRs
    Feat,      // Show only "feat:" PRs
    Fix,       // Show only "fix:" PRs
    Chore,     // Show only "chore:" PRs
}

impl PrFilter {
    fn matches(&self, title: &str) -> bool {
        match self {
            PrFilter::None => true,
            PrFilter::Feat => title.to_lowercase().contains("feat"),
            PrFilter::Fix => title.to_lowercase().contains("fix"),
            PrFilter::Chore => title.to_lowercase().contains("chore"),
        }
    }

    fn label(&self) -> &str {
        match self {
            PrFilter::None => "All",
            PrFilter::Feat => "Feat",
            PrFilter::Fix => "Fix",
            PrFilter::Chore => "Chore",
        }
    }

    fn next(&self) -> Self {
        match self {
            PrFilter::None => PrFilter::Feat,
            PrFilter::Feat => PrFilter::Fix,
            PrFilter::Fix => PrFilter::Chore,
            PrFilter::Chore => PrFilter::None,
        }
    }
}

impl Repo {
    fn new(org: &str, repo: &str, branch: &str) -> Repo {
        Repo {
            org: org.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
        }
    }
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
enum Action {
    Bootstrap,
    Rebase,
    RefreshCurrentRepo,
    CycleFilter,
    SelectNextRepo,
    SelectPreviousRepo,
    SelectRepoByIndex(usize),
    TogglePrSelection,
    NavigateToNextPr,
    NavigateToPreviousPr,
    MergeSelectedPrs,
    OpenCurrentPrInBrowser,

    // Background task completion notifications
    BootstrapComplete(Result<BootstrapResult, String>),
    RepoDataLoaded(usize, Result<Vec<Pr>, String>),
    RefreshComplete(Result<Vec<Pr>, String>),
    MergeStatusUpdated(usize, usize, crate::pr::MergeableStatus), // repo_index, pr_number, status
    RebaseStatusUpdated(usize, usize, bool), // repo_index, pr_number, needs_rebase
    RebaseComplete(Result<(), String>),
    MergeComplete(Result<(), String>),

    Quit,
    None,
}

// Result types for background tasks
struct BootstrapResult {
    repos: Vec<Repo>,
    selected_repo: usize,
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
}

async fn update(app: &mut App, msg: Action) -> Result<Action> {
    match msg {
        Action::Quit => app.should_quit = true,

        Action::Bootstrap => {
            // Stage 1: Loading repositories (fast, synchronous)
            app.bootstrap_state = BootstrapState::LoadingRepositories;

            match loading_recent_repos() {
                Ok(repos) => {
                    app.recent_repos = repos;

                    if app.recent_repos.is_empty() {
                        app.bootstrap_state = BootstrapState::Error(
                            "No repositories configured. Add repositories to .recent-repositories.json".to_string()
                        );
                        return Ok(Action::None);
                    }
                }
                Err(err) => {
                    app.bootstrap_state = BootstrapState::Error(format!("Failed to load repositories: {}", err));
                    return Ok(Action::None);
                }
            }

            // Stage 2: Restoring session (fast, synchronous)
            app.bootstrap_state = BootstrapState::RestoringSession;

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
                app.selected_repo = 0;
            }

            // Stage 3: Loading PRs (slow, move to background)
            app.bootstrap_state = BootstrapState::LoadingPRs;

            // Set status
            app.task_status = Some(TaskStatus {
                message: format!("Loading PRs from {} repositories...", app.recent_repos.len()),
                status_type: TaskStatusType::Running,
            });

            // Set all repos to loading state
            for i in 0..app.recent_repos.len() {
                let data = app.repo_data.entry(i).or_default();
                data.loading_state = LoadingState::Loading;
            }

            // Trigger background loading
            let _ = app.task_tx.send(BackgroundTask::LoadAllRepos {
                repos: app.recent_repos.clone(),
                filter: app.filter.clone(),
                octocrab: app.octocrab()?,
            });
        }

        Action::RepoDataLoaded(index, result) => {
            // Handle completion of background repo loading
            let data = app.repo_data.entry(index).or_default();
            match result {
                Ok(prs) => {
                    data.prs = prs.clone();
                    data.loading_state = LoadingState::Loaded;
                    if data.table_state.selected().is_none() && !data.prs.is_empty() {
                        data.table_state.select(Some(0));
                    }

                    // Trigger background merge status checks for this repo
                    if let Some(repo) = app.recent_repos.get(index).cloned() {
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
            if index == app.selected_repo {
                app.load_repo_state();
            }

            // Check if all repos are done loading
            let all_loaded = app.repo_data.len() == app.recent_repos.len()
                && app.repo_data.values().all(|d| {
                    matches!(d.loading_state, LoadingState::Loaded | LoadingState::Error(_))
                });

            if all_loaded && app.bootstrap_state == BootstrapState::LoadingPRs {
                app.bootstrap_state = BootstrapState::Completed;
                // Clear loading status and show success
                app.task_status = Some(TaskStatus {
                    message: "All repositories loaded successfully".to_string(),
                    status_type: TaskStatusType::Success,
                });
            }
        }

        Action::MergeStatusUpdated(repo_index, pr_number, status) => {
            // Update the merge status for a specific PR
            if let Some(data) = app.repo_data.get_mut(&repo_index) {
                if let Some(pr) = data.prs.iter_mut().find(|pr| pr.number == pr_number) {
                    pr.mergeable = status;
                }
            }

            // If this is the current repo, update app.prs too
            if repo_index == app.selected_repo {
                if let Some(pr) = app.prs.iter_mut().find(|pr| pr.number == pr_number) {
                    pr.mergeable = status;
                }
            }
        }

        Action::RebaseStatusUpdated(repo_index, pr_number, needs_rebase) => {
            // Update the rebase status for a specific PR
            if let Some(data) = app.repo_data.get_mut(&repo_index) {
                if let Some(pr) = data.prs.iter_mut().find(|pr| pr.number == pr_number) {
                    pr.needs_rebase = needs_rebase;
                }
            }

            // If this is the current repo, update app.prs too
            if repo_index == app.selected_repo {
                if let Some(pr) = app.prs.iter_mut().find(|pr| pr.number == pr_number) {
                    pr.needs_rebase = needs_rebase;
                }
            }
        }

        Action::RefreshCurrentRepo => {
            // Trigger background refresh
            if let Some(repo) = app.repo().cloned() {
                app.loading_state = LoadingState::Loading;
                let data = app.repo_data.entry(app.selected_repo).or_default();
                data.loading_state = LoadingState::Loading;

                app.task_status = Some(TaskStatus {
                    message: format!("Refreshing {}/{}...", repo.org, repo.repo),
                    status_type: TaskStatusType::Running,
                });

                let _ = app.task_tx.send(BackgroundTask::LoadSingleRepo {
                    repo_index: app.selected_repo,
                    repo,
                    filter: app.filter.clone(),
                    octocrab: app.octocrab()?,
                });
            }
        }

        Action::CycleFilter => {
            // Cycle to next filter and reload current repo
            app.filter = app.filter.next();

            app.task_status = Some(TaskStatus {
                message: format!("Filtering by: {}", app.filter.label()),
                status_type: TaskStatusType::Running,
            });

            if let Some(repo) = app.repo().cloned() {
                app.loading_state = LoadingState::Loading;
                let data = app.repo_data.entry(app.selected_repo).or_default();
                data.loading_state = LoadingState::Loading;

                let _ = app.task_tx.send(BackgroundTask::LoadSingleRepo {
                    repo_index: app.selected_repo,
                    repo,
                    filter: app.filter.clone(),
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
                        let prs_needing_rebase: Vec<usize> = repo_data.prs
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
                app.task_status = Some(TaskStatus {
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
            match result {
                Ok(_) => {
                    debug!("Rebase completed successfully");
                    app.task_status = Some(TaskStatus {
                        message: "Rebase completed successfully".to_string(),
                        status_type: TaskStatusType::Success,
                    });
                }
                Err(err) => {
                    debug!("Rebase failed: {}", err);
                    app.task_status = Some(TaskStatus {
                        message: format!("Rebase failed: {}", err),
                        status_type: TaskStatusType::Error,
                    });
                }
            }
        }

        Action::MergeSelectedPrs => {
            // Trigger background merge
            if let Some(repo) = app.repo().cloned() {
                let repo_data = app.get_current_repo_data();
                let selected_count = repo_data.selected_prs.len();

                app.task_status = Some(TaskStatus {
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

        Action::MergeComplete(result) => {
            match result {
                Ok(_) => {
                    debug!("Merge completed successfully");
                    app.task_status = Some(TaskStatus {
                        message: "Merge completed successfully".to_string(),
                        status_type: TaskStatusType::Success,
                    });
                    // Clear selections after successful merge
                    app.selected_prs.clear();
                    let data = app.repo_data.entry(app.selected_repo).or_default();
                    data.selected_prs.clear();
                }
                Err(err) => {
                    debug!("Merge failed: {}", err);
                    app.task_status = Some(TaskStatus {
                        message: format!("Merge failed: {}", err),
                        status_type: TaskStatusType::Error,
                    });
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
                            let url = format!("https://github.com/{}/{}/pull/{}", repo.org, repo.repo, pr.number);
                            #[cfg(target_os = "macos")]
                            let _ = std::process::Command::new("open").arg(&url).spawn();
                            #[cfg(target_os = "linux")]
                            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                            #[cfg(target_os = "windows")]
                            let _ = std::process::Command::new("cmd").args(&["/C", "start", &url]).spawn();
                        }
                    }
                } else if let Some(selected_idx) = repo_data.table_state.selected() {
                    // If no multi-selection, open the currently focused PR
                    if let Some(pr) = repo_data.prs.get(selected_idx) {
                        let url = format!("https://github.com/{}/{}/pull/{}", repo.org, repo.repo, pr.number);
                        #[cfg(target_os = "macos")]
                        let _ = std::process::Command::new("open").arg(&url).spawn();
                        #[cfg(target_os = "linux")]
                        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                        #[cfg(target_os = "windows")]
                        let _ = std::process::Command::new("cmd").args(&["/C", "start", &url]).spawn();
                    }
                }
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
        }
        Action::NavigateToNextPr => {
            app.next();
        }
        Action::NavigateToPreviousPr => {
            app.previous();
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
                BackgroundTask::LoadAllRepos { repos, filter, octocrab } => {
                    // Spawn parallel tasks for each repo
                    let mut tasks = Vec::new();
                    for (index, repo) in repos.iter().enumerate() {
                        let octocrab = octocrab.clone();
                        let repo = repo.clone();
                        let filter = filter.clone();

                        let task = tokio::spawn(async move {
                            let result = fetch_github_data(&octocrab, &repo, &filter).await
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
                BackgroundTask::LoadSingleRepo { repo_index, repo, filter, octocrab } => {
                    let result = fetch_github_data(&octocrab, &repo, &filter).await
                        .map_err(|e| e.to_string());
                    let _ = action_tx.send(Action::RepoDataLoaded(repo_index, result));
                }
                BackgroundTask::CheckMergeStatus { repo_index, repo, pr_numbers, octocrab } => {
                    // Check merge status for each PR in parallel
                    let mut tasks = Vec::new();
                    for pr_number in pr_numbers {
                        let octocrab = octocrab.clone();
                        let repo = repo.clone();
                        let action_tx = action_tx.clone();

                        let task = tokio::spawn(async move {
                            use crate::pr::MergeableStatus;

                            // Fetch detailed PR info to get mergeable status and rebase status
                            match octocrab.pulls(&repo.org, &repo.repo).get(pr_number as u64).await {
                                Ok(pr_detail) => {
                                    let status = match pr_detail.mergeable {
                                        Some(true) => MergeableStatus::Mergeable,
                                        Some(false) => {
                                            // Check if it's due to conflicts or blocks
                                            if let Some(ref state) = pr_detail.mergeable_state {
                                                match state {
                                                    octocrab::models::pulls::MergeableState::Dirty => MergeableStatus::Conflicted,
                                                    octocrab::models::pulls::MergeableState::Blocked => MergeableStatus::Blocked,
                                                    _ => MergeableStatus::Blocked,
                                                }
                                            } else {
                                                MergeableStatus::Conflicted
                                            }
                                        }
                                        None => MergeableStatus::Unknown,
                                    };

                                    // Check if PR needs rebase (Behind state means PR is behind base branch)
                                    let needs_rebase = if let Some(ref state) = pr_detail.mergeable_state {
                                        matches!(state, octocrab::models::pulls::MergeableState::Behind)
                                    } else {
                                        false
                                    };

                                    let _ = action_tx.send(Action::MergeStatusUpdated(repo_index, pr_number, status));
                                    let _ = action_tx.send(Action::RebaseStatusUpdated(repo_index, pr_number, needs_rebase));
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
                BackgroundTask::Rebase { repo, prs, selected_indices, octocrab } => {
                    let mut success = true;
                    for &idx in &selected_indices {
                        if let Some(pr) = prs.get(idx) {
                            // For dependabot PRs, use comment-based rebase
                            if pr.author.starts_with("dependabot") {
                                if let Err(_) = comment(&octocrab, &repo, pr, "@dependabot rebase").await {
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
                    let result = if success { Ok(()) } else { Err("Some rebases failed".to_string()) };
                    let _ = action_tx.send(Action::RebaseComplete(result));
                }
                BackgroundTask::Merge { repo, prs, selected_indices, octocrab } => {
                    let mut success = true;
                    for &idx in &selected_indices {
                        if let Some(pr) = prs.get(idx) {
                            if let Err(_) = merge(&octocrab, &repo, pr).await {
                                success = false;
                            }
                        }
                    }
                    let result = if success { Ok(()) } else { Err("Some merges failed".to_string()) };
                    let _ = action_tx.send(Action::MergeComplete(result));
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
        t.draw(|f| {
            ui(f, &mut app);
        })?;

        if let Some(action) = action_rx.recv().await {
            if let Err(err) = update(&mut app, action).await {
                app.loading_state = LoadingState::Error(err.to_string());
                app.should_quit = true;
                debug!("Error updating app: {}", err);
            }
        }

        if app.should_quit {
            store_recent_repos(&app.recent_repos)?;
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
    if app.bootstrap_state != BootstrapState::Completed {
        let message = match &app.bootstrap_state {
            BootstrapState::NotStarted => "Initializing application...",
            BootstrapState::LoadingRepositories => "Loading repositories...",
            BootstrapState::RestoringSession => "Restoring session...",
            BootstrapState::LoadingPRs => "Loading pull requests from all repositories...",
            BootstrapState::Error(err) => {
                // Return early for error to show it
                f.render_widget(
                    Paragraph::new(format!("Error: {}", err))
                        .centered()
                        .style(Style::default().fg(Color::Red)),
                    f.area(),
                );
                return;
            }
            BootstrapState::Completed => unreachable!(),
        };

        f.render_widget(
            Paragraph::new(message)
                .centered()
                .style(Style::default().fg(app.colors.row_fg)),
            f.area(),
        );
        return;
    }

    // If no repositories at all (shouldn't happen after bootstrap completes)
    if app.recent_repos.is_empty() {
        f.render_widget(
            Paragraph::new("No repositories configured. Add repositories to .recent-repositories.json").centered(),
            f.area(),
        );
        return;
    }

    // Split the layout: tabs on top, table in middle, action panel and status at bottom
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tabs
            Constraint::Min(0),    // Table
            Constraint::Length(3), // Action panel
            Constraint::Length(1), // Status line
        ])
        .split(f.area());

    // Render tabs (always visible when there are repos)
    let tab_titles: Vec<Line> = app
        .recent_repos
        .iter()
        .enumerate()
        .map(|(i, repo)| {
            let number = if i < 9 { format!("{} ", i + 1) } else { String::new() };
            Line::from(format!("{}{}/{}", number, repo.org, repo.repo))
        })
        .collect();

    let tabs = Tabs::new(tab_titles)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(format!("Projects [Tab/1-9: switch, /: cycle] | Filter: {} [f: cycle]", app.filter.label())))
        .select(app.selected_repo)
        .style(Style::default().fg(app.colors.row_fg))
        .highlight_style(
            Style::default()
                .fg(app.colors.selected_row_style_fg)
                .add_modifier(Modifier::BOLD)
                .bg(app.colors.header_bg),
        );

    f.render_widget(tabs, chunks[0]);

    // Get the selected repo (should always exist if we have repos)
    let Some(selected_repo) = app.repo() else {
        f.render_widget(
            Paragraph::new("Error: Invalid repository selection").centered(),
            chunks[1],
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
        .fg(app.colors.header_fg)
        .bg(app.colors.header_bg);

    let header_cells = ["#PR", "Description", "Author", "#Comments", "Status", "Rebase"]
        .iter()
        .map(|h| Cell::from(*h).style(header_style));

    let header = Row::new(header_cells)
        .style(Style::default().bg(Color::Blue))
        .height(1);

    let selected_row_style = Style::default()
        .add_modifier(Modifier::REVERSED)
        .fg(app.colors.selected_row_style_fg);

    // Check if we should show a message instead of PRs
    if repo_data.prs.is_empty() {
        let message = match &repo_data.loading_state {
            LoadingState::Loading => "Loading pull requests...",
            LoadingState::Error(_err) => "Error loading data. Press Ctrl+r to retry.",
            _ => "No pull requests found matching filter",
        };

        let paragraph = Paragraph::new(message)
            .block(block)
            .style(Style::default().fg(app.colors.row_fg))
            .alignment(ratatui::layout::Alignment::Center);

        f.render_widget(paragraph, chunks[1]);
    } else {
        let rows = repo_data.prs.iter().enumerate().map(|(i, item)| {
            let color = match i % 2 {
                0 => app.colors.normal_row_color,
                _ => app.colors.alt_row_color,
            };
            let color = if repo_data.selected_prs.contains(&i) {
                app.colors.selected_cell_style_fg
            } else {
                color
            };
            let row: Row = item.into();
            row.style(Style::new().fg(app.colors.row_fg).bg(color))
                .height(1)
        });

        let widths = [
            Constraint::Percentage(8),   // #PR
            Constraint::Percentage(57),  // Description
            Constraint::Percentage(15),  // Author
            Constraint::Percentage(10),  // #Comments
            Constraint::Percentage(5),   // Status
            Constraint::Percentage(5),   // Rebase
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(block)
            .row_highlight_style(selected_row_style);

        // Get mutable reference to the current repo's table state
        let table_state = &mut app.get_current_repo_data_mut().table_state;
        f.render_stateful_widget(table, chunks[1], table_state);
    }

    // Render context-sensitive action panel at the bottom
    render_action_panel(f, app, chunks[2]);

    // Render status line at the very bottom
    render_status_line(f, app, chunks[3]);
}

/// Render the bottom action panel with context-sensitive shortcuts
fn render_action_panel(f: &mut Frame, app: &App, area: Rect) {
    let repo_data = app.get_current_repo_data();
    let selected_count = repo_data.selected_prs.len();

    // Split into context-sensitive (left) and global actions (right)
    let mut context_actions: Vec<(String, String, Color)> = Vec::new();
    let mut global_actions: Vec<(String, String, Color)> = Vec::new();

    // Context-sensitive actions (what can I do NOW with this selection)
    if selected_count > 0 {
        // Highlight merge action when PRs are selected
        context_actions.push((
            "m".to_string(),
            format!("Merge ({})", selected_count),
            tailwind::GREEN.c700,
        ));

        // Show rebase action for manually selected PRs
        context_actions.push((
            "r".to_string(),
            format!("Rebase ({})", selected_count),
            tailwind::BLUE.c700,
        ));
    } else if !repo_data.prs.is_empty() {
        // When nothing selected, show how to select
        context_actions.push((
            "Space".to_string(),
            "Select".to_string(),
            tailwind::AMBER.c600,
        ));

        // Check if there are PRs that need rebase - show auto-rebase option
        let prs_needing_rebase = repo_data.prs.iter().filter(|pr| pr.needs_rebase).count();
        if prs_needing_rebase > 0 {
            context_actions.push((
                "r".to_string(),
                format!("Auto-rebase ({})", prs_needing_rebase),
                tailwind::YELLOW.c600,
            ));
        }
    }

    // Add Enter action when PR(s) are selected or focused
    if !repo_data.prs.is_empty() {
        if selected_count > 0 {
            context_actions.push((
                "Enter".to_string(),
                format!("Open in Browser ({})", selected_count),
                tailwind::PURPLE.c600,
            ));
        } else if repo_data.table_state.selected().is_some() {
            context_actions.push((
                "Enter".to_string(),
                "Open in Browser".to_string(),
                tailwind::PURPLE.c600,
            ));
        }
    }

    // Global actions (always available, less emphasized)
    global_actions.push(("↑↓/jk".to_string(), "Navigate".to_string(), app.colors.header_bg));
    global_actions.push(("f".to_string(), format!("Filter: {}", app.filter.label()), app.colors.header_bg));
    global_actions.push(("Tab".to_string(), "Switch Project".to_string(), app.colors.header_bg));
    global_actions.push(("Ctrl+r".to_string(), "Refresh".to_string(), app.colors.header_bg));
    global_actions.push(("q".to_string(), "Quit".to_string(), app.colors.header_bg));

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
                Style::default().fg(app.colors.row_fg),
            ));
        }
        spans
    };

    // Split the area into left and right sections
    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), // Left: context-sensitive
            Constraint::Percentage(50), // Right: global actions
        ])
        .split(area);

    // Render context-sensitive actions (left-aligned)
    let context_spans = create_action_spans(&context_actions);
    let context_line = Line::from(context_spans);
    let context_paragraph = Paragraph::new(context_line)
        .block(Block::default().borders(Borders::ALL).title("Actions"))
        .alignment(ratatui::layout::Alignment::Left);
    f.render_widget(context_paragraph, sections[0]);

    // Render global actions (right-aligned)
    let global_spans = create_action_spans(&global_actions);
    let global_line = Line::from(global_spans);
    let global_paragraph = Paragraph::new(global_line)
        .block(Block::default().borders(Borders::ALL).title("Global"))
        .alignment(ratatui::layout::Alignment::Right);
    f.render_widget(global_paragraph, sections[1]);
}

/// Render the status line showing background task progress
fn render_status_line(f: &mut Frame, app: &App, area: Rect) {
    if let Some(ref status) = app.task_status {
        let (icon, color) = match status.status_type {
            TaskStatusType::Running => ("⏳", Color::Yellow),
            TaskStatusType::Success => ("✓", Color::Green),
            TaskStatusType::Error => ("✗", Color::Red),
        };

        let status_text = format!(" {} {}", icon, status.message);
        let status_span = Span::styled(
            status_text,
            Style::default()
                .fg(color)
                .add_modifier(Modifier::BOLD),
        );

        let paragraph = Paragraph::new(Line::from(status_span))
            .style(Style::default().bg(app.colors.buffer_bg));
        f.render_widget(paragraph, area);
    }
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
    fn new(action_tx: mpsc::UnboundedSender<Action>, task_tx: mpsc::UnboundedSender<BackgroundTask>) -> App {
        App {
            action_tx,
            task_tx,
            should_quit: false,
            state: TableState::default(),
            prs: Vec::new(),
            recent_repos: Vec::new(),
            selected_repo: 0,
            filter: PrFilter::None,
            selected_prs: Vec::new(),
            colors: TableColors::new(&PALETTES[0]),
            loading_state: LoadingState::Idle,
            bootstrap_state: BootstrapState::NotStarted,
            repo_data: std::collections::HashMap::new(),
            task_status: None,
        }
    }

    /// Get the current repo data (read-only)
    fn get_current_repo_data(&self) -> RepoData {
        self.repo_data
            .get(&self.selected_repo)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the current repo data (mutable)
    fn get_current_repo_data_mut(&mut self) -> &mut RepoData {
        self.repo_data.entry(self.selected_repo).or_default()
    }

    /// Save current state to the repo data before switching tabs
    fn save_current_repo_state(&mut self) {
        let data = self.repo_data.entry(self.selected_repo).or_default();
        data.prs = self.prs.clone();
        data.table_state = self.state.clone();
        data.selected_prs = self.selected_prs.clone();
        data.loading_state = self.loading_state.clone();
    }

    /// Load state from repo data when switching tabs
    fn load_repo_state(&mut self) {
        let data = self.get_current_repo_data();
        self.prs = data.prs;
        self.state = data.table_state;
        self.selected_prs = data.selected_prs;
        self.loading_state = data.loading_state;
    }

    fn octocrab(&self) -> Result<Octocrab> {
        Ok(Octocrab::builder()
            .personal_token(
                env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN environment variable must be set"),
            )
            .build()?)
    }

    fn repo(&self) -> Option<&Repo> {
        self.recent_repos.get(self.selected_repo)
    }

    async fn restore_session(&mut self, state: PersistedState) -> Result<()> {
        // Restore the selected repository from the persisted state
        if let Some(index) = self
            .recent_repos
            .iter()
            .position(|r| r == &state.selected_repo)
        {
            self.selected_repo = index;
        } else {
            // If the persisted repo is not found, default to first repo
            debug!("Persisted repository not found in recent repositories, defaulting to first");
            self.selected_repo = 0;
        }

        Ok(())
    }

    /// Fetch data from GitHub for the selected repository and filter
    async fn fetch_data(&mut self, repo: &Repo) -> Result<()> {
        self.loading_state = LoadingState::Loading;

        let octocrab = self.octocrab()?.clone();
        let repo = repo.clone();
        let filter = self.filter.clone();

        let github_data =
            tokio::task::spawn(async move { fetch_github_data(&octocrab, &repo, &filter).await })
                .await??;
        self.prs = github_data;

        self.loading_state = LoadingState::Loaded;

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
        self.state.select(Some(i));
        let data = self.repo_data.entry(self.selected_repo).or_default();
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
        self.state.select(Some(i));
        let data = self.repo_data.entry(self.selected_repo).or_default();
        data.table_state.select(Some(i));
    }

    /// Toggle the selection of the currently selected PR
    fn select_toggle(&mut self) {
        let repo_data = self.get_current_repo_data();
        let i = repo_data.table_state.selected().unwrap_or(0);

        // Update both the app state and repo data
        if self.selected_prs.contains(&i) {
            self.selected_prs.retain(|&x| x != i);
        } else {
            self.selected_prs.push(i);
        }

        let data = self.repo_data.entry(self.selected_repo).or_default();
        if data.selected_prs.contains(&i) {
            data.selected_prs.retain(|&x| x != i);
        } else {
            data.selected_prs.push(i);
        }
    }

    /// Select the next repo (cycle forward through tabs)
    fn select_next_repo(&mut self) {
        self.save_current_repo_state();
        self.selected_repo = (self.selected_repo + 1) % self.recent_repos.len();
        self.load_repo_state();
    }

    /// Select the previous repo (cycle backward through tabs)
    fn select_previous_repo(&mut self) {
        self.save_current_repo_state();
        self.selected_repo = if self.selected_repo == 0 {
            self.recent_repos.len() - 1
        } else {
            self.selected_repo - 1
        };
        self.load_repo_state();
    }

    /// Select a repo by index (for number key shortcuts)
    fn select_repo_by_index(&mut self, index: usize) {
        if index < self.recent_repos.len() {
            self.save_current_repo_state();
            self.selected_repo = index;
            self.load_repo_state();
        }
    }

    /// Load data for all repositories in parallel on startup
    async fn load_all_repos(&mut self) -> Result<()> {
        let octocrab = self.octocrab()?;
        let filter = self.filter.clone();
        let repos = self.recent_repos.clone();

        // Set all repos to loading state
        for i in 0..repos.len() {
            let data = self.repo_data.entry(i).or_default();
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
                let data = self.repo_data.entry(index).or_default();
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
        self.loading_state = LoadingState::Loading;
        let data = self.repo_data.entry(self.selected_repo).or_default();
        data.loading_state = LoadingState::Loading;

        self.fetch_data(&repo).await?;

        // Update the repo data cache
        let data = self.repo_data.entry(self.selected_repo).or_default();
        data.prs = self.prs.clone();
        data.loading_state = self.loading_state.clone();

        Ok(())
    }

    async fn select_repo(&mut self) -> Result<()> {
        let Some(repo) = self.repo().cloned() else {
            bail!("No repository selected");
        };
        debug!("Selecting repo: {:?}", repo);

        // This function is a placeholder for future implementation
        // It could be used to select a specific repo from a list or input
        self.selected_prs.clear();
        self.fetch_data(&repo).await?;
        self.state.select(Some(0));
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
        for &pr_index in &self.selected_prs {
            if let Some(pr) = self.prs.get(pr_index) {
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
        debug!("Rebasing selected PRs: {:?}", self.selected_prs);

        Ok(())
    }

    /// Merge the selected PRs
    async fn merge(&mut self) -> Result<()> {
        let Some(repo) = self.repo() else {
            bail!("No repository selected for merging");
        };
        let octocrab = self.octocrab()?;
        let mut selected_prs = self.selected_prs.clone();
        for &pr_index in self.selected_prs.iter() {
            if let Some(pr) = self.prs.get(pr_index) {
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

        // todo: now clean up `self.prs` by those that are not in `selected_prs` anymore,
        // there the index of the PRs is to take

        self.selected_prs = selected_prs;
        debug!("Merging selected PRs: {:?}", self.selected_prs);

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
            pr.title.as_ref().map(|t| filter.matches(t)).unwrap_or(false)
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
    use crossterm::event::KeyModifiers;
    let shift_pressed = key.modifiers.contains(KeyModifiers::SHIFT);
    let ctrl_pressed = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('f') => Action::CycleFilter,
        KeyCode::Char('r') if ctrl_pressed => Action::RefreshCurrentRepo,
        KeyCode::Char('r') => Action::Rebase,
        KeyCode::Char('/') => Action::SelectNextRepo,
        KeyCode::Tab if shift_pressed => Action::SelectPreviousRepo,
        KeyCode::Tab => Action::SelectNextRepo,
        KeyCode::BackTab => Action::SelectPreviousRepo, // Shift+Tab on some terminals
        KeyCode::Char('j') | KeyCode::Down => Action::NavigateToNextPr,
        KeyCode::Char('k') | KeyCode::Up => Action::NavigateToPreviousPr,
        KeyCode::Char(' ') => Action::TogglePrSelection,
        KeyCode::Char('m') => Action::MergeSelectedPrs,
        KeyCode::Enter => Action::OpenCurrentPrInBrowser,
        // Number keys 1-9 for direct tab selection
        KeyCode::Char('1') => Action::SelectRepoByIndex(0),
        KeyCode::Char('2') => Action::SelectRepoByIndex(1),
        KeyCode::Char('3') => Action::SelectRepoByIndex(2),
        KeyCode::Char('4') => Action::SelectRepoByIndex(3),
        KeyCode::Char('5') => Action::SelectRepoByIndex(4),
        KeyCode::Char('6') => Action::SelectRepoByIndex(5),
        KeyCode::Char('7') => Action::SelectRepoByIndex(6),
        KeyCode::Char('8') => Action::SelectRepoByIndex(7),
        KeyCode::Char('9') => Action::SelectRepoByIndex(8),
        _ => Action::None,
    }
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
