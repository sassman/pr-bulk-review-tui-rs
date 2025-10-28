use std::collections::HashMap;
use ratatui::widgets::TableState;

use crate::{
    config::Config,
    log::LogPanel,
    merge_bot::MergeBot,
    pr::Pr,
    theme::Theme,
};

/// Root application state following Redux pattern
#[derive(Debug, Clone)]
pub struct AppState {
    pub ui: UiState,
    pub repos: ReposState,
    pub log_panel: LogPanelState,
    pub merge_bot: MergeBotState,
    pub task: TaskState,
    pub config: Config,
    pub theme: Theme,
}

/// UI-specific state (shortcuts panel, spinner, quit flag)
#[derive(Debug, Clone)]
pub struct UiState {
    pub show_shortcuts: bool,
    pub shortcuts_scroll: usize,
    pub shortcuts_max_scroll: usize,
    pub spinner_frame: usize,
    pub should_quit: bool,
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
    pub selected_prs: Vec<usize>,
    pub colors: TableColors,
}

/// Log panel state
#[derive(Debug, Clone)]
pub struct LogPanelState {
    pub panel: Option<LogPanel>,
}

/// Merge bot state (wrapper around existing MergeBot)
#[derive(Debug, Clone)]
pub struct MergeBotState {
    pub bot: MergeBot,
}

/// Background task status state
#[derive(Debug, Clone)]
pub struct TaskState {
    pub status: Option<TaskStatus>,
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
}

#[derive(Debug, Clone, Default)]
pub struct RepoData {
    pub prs: Vec<Pr>,
    pub table_state: TableState,
    pub selected_prs: Vec<usize>,
    pub loading_state: LoadingState,
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
    LoadingPRs,
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
    pub const fn new(color: &ratatui::style::palette::tailwind::Palette) -> Self {
        use ratatui::style::palette::tailwind;
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

impl Default for TableColors {
    fn default() -> Self {
        use ratatui::style::{palette::tailwind, Color};
        Self {
            buffer_bg: tailwind::SLATE.c950,
            header_bg: tailwind::BLUE.c500,
            header_fg: tailwind::SLATE.c200,
            row_fg: tailwind::SLATE.c200,
            selected_row_style_fg: tailwind::BLUE.c400,
            selected_column_style_fg: tailwind::BLUE.c400,
            selected_cell_style_fg: tailwind::BLUE.c600,
            normal_row_color: tailwind::SLATE.c950,
            alt_row_color: tailwind::SLATE.c900,
            footer_border_color: Color::White,
        }
    }
}

// Default implementations
impl Default for AppState {
    fn default() -> Self {
        Self {
            ui: UiState::default(),
            repos: ReposState::default(),
            log_panel: LogPanelState::default(),
            merge_bot: MergeBotState::default(),
            task: TaskState::default(),
            config: Config::default(),
            theme: Theme::default(),
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            show_shortcuts: false,
            shortcuts_scroll: 0,
            shortcuts_max_scroll: 0,
            spinner_frame: 0,
            should_quit: false,
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
            selected_prs: Vec::new(),
            colors: TableColors::default(),
        }
    }
}

impl Default for LogPanelState {
    fn default() -> Self {
        Self { panel: None }
    }
}

impl Default for MergeBotState {
    fn default() -> Self {
        Self {
            bot: MergeBot::new(),
        }
    }
}

impl Default for TaskState {
    fn default() -> Self {
        Self { status: None }
    }
}
