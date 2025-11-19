/// Debug console log capture system
///
/// This module provides a custom logger that captures all log messages
/// into a thread-safe circular buffer for display in the debug console.
use chrono::{DateTime, Utc};
use log::{Level, Log, Metadata, Record};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Maximum number of log entries to keep in memory
const MAX_LOG_ENTRIES: usize = 1000;

/// A single log entry with timestamp and metadata
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: Level,
    pub target: String,
    pub message: String,
}

/// Thread-safe log buffer shared between logger and UI
pub type LogBuffer = Arc<Mutex<VecDeque<LogEntry>>>;

/// Custom logger that captures logs to both env_logger and our buffer
pub struct DebugConsoleLogger {
    logs: LogBuffer,
    env_logger: env_logger::Logger,
    console_filter: env_logger::Logger,
}

impl DebugConsoleLogger {
    /// Create a new debug console logger with env_logger backend
    pub fn new(logs: LogBuffer) -> Self {
        // Create env_logger for terminal output only (defaults to Error level)
        let env_logger = env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Error)
            .build();

        // Create separate filter for console buffer
        // Default: Only show logs from this crate (gh_pr_tui=debug)
        // Override with RUST_LOG env var if set
        let console_filter = if std::env::var("RUST_LOG").is_ok() {
            // User set RUST_LOG, respect it
            env_logger::Builder::from_default_env().build()
        } else {
            // No RUST_LOG set, default to this crate only at Debug level
            env_logger::Builder::new()
                .filter_module("gh_pr_tui", log::LevelFilter::Debug)
                .build()
        };

        Self {
            logs,
            env_logger,
            console_filter,
        }
    }

    /// Create a new empty log buffer
    pub fn create_buffer() -> LogBuffer {
        Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES)))
    }
}

impl Log for DebugConsoleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Enable if either console or terminal wants it
        self.console_filter.enabled(metadata) || self.env_logger.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        // Capture to console buffer if console_filter allows it (respects RUST_LOG)
        if self.console_filter.enabled(record.metadata()) {
            let entry = LogEntry {
                timestamp: Utc::now(),
                level: record.level(),
                target: record.target().to_string(),
                message: format!("{}", record.args()),
            };

            if let Ok(mut logs) = self.logs.lock() {
                // Remove oldest entry if we've hit the limit
                if logs.len() >= MAX_LOG_ENTRIES {
                    logs.pop_front();
                }
                logs.push_back(entry);
            }
        }

        // Separately, log to terminal via env_logger (defaults to Error only)
        if self.env_logger.enabled(record.metadata()) {
            self.env_logger.log(record);
        }
    }

    fn flush(&self) {
        self.env_logger.flush();
    }
}

/// Initialize the debug console logger
///
/// This should be called once at application startup before any logging occurs.
/// Returns the log buffer that can be shared with the UI.
///
/// # Default Behavior
///
/// By default, the debug console shows **only logs from this crate** (gh_pr_tui)
/// at Debug level. This filters out noise from dependencies like octocrab, tokio, ratatui, etc.
///
/// # Filtering with RUST_LOG
///
/// You can override the default filtering with RUST_LOG:
///
/// - No RUST_LOG (default): Only logs from this crate at Debug+ level
/// - `RUST_LOG=debug`: All Debug+ logs from all modules (including dependencies)
/// - `RUST_LOG=gh_pr_tui::task=debug`: Only logs from the task module
/// - `RUST_LOG=info`: Only Info+ logs from all modules
///
/// Note: Crate name uses underscores (gh_pr_tui), not hyphens!
///
/// Terminal output is always Error-level only (keeps terminal clean).
pub fn init_logger() -> LogBuffer {
    let logs = DebugConsoleLogger::create_buffer();
    let logger = DebugConsoleLogger::new(logs.clone());

    log::set_boxed_logger(Box::new(logger)).expect("Failed to initialize logger");

    // Set max level based on env_logger configuration
    log::set_max_level(log::LevelFilter::Debug);

    // Add welcome message to show console is working
    log::info!("Debug console initialized - press ` or ~ to toggle");
    log::debug!("Logger configured with Debug level filtering");

    logs
}
