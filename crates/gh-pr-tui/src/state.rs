use ratatui::widgets::TableState;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use crate::{config::Config, log::LogPanel, merge_bot::MergeBot, pr::Pr, theme::Theme};

/// Root application state following Redux pattern
#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub ui: UiState,
    pub repos: ReposState,
    pub log_panel: LogPanelState,
    pub merge_bot: MergeBotState,
    pub task: TaskState,
    pub debug_console: DebugConsoleState,
    pub config: Config,
    pub theme: Theme,
}

/// Pending key press for two-key combinations
#[derive(Debug, Clone)]
pub struct PendingKeyPress {
    pub key: char,
    pub timestamp: std::time::Instant,
}

/// UI-specific state (shortcuts panel, spinner, quit flag)
#[derive(Debug, Clone)]
pub struct UiState {
    pub show_shortcuts: bool,
    pub shortcuts_scroll: usize,
    pub shortcuts_max_scroll: usize,
    pub spinner_frame: usize,
    pub should_quit: bool,
    pub show_add_repo: bool,
    pub add_repo_form: AddRepoForm,
    /// Shared state for event handler to know if add repo popup is open
    pub show_add_repo_shared: Arc<Mutex<bool>>,
    /// Close PR popup state (None = hidden, Some = visible with state)
    pub close_pr_state: Option<ClosePrState>,
    /// Pending key press for two-key combinations (3 second timeout)
    /// Shared with event handler for checking multi-key shortcuts
    pub pending_key: Arc<Mutex<Option<PendingKeyPress>>>,
}

/// Form state for adding a new repository
#[derive(Debug, Clone, Default)]
pub struct AddRepoForm {
    pub org: String,
    pub repo: String,
    pub branch: String,
    pub focused_field: AddRepoField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddRepoField {
    #[default]
    Org,
    Repo,
    Branch,
}

/// State for the close PR popup/view
#[derive(Debug, Clone)]
pub struct ClosePrState {
    pub comment: String,
}

impl Default for ClosePrState {
    fn default() -> Self {
        Self::new()
    }
}

impl ClosePrState {
    pub fn new() -> Self {
        Self {
            comment: "Not needed anymore".to_string(),
        }
    }
}

/// Repository and PR state
#[derive(Debug, Clone)]
pub struct ReposState {
    pub recent_repos: Vec<Repo>,
    pub selected_repo: usize,
    pub filter: PrFilter,
    pub repo_data: HashMap<usize, RepoData>,
    pub loading_state: LoadingState,
    pub bootstrap_state: BootstrapState,
    // Legacy fields from App for backward compatibility during migration
    pub prs: Vec<Pr>,
    pub state: TableState,
    pub colors: TableColors,
}

/// Log panel state
#[derive(Debug, Clone)]
pub struct LogPanelState {
    pub panel: Option<LogPanel>,
    /// Shared state for event handler to know if log panel is open
    pub log_panel_open_shared: Arc<Mutex<bool>>,
    /// Shared state for event handler to know if job list has focus
    pub job_list_focused_shared: Arc<Mutex<bool>>,
}

/// Merge bot state (wrapper around existing MergeBot)
#[derive(Debug, Clone, Default)]
pub struct MergeBotState {
    pub bot: MergeBot,
}

/// Background task status state
#[derive(Debug, Clone, Default)]
pub struct TaskState {
    pub status: Option<TaskStatus>,
}

/// Debug console state (Quake-style drop-down console)
#[derive(Debug, Clone)]
pub struct DebugConsoleState {
    pub is_open: bool,
    pub scroll_offset: usize,
    pub auto_scroll: bool,   // Follow new logs as they arrive
    pub height_percent: u16, // Height as percentage of screen (30-70)
    pub logs: crate::log_capture::LogBuffer,
    pub viewport_height: usize, // Updated during rendering for page down
}

// Re-export types from main.rs that are part of state
#[derive(Debug, Clone)]
pub struct TaskStatus {
    pub message: String,
    pub status_type: TaskStatusType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatusType {
    Running,
    Success,
    Error,
    Warning,
}

/// Newtype wrapper for GitHub PR numbers, providing type safety.
/// Can only be constructed from a Pr to prevent confusion with array indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrNumber(usize);

impl PrNumber {
    /// Create a PrNumber from a PR reference
    pub fn from_pr(pr: &Pr) -> Self {
        PrNumber(pr.number)
    }

    /// Get the raw usize value (for API calls, display, serialization, etc.)
    pub fn value(&self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct RepoData {
    pub prs: Vec<Pr>,
    pub table_state: TableState,
    pub selected_pr_numbers: HashSet<PrNumber>, // Type-safe PR numbers
    pub loading_state: LoadingState,
    pub auto_merge_queue: Vec<AutoMergePR>,
    pub operation_monitor_queue: Vec<OperationMonitor>,
}

/// Represents a PR in the auto-merge queue
#[derive(Debug, Clone)]
pub struct AutoMergePR {
    pub pr_number: usize,
    pub started_at: std::time::Instant,
    pub check_count: usize,
}

/// Type of operation being monitored
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationType {
    Rebase,
    Merge,
}

/// Represents a PR operation being monitored
#[derive(Debug, Clone)]
pub struct OperationMonitor {
    pub pr_number: usize,
    pub operation: OperationType,
    pub started_at: std::time::Instant,
    pub check_count: usize,
    pub last_head_sha: Option<String>, // Track SHA to detect rebase completion
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Eq, Clone, PartialEq)]
pub struct Repo {
    pub org: String,
    pub repo: String,
    pub branch: String,
}

impl Repo {
    pub fn new(org: &str, repo: &str, branch: &str) -> Repo {
        Repo {
            org: org.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Eq, Clone, PartialEq)]
pub enum PrFilter {
    None,
    Feat,
    Fix,
    Chore,
}

impl PrFilter {
    pub fn matches(&self, title: &str) -> bool {
        match self {
            PrFilter::None => true,
            PrFilter::Feat => title.to_lowercase().contains("feat"),
            PrFilter::Fix => title.to_lowercase().contains("fix"),
            PrFilter::Chore => title.to_lowercase().contains("chore"),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            PrFilter::None => "All",
            PrFilter::Feat => "Feat",
            PrFilter::Fix => "Fix",
            PrFilter::Chore => "Chore",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            PrFilter::None => PrFilter::Feat,
            PrFilter::Feat => PrFilter::Fix,
            PrFilter::Fix => PrFilter::Chore,
            PrFilter::Chore => PrFilter::None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LoadingState {
    #[default]
    Idle,
    Loading,
    Loaded,
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BootstrapState {
    #[default]
    NotStarted,
    LoadingRepositories,
    RestoringSession,
    LoadingFirstRepo,      // Loading the selected repo from session
    UIReady,               // First repo loaded, UI can be shown
    LoadingRemainingRepos, // Loading other repos in background
    Completed,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct TableColors {
    pub buffer_bg: ratatui::style::Color,
    pub header_bg: ratatui::style::Color,
    pub header_fg: ratatui::style::Color,
    pub row_fg: ratatui::style::Color,
    pub selected_row_style_fg: ratatui::style::Color,
    pub selected_column_style_fg: ratatui::style::Color,
    pub selected_cell_style_fg: ratatui::style::Color,
    pub normal_row_color: ratatui::style::Color,
    pub alt_row_color: ratatui::style::Color,
    pub footer_border_color: ratatui::style::Color,
}

impl TableColors {
    pub fn from_theme(theme: &crate::theme::Theme) -> Self {
        Self {
            buffer_bg: theme.bg_primary,
            header_bg: theme.table_header_bg,
            header_fg: theme.table_header_fg,
            row_fg: theme.table_row_fg,
            selected_row_style_fg: theme.selected_bg,
            selected_column_style_fg: theme.selected_bg,
            selected_cell_style_fg: theme.selected_bg,
            normal_row_color: theme.table_row_bg_normal,
            alt_row_color: theme.table_row_bg_alt,
            footer_border_color: theme.selected_fg,
        }
    }
}

impl Default for TableColors {
    fn default() -> Self {
        // Use default theme colors
        Self::from_theme(&crate::theme::Theme::default())
    }
}

// Default implementations

impl Default for UiState {
    fn default() -> Self {
        Self {
            show_shortcuts: false,
            shortcuts_scroll: 0,
            shortcuts_max_scroll: 0,
            spinner_frame: 0,
            should_quit: false,
            show_add_repo: false,
            add_repo_form: AddRepoForm::default(),
            show_add_repo_shared: Arc::new(Mutex::new(false)),
            close_pr_state: None,
            pending_key: Arc::new(Mutex::new(None)),
        }
    }
}

impl Default for ReposState {
    fn default() -> Self {
        Self {
            recent_repos: Vec::new(),
            selected_repo: 0,
            filter: PrFilter::None,
            repo_data: HashMap::new(),
            loading_state: LoadingState::default(),
            bootstrap_state: BootstrapState::default(),
            prs: Vec::new(),
            state: TableState::default(),
            colors: TableColors::default(),
        }
    }
}

impl Default for LogPanelState {
    fn default() -> Self {
        Self {
            panel: None,
            log_panel_open_shared: Arc::new(Mutex::new(false)),
            job_list_focused_shared: Arc::new(Mutex::new(true)), // Start with job list focused
        }
    }
}

impl Default for DebugConsoleState {
    fn default() -> Self {
        Self {
            is_open: false,
            scroll_offset: 0,
            auto_scroll: true,
            height_percent: 50, // 50% of screen height
            logs: crate::log_capture::DebugConsoleLogger::create_buffer(),
            viewport_height: 20, // Default, updated during rendering
        }
    }
}
