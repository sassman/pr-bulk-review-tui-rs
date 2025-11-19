use anyhow::Result;

/// Action enum - represents all possible actions in the application
/// Actions are dispatched to the reducer to update state
#[derive(Debug, Clone)]
pub enum Action {
    // User-initiated actions
    Bootstrap,
    Rebase,
    RefreshCurrentRepo,
    ReloadRepo(usize), // Reload specific repo by index (e.g., after PR merged)
    RerunFailedJobs,
    CycleFilter,
    SelectNextRepo,
    SelectPreviousRepo,
    SelectRepoByIndex(usize),
    TogglePrSelection,
    NavigateToNextPr,
    NavigateToPreviousPr,
    ClearPrSelection,
    MergeSelectedPrs,
    ApprovePrs,
    StartMergeBot,
    MergeBotTick, // Internal action for merge bot processing
    OpenCurrentPrInBrowser,
    OpenBuildLogs,
    OpenInIDE,
    CloseLogPanel,
    // Log panel - tree navigation
    SelectNextJob,
    SelectPrevJob,
    FocusJobList,
    FocusLogViewer,
    ToggleTreeNode, // Toggle expand/collapse at cursor
    // Log panel - log viewer scrolling
    ScrollLogPanelUp,
    ScrollLogPanelDown,
    PageLogPanelDown,
    ScrollLogPanelLeft,
    ScrollLogPanelRight,
    // Log panel - step and error navigation
    NextStep,
    PrevStep,
    NextError,      // Jump to next step/job with errors
    PrevError,      // Jump to previous step/job with errors
    NextLogSection, // Error navigation (kept for backwards compat)
    PrevLogSection, // Error navigation (kept for backwards compat)
    ToggleTimestamps,
    ToggleShortcuts,
    ScrollShortcutsUp,
    ScrollShortcutsDown,

    // Add repository popup
    ShowAddRepoPopup,
    HideAddRepoPopup,
    AddRepoFormInput(char),
    AddRepoFormBackspace,
    AddRepoFormNextField,
    AddRepoFormSubmit,

    // Close PR popup
    ShowClosePrPopup,
    HideClosePrPopup,
    ClosePrFormInput(char),
    ClosePrFormBackspace,
    ClosePrFormSubmit,

    // Repository management
    DeleteCurrentRepo,

    // State update actions (dispatched internally)
    SetBootstrapState(crate::state::BootstrapState),
    SetLoadingState(crate::state::LoadingState),
    SetTaskStatus(Option<crate::state::TaskStatus>),
    SetReposLoading(Vec<usize>), // Set multiple repos to loading state
    TickSpinner,                 // Increment spinner animation frame

    // Background task completion notifications
    BootstrapComplete(Result<BootstrapResult, String>),
    RepoLoadingStarted(usize), // Sent when we start fetching repo data
    RepoDataLoaded(usize, Result<Vec<crate::pr::Pr>, String>),
    RefreshComplete(Result<Vec<crate::pr::Pr>, String>),
    MergeStatusUpdated(usize, usize, crate::pr::MergeableStatus), // repo_index, pr_number, status
    RebaseStatusUpdated(usize, usize, bool), // repo_index, pr_number, needs_rebase
    CommentCountUpdated(usize, usize, usize), // repo_index, pr_number, comment_count
    RebaseComplete(Result<(), String>),
    MergeComplete(Result<(), String>),
    RerunJobsComplete(Result<(), String>),
    ApprovalComplete(Result<(), String>),
    ClosePrComplete(Result<(), String>),
    PRMergedConfirmed(usize, usize, bool), // repo_index, pr_number, is_merged
    BuildLogsLoaded(
        Vec<(crate::log::JobMetadata, gh_actions_log_parser::JobLog)>,
        crate::log::PrContext,
    ),
    IDEOpenComplete(Result<(), String>),

    // Auto-merge queue management
    AddToAutoMergeQueue(usize, usize),      // repo_index, pr_number
    RemoveFromAutoMergeQueue(usize, usize), // repo_index, pr_number
    AutoMergeStatusCheck(usize, usize),     // repo_index, pr_number - periodic check

    // Operation monitoring (rebase/merge progress tracking)
    StartOperationMonitor(usize, usize, crate::state::OperationType), // repo_index, pr_number, operation
    OperationMonitorCheck(usize, usize), // repo_index, pr_number - periodic check
    RemoveFromOperationMonitor(usize, usize), // repo_index, pr_number

    // Debug console (Quake-style drop-down)
    ToggleDebugConsole,
    ScrollDebugConsoleUp,
    ScrollDebugConsoleDown,
    PageDebugConsoleDown,
    ToggleDebugAutoScroll,
    ClearDebugLogs,

    // Viewport height updates (for page down scrolling)
    UpdateLogPanelViewport(usize),
    UpdateDebugConsoleViewport(usize),

    Quit,
    None,
}

/// Result type for bootstrap action
#[derive(Debug, Clone)]
pub struct BootstrapResult {
    pub repos: Vec<crate::Repo>,
    pub selected_repo: usize,
}
