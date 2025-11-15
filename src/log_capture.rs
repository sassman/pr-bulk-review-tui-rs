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
}

impl DebugConsoleLogger {
    /// Create a new debug console logger with env_logger backend
    pub fn new(logs: LogBuffer) -> Self {
        // Create env_logger with default to Debug level (can be overridden by RUST_LOG)
        let env_logger = env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug) // Default to Debug if RUST_LOG not set
            .build();

        Self { logs, env_logger }
    }

    /// Create a new empty log buffer
    pub fn create_buffer() -> LogBuffer {
        Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES)))
    }
}

impl Log for DebugConsoleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Delegate to env_logger for filtering
        self.env_logger.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        // Only log if env_logger would log it
        if !self.env_logger.enabled(record.metadata()) {
            return;
        }

        // Log to env_logger first
        self.env_logger.log(record);

        // Capture to our buffer
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

    fn flush(&self) {
        self.env_logger.flush();
    }
}

/// Initialize the debug console logger
///
/// This should be called once at application startup before any logging occurs.
/// Returns the log buffer that can be shared with the UI.
pub fn init_logger() -> LogBuffer {
    let logs = DebugConsoleLogger::create_buffer();
    let logger = DebugConsoleLogger::new(logs.clone());

    log::set_boxed_logger(Box::new(logger))
        .expect("Failed to initialize logger");

    // Set max level based on env_logger configuration
    log::set_max_level(log::LevelFilter::Debug);

    // Add welcome message to show console is working
    log::info!("Debug console initialized - press ` or ~ to toggle");
    log::debug!("Logger configured with Debug level filtering");

    logs
}
