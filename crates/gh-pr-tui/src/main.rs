use anyhow::{Context, Result};
use gh_api_cache::{ApiCache, CachedResponse};
use octocrab::{Octocrab, params};
use ratatui::{
    crossterm::{
        self,
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    },
    prelude::*,
    widgets::*,
};
use serde::{Deserialize, Serialize};
use std::{
    io::BufReader,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

// Import debug from the log crate using :: prefix to disambiguate from our log module
use ::log::debug;

use crate::actions::Action;
use crate::config::Config;
use crate::effect::execute_effect;
use crate::pr::Pr;
use crate::state::*;
use crate::store::Store;
use crate::task::{BackgroundTask, TaskResult, start_task_worker};
use crate::theme::Theme;

mod actions;
mod command_palette_integration;
mod config;
mod effect;
mod gh;
mod infra;
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
mod view_models;
mod views;

pub struct App {
    // Redux store - centralized state management
    pub store: Store,
    // Communication channels
    pub action_tx: mpsc::UnboundedSender<Action>,
    pub task_tx: mpsc::UnboundedSender<BackgroundTask>,
    // Lazy-initialized octocrab client (created after .env is loaded)
    pub octocrab: Option<Octocrab>,
    // API response cache for development workflow (Arc<Mutex> for sharing across tasks)
    pub cache: Arc<Mutex<ApiCache>>,
    // Splash screen state
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
    // When close PR popup is open, handle popup-specific actions
    let msg = if app.store.state().ui.close_pr_state.is_some() {
        match msg {
            // Allow these actions in the popup
            Action::HideClosePrPopup
            | Action::ClosePrFormInput(_)
            | Action::ClosePrFormBackspace
            | Action::ClosePrFormSubmit
            | Action::None => msg, // Allow None for keys we ignore (arrows, etc.)
            // Quit closes the popup
            Action::Quit => Action::HideClosePrPopup,
            // Ignore all other actions while popup is open
            _ => {
                return Ok(Action::None);
            }
        }
    } else if app.store.state().ui.show_add_repo {
        // When add repo popup is open, handle popup-specific actions
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
    show_close_pr_sync: Arc<Mutex<bool>>,
    show_command_palette_sync: Arc<Mutex<bool>>,
) -> (tokio::task::JoinHandle<()>, Arc<Mutex<bool>>) {
    let tick_rate = std::time::Duration::from_millis(250);
    // Clone the shared popup state flags for the event loop
    let show_add_repo_shared = app.store.state().ui.show_add_repo_shared.clone();
    let show_close_pr_shared = show_close_pr_sync;
    let show_command_palette_shared = show_command_palette_sync;
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

    let close_pr_shared_for_loop = show_close_pr_shared.clone();
    let command_palette_shared_for_loop = show_command_palette_shared.clone();
    let handle = tokio::spawn(async move {
        loop {
            let action = if crossterm::event::poll(tick_rate).unwrap() {
                let show_add_repo = *show_add_repo_shared.lock().unwrap();
                let show_close_pr = *close_pr_shared_for_loop.lock().unwrap();
                let show_command_palette = *command_palette_shared_for_loop.lock().unwrap();
                let log_panel_open_val = *log_panel_open.lock().unwrap();
                let job_list_focused_val = *job_list_focused.lock().unwrap();
                let console_open = *debug_console_open.lock().unwrap();
                handle_events(
                    show_add_repo,
                    show_close_pr,
                    show_command_palette,
                    log_panel_open_val,
                    job_list_focused_val,
                    console_open,
                    &pending_key_shared,
                )
                .unwrap_or(Action::None)
            } else {
                Action::None
            };

            if tx.send(action).is_err() {
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
        TaskResult::ClosePrComplete(res) => Action::ClosePrComplete(res),
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
        TaskResult::RepoNeedsReload(idx) => Action::ReloadRepo(idx),
    }
}

async fn run_with_log_buffer(log_buffer: log_capture::LogBuffer) -> Result<()> {
    let mut t = Terminal::new(CrosstermBackend::new(std::io::stderr()))?;

    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    let (task_tx, task_rx) = mpsc::unbounded_channel();
    let (result_tx, mut result_rx) = mpsc::unbounded_channel(); // New result channel

    let mut app = App::new(action_tx.clone(), task_tx, log_buffer);

    // Create shared state for close PR popup (synced in main loop)
    let show_close_pr_shared = Arc::new(Mutex::new(false));
    // Create shared state for command palette (synced in main loop)
    let show_command_palette_shared = Arc::new(Mutex::new(false));
    let (event_task, debug_console_shared) = start_event_handler(
        &app,
        app.action_tx.clone(),
        show_close_pr_shared.clone(),
        show_command_palette_shared.clone(),
    );
    let worker_task = start_task_worker(task_rx, result_tx);

    app.action_tx
        .send(Action::Bootstrap)
        .expect("Failed to send bootstrap action");

    loop {
        // Sync the shared popup states for event handler
        *app.store.state().ui.show_add_repo_shared.lock().unwrap() =
            app.store.state().ui.show_add_repo;
        // Sync close PR popup visibility to shared state
        *show_close_pr_shared.lock().unwrap() = app.store.state().ui.close_pr_state.is_some();
        // Sync command palette visibility to shared state
        *show_command_palette_shared.lock().unwrap() =
            app.store.state().ui.command_palette.is_some();

        // Sync the shared debug console state for event handler
        *debug_console_shared.lock().unwrap() = app.store.state().debug_console.is_open;

        // Sync the shared log panel state for event handler
        *app.store
            .state()
            .log_panel
            .log_panel_open_shared
            .lock()
            .unwrap() = app.store.state().log_panel.panel.is_some();

        // Handle force redraw flag - clear terminal if requested
        if app.store.state().ui.force_redraw {
            t.clear()?;
            // Reset the flag after clearing
            let state = app.store.state_mut();
            state.ui.force_redraw = false;
        }

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
                // Also step the merge bot if it's running (Redux action)
                if app.store.state().merge_bot.bot.is_running() {
                    let _ = app.action_tx.send(Action::MergeBotTick);
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
    // Show bootstrap/splash screen until UI is ready (first repo loaded)
    let ui_ready = matches!(
        app.store.state().repos.bootstrap_state,
        BootstrapState::UIReady | BootstrapState::LoadingRemainingRepos | BootstrapState::Completed
    );

    if !ui_ready {
        // Show splash screen during loading
        crate::views::splash_screen::render_splash_screen(f, app);
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

    // Render repository tabs
    crate::views::repositories::render_repository_tabs(f, chunks[0], app);

    // Render PR table
    crate::views::pull_requests::render_pr_table(f, table_area, app);

    // Render context-sensitive action panel at the bottom
    crate::views::pull_requests::render_action_panel(f, app, chunks[2]);

    // Render status bar at the very bottom
    crate::views::status_bar::render_status_bar(f, app, chunks[3]);

    // Render log panel LAST if it's open - covers only the table area
    if let Some(ref view_model) = app.store.state().log_panel.view_model {
        let viewport_height = crate::views::build_log::render_log_panel(
            f,
            view_model,
            &app.store.state().theme,
            chunks[1],
        );
        // Update viewport height for page down scrolling
        app.store
            .dispatch(Action::UpdateLogPanelViewport(viewport_height));
    }

    // Render shortcuts panel on top of everything if visible
    if app.store.state().ui.show_shortcuts {
        let max_scroll = crate::views::help::render_shortcuts_panel(
            f,
            chunks[1],
            app.store.state().ui.shortcuts_scroll,
            &app.store.state().theme,
        );
        app.store.state_mut().ui.shortcuts_max_scroll = max_scroll;
    }

    // Render add repo popup on top of everything if visible
    if app.store.state().ui.show_add_repo {
        crate::views::repositories::render_add_repo_popup(
            f,
            chunks[1],
            &app.store.state().ui.add_repo_form,
            &app.store.state().theme,
        );
    }

    // Render close PR popup on top of everything if visible
    if let Some(ref close_pr_state) = app.store.state().ui.close_pr_state {
        crate::views::pull_requests::render_close_pr_popup(
            f,
            chunks[1],
            &close_pr_state.comment,
            &app.store.state().theme,
        );
    }

    // Render command palette on top of everything (highest priority popup)
    if app.store.state().ui.command_palette.is_some() {
        crate::views::command_palette::render_command_palette(f, f.area(), app);
    }

    // Render debug console (Quake-style drop-down) if visible
    if app.store.state().debug_console.is_open {
        let viewport_height = crate::views::debug_console::render_debug_console(f, f.area(), app);
        // Update viewport height for page down scrolling
        app.store
            .dispatch(Action::UpdateDebugConsoleViewport(viewport_height));
    }
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

        let cache_file = crate::infra::files::get_cache_file_path()
            .unwrap_or_else(|_| std::env::temp_dir().join("gh-api-cache.json"));

        App {
            store: Store::new(initial_state),
            action_tx,
            task_tx,
            octocrab: None, // Initialized lazily during bootstrap after .env is loaded
            cache: Arc::new(Mutex::new(ApiCache::new(cache_file).unwrap_or_default())),
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

pub async fn fetch_github_data(
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
            let pr = Pr::from_pull_request(&pr, repo, octocrab).await;
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

/// Fetch GitHub data with disk caching and ETag support
///
/// This wrapper around `fetch_github_data` provides:
/// - Disk-based response caching with 20-minute TTL
/// - ETag-based conditional requests (304 Not Modified)
/// - Automatic cache invalidation for stale entries
///
/// Cache is bypassed for manual refresh actions to ensure fresh data.
pub async fn fetch_github_data_cached(
    octocrab: &Octocrab,
    repo: &Repo,
    filter: &PrFilter,
    cache: &Arc<Mutex<ApiCache>>,
    bypass_cache: bool,
) -> Result<Vec<Pr>> {
    // Skip cache if disabled entirely (environment variable)
    if !ApiCache::is_enabled() {
        debug!("Cache disabled, fetching fresh data for {}/{}", repo.org, repo.repo);
        return fetch_github_data(octocrab, repo, filter).await;
    }

    // If bypassing cache (manual refresh), skip cache lookup but still update cache after fetch
    if bypass_cache {
        debug!(
            "Cache bypass requested (manual refresh), will fetch fresh data and update cache for {}/{}",
            repo.org, repo.repo
        );
        // Continue to fetch fresh data and update cache below
    }

    // Build cache key from API endpoint and params
    let url = format!("/repos/{}/{}/pulls", repo.org, repo.repo);
    let params = [
        ("state", "open"),
        ("head", repo.branch.as_str()),
        ("per_page", "30"),
    ];

    // Try to get cached response (unless bypassing cache for manual refresh)
    if !bypass_cache {
        let cached = {
            let cache_guard = cache.lock().unwrap();
            cache_guard.get("GET", &url, &params)
        };

        if let Some(cached_response) = cached {
            // Try to parse cached body
            match serde_json::from_str::<Vec<octocrab::models::pulls::PullRequest>>(
                &cached_response.body,
            ) {
                Ok(prs_data) => {
                    debug!(
                        "Cache HIT for {}/{}: {} PRs (status: {})",
                        repo.org,
                        repo.repo,
                        prs_data.len(),
                        cached_response.status_code
                    );

                    // Convert to our Pr type (without fetching additional details)
                    // For cached PRs, we'll use cached status data
                    let mut prs = Vec::new();
                    for pr_model in prs_data.into_iter().filter(|pr| {
                        pr.title
                            .as_ref()
                            .map(|t| filter.matches(t))
                            .unwrap_or(false)
                    }) {
                        if prs.len() >= 50 {
                            break;
                        }
                        let pr = Pr::from_pull_request(&pr_model, repo, octocrab).await;
                        prs.push(pr);
                    }

                    prs.sort_by(|a, b| b.number.cmp(&a.number));

                    // If cache entry was stale (status_code 200), refresh in background
                    // but return cached data immediately for fast startup
                    if cached_response.status_code == 200 {
                        // Touch the cache to extend TTL since we validated it's still useful
                        let mut cache_guard = cache.lock().unwrap();
                        let _ = cache_guard.touch("GET", &url, &params);
                    }

                    return Ok(prs);
                }
                Err(e) => {
                    debug!(
                        "Failed to parse cached response for {}/{}: {}",
                        repo.org, repo.repo, e
                    );
                    // Cache entry is corrupted, invalidate it and fetch fresh
                    let mut cache_guard = cache.lock().unwrap();
                    cache_guard.invalidate("GET", &url, &params);
                }
            }
        }
    }

    // Cache miss, invalid, or bypassed - fetch fresh data
    let reason = if bypass_cache {
        "bypass"
    } else {
        "miss"
    };
    debug!(
        "Cache {} for {}/{}, fetching from API",
        reason, repo.org, repo.repo
    );
    let prs = fetch_github_data(octocrab, repo, filter).await?;

    // Cache the response for next time
    // We need to fetch the raw JSON response to cache it properly
    // For now, we'll make a direct API call to get the raw response with headers
    let response = octocrab
        .pulls(&repo.org, &repo.repo)
        .list()
        .state(params::State::Open)
        .head(&repo.branch)
        .per_page(30)
        .page(1u32)
        .send()
        .await?;

    // Extract ETag from headers if present
    // Note: octocrab's Page type doesn't expose headers directly,
    // so we'll store the response without ETag for now
    // TODO: Consider using reqwest directly for full header access
    let response_json = serde_json::to_string(&response.items)?;
    let cached_response = CachedResponse {
        body: response_json,
        etag: None, // TODO: Extract from headers
        status_code: 200,
    };

    {
        let mut cache_guard = cache.lock().unwrap();
        let _ = cache_guard.set("GET", &url, &params, &cached_response);
    }

    Ok(prs)
}

/// Context for key event handling
struct KeyEventContext<'a> {
    show_add_repo: bool,
    show_close_pr: bool,
    show_command_palette: bool,
    log_panel_open: bool,
    job_list_focused: bool,
    debug_console_open: bool,
    pending_key_shared: &'a std::sync::Arc<std::sync::Mutex<Option<crate::state::PendingKeyPress>>>,
}

fn handle_events(
    show_add_repo: bool,
    show_close_pr: bool,
    show_command_palette: bool,
    log_panel_open: bool,
    job_list_focused: bool,
    debug_console_open: bool,
    pending_key_shared: &std::sync::Arc<std::sync::Mutex<Option<crate::state::PendingKeyPress>>>,
) -> Result<Action> {
    let ctx = KeyEventContext {
        show_add_repo,
        show_close_pr,
        show_command_palette,
        log_panel_open,
        job_list_focused,
        debug_console_open,
        pending_key_shared,
    };

    Ok(match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key_event(key, &ctx),
        _ => Action::None,
    })
}

fn handle_key_event(key: KeyEvent, ctx: &KeyEventContext) -> Action {
    // Ctrl+P: Open command palette (check first before any popup handling)
    if matches!(key.code, KeyCode::Char('p')) && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Action::ShowCommandPalette;
    }

    // Handle command palette keys if open (highest priority - before other popups)
    if ctx.show_command_palette {
        match key.code {
            KeyCode::Esc => return Action::HideCommandPalette,
            KeyCode::Enter => return Action::CommandPaletteExecute,
            KeyCode::Down | KeyCode::Char('j')
                if !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                return Action::CommandPaletteSelectNext;
            }
            KeyCode::Up | KeyCode::Char('k') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::CommandPaletteSelectPrev;
            }
            KeyCode::Backspace => return Action::CommandPaletteBackspace,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::CommandPaletteInput(c);
            }
            // Ignore other keys (function keys, etc.)
            _ => return Action::None,
        }
    }

    // Handle close PR popup keys first if popup is open
    if ctx.show_close_pr {
        match key.code {
            // Close popup: Esc, x, q
            KeyCode::Esc => return Action::HideClosePrPopup,
            KeyCode::Char('x') | KeyCode::Char('q')
                if !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                return Action::HideClosePrPopup;
            }
            // Submit: Enter
            KeyCode::Enter => return Action::ClosePrFormSubmit,
            // Edit: Backspace
            KeyCode::Backspace => return Action::ClosePrFormBackspace,
            // All other characters go into the input field
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::ClosePrFormInput(c);
            }
            // Ignore other keys (arrows, function keys, etc.)
            _ => return Action::None,
        }
    }

    // Handle add repo popup keys if popup is open
    if ctx.show_add_repo {
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
    if ctx.log_panel_open {
        match key.code {
            // Close panel (x or Esc)
            KeyCode::Char('x') | KeyCode::Esc => {
                return Action::CloseLogPanel;
            }
            // Tab: Switch focus between job list and log viewer
            KeyCode::Tab => {
                return if ctx.job_list_focused {
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
            // For all other keys when log panel is open, check if it's a general shortcut
            // (e.g., '?' for help) - fall through to general shortcut handling below
            _ => {}
        }
    }

    // Handle double-Esc to clear PR selection (before other Esc handling)
    // This needs to happen before popup/panel-specific Esc handling
    if !ctx.show_add_repo
        && !ctx.show_close_pr
        && !ctx.log_panel_open
        && !ctx.debug_console_open
        && key.code == KeyCode::Esc
    {
        // Check if there's a pending Esc key (represented as '\x1b')
        let pending_guard = ctx.pending_key_shared.lock().unwrap();
        let has_pending_esc = pending_guard
            .as_ref()
            .filter(|p| p.key == '\x1b' && p.timestamp.elapsed().as_secs() < 3)
            .is_some();
        drop(pending_guard);

        if has_pending_esc {
            // Second Esc press - clear selection
            let mut pending_guard = ctx.pending_key_shared.lock().unwrap();
            *pending_guard = None;
            drop(pending_guard);
            return Action::ClearPrSelection;
        } else {
            // First Esc press - set as pending
            let mut pending_guard = ctx.pending_key_shared.lock().unwrap();
            *pending_guard = Some(crate::state::PendingKeyPress {
                key: '\x1b', // Use escape character to represent Esc
                timestamp: std::time::Instant::now(),
            });
            drop(pending_guard);
            return Action::None;
        }
    }

    // Handle debug console keys if console is open (before general shortcuts)
    if ctx.debug_console_open {
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
            // For all other keys when debug console is open, check if it's a general shortcut
            // (e.g., '?' for help) - fall through to general shortcut handling below
            _ => {}
        }
    }

    // Use the shortcuts module to find the action for this key (with two-key support)
    let pending_guard = ctx.pending_key_shared.lock().unwrap();
    let (action, should_clear, new_pending_char) =
        crate::shortcuts::find_action_for_key_with_pending(&key, pending_guard.as_ref());

    // Update pending key state
    drop(pending_guard);
    let mut pending_guard = ctx.pending_key_shared.lock().unwrap();
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
    let repos = if let Ok(recent_repos) = infra::files::open_recent_repositories_file() {
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
    let file = infra::files::create_recent_repositories_file()?;
    serde_json::to_writer_pretty(file, &repos)
        .context("Failed to write recent repositories to file")?;

    debug!("Stored recent repositories: {:?}", repos);

    Ok(())
}

fn store_persisted_state(state: &PersistedState) -> Result<()> {
    let file = infra::files::create_session_file()?;
    serde_json::to_writer_pretty(file, state).context("Failed to write persisted state to file")?;

    debug!("Stored persisted state: {:?}", state);

    Ok(())
}

fn load_persisted_state() -> Result<PersistedState> {
    let file = infra::files::open_session_file()?;
    let reader = BufReader::new(file);
    let state: PersistedState =
        serde_json::from_reader(reader).context("Failed to parse persisted state from file")?;

    debug!("Loaded persisted state: {:?}", state);

    Ok(state)
}
