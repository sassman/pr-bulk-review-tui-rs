//! Type definitions for GitHub Actions log parsing

use serde::{Deserialize, Serialize};

/// Root structure containing all parsed logs from a workflow run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedLog {
    /// All job logs extracted from the workflow
    pub jobs: Vec<JobLog>,
}

/// A single job's log output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobLog {
    /// Name of the job (derived from filename in ZIP)
    pub name: String,
    /// All parsed log lines for this job
    pub lines: Vec<LogLine>,
}

/// A single line in the log with all metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    /// Raw text content (ANSI codes preserved)
    pub content: String,

    /// Cleaned display content (workflow commands removed, ready for display)
    pub display_content: String,

    /// Extracted timestamp (if present in GitHub Actions format)
    pub timestamp: Option<String>,

    /// Styled text segments with ANSI styling preserved
    pub styled_segments: Vec<StyledSegment>,

    /// GitHub Actions workflow command (if this line contains one)
    pub command: Option<WorkflowCommand>,

    /// Group nesting level (0 = not in group, 1+ = nested depth)
    pub group_level: usize,

    /// Title of the containing group (if inside a group)
    pub group_title: Option<String>,

    /// True if this line is pure metadata that should be hidden in normal view
    /// (e.g., ##[endgroup] with no message, empty lines with commands)
    pub is_metadata: bool,

    /// True if this line is a command invocation (had [command] prefix)
    pub is_command: bool,
}

/// A segment of text with preserved ANSI styling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyledSegment {
    /// The text content
    pub text: String,

    /// Applied ANSI styling
    pub style: AnsiStyle,
}

/// ANSI styling information
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnsiStyle {
    /// Foreground color
    pub fg_color: Option<Color>,

    /// Background color
    pub bg_color: Option<Color>,

    /// Bold text
    pub bold: bool,

    /// Faint/dim text
    pub faint: bool,

    /// Italic text
    pub italic: bool,

    /// Underlined text
    pub underline: bool,

    /// Blinking text
    pub blink: bool,

    /// Reversed foreground/background
    pub reversed: bool,

    /// Hidden text
    pub hidden: bool,

    /// Strikethrough text
    pub strikethrough: bool,
}

/// ANSI color representation supporting multiple color modes
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Color {
    /// 24-bit RGB color
    Rgb(u8, u8, u8),

    /// 256-color palette index
    Palette256(u8),

    /// Named ANSI color (0-15)
    Named(NamedColor),
}

/// Standard ANSI named colors (0-15)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum NamedColor {
    Black = 0,
    Red = 1,
    Green = 2,
    Yellow = 3,
    Blue = 4,
    Magenta = 5,
    Cyan = 6,
    White = 7,
    BrightBlack = 8,
    BrightRed = 9,
    BrightGreen = 10,
    BrightYellow = 11,
    BrightBlue = 12,
    BrightMagenta = 13,
    BrightCyan = 14,
    BrightWhite = 15,
}

/// GitHub Actions workflow command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowCommand {
    /// Start of a collapsible group: ::group::{title}
    GroupStart {
        /// Group title/name
        title: String,
    },

    /// End of a group: ::endgroup::
    GroupEnd,

    /// Error annotation: ::error file={f},line={l}::{message}
    Error {
        /// Error message
        message: String,
        /// Optional parameters
        params: CommandParams,
    },

    /// Warning annotation: ::warning::{message}
    Warning {
        /// Warning message
        message: String,
        /// Optional parameters
        params: CommandParams,
    },

    /// Debug message: ::debug::{message}
    Debug {
        /// Debug message
        message: String,
    },

    /// Notice annotation: ::notice::{message}
    Notice {
        /// Notice message
        message: String,
        /// Optional parameters
        params: CommandParams,
    },
}

/// Optional parameters for workflow commands
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandParams {
    /// Source file (file=)
    pub file: Option<String>,

    /// Line number (line=)
    pub line: Option<usize>,

    /// Column number (col=)
    pub col: Option<usize>,

    /// End column (endColumn=)
    pub end_column: Option<usize>,

    /// End line (endLine=)
    pub end_line: Option<usize>,

    /// Optional title (title=)
    pub title: Option<String>,
}

impl ParsedLog {
    /// Create a new empty parsed log
    pub fn new() -> Self {
        Self { jobs: Vec::new() }
    }
}

impl Default for ParsedLog {
    fn default() -> Self {
        Self::new()
    }
}

impl JobLog {
    /// Create a new job log
    pub fn new(name: String) -> Self {
        Self {
            name,
            lines: Vec::new(),
        }
    }
}

impl LogLine {
    /// Create a new log line with just content
    pub fn new(content: String) -> Self {
        let display_content = content.clone();
        Self {
            content,
            display_content,
            timestamp: None,
            styled_segments: Vec::new(),
            command: None,
            group_level: 0,
            group_title: None,
            is_metadata: false,
            is_command: false,
        }
    }

    /// Get the plain text content without ANSI codes
    pub fn plain_text(&self) -> String {
        self.styled_segments
            .iter()
            .map(|seg| seg.text.as_str())
            .collect::<Vec<_>>()
            .join("")
    }

    /// Check if this line should be displayed (not pure metadata)
    pub fn should_display(&self) -> bool {
        !self.is_metadata
    }
}

impl StyledSegment {
    /// Create a new unstyled segment
    pub fn new(text: String) -> Self {
        Self {
            text,
            style: AnsiStyle::default(),
        }
    }

    /// Create a segment with specific styling
    pub fn with_style(text: String, style: AnsiStyle) -> Self {
        Self { text, style }
    }
}

/// Hierarchical view of workflow logs
/// Structure: Workflow contains Jobs, Jobs contain Steps, Steps contain Lines
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogTree {
    /// Root-level workflows
    pub workflows: Vec<WorkflowNode>,
}

/// A workflow containing multiple jobs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    /// Workflow name (e.g., "CI", "Deploy")
    pub name: String,
    /// Jobs within this workflow
    pub jobs: Vec<JobNode>,
    /// Total error count across all jobs
    pub total_errors: usize,
    /// Whether any job failed
    pub has_failures: bool,
}

/// A job within a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobNode {
    /// Original job log name from ZIP
    pub name: String,
    /// Steps within this job
    pub steps: Vec<StepNode>,
    /// Total errors in this job
    pub error_count: usize,
}

/// A step within a job (corresponds to ::group:: sections)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepNode {
    /// Step name from ::group:: title
    pub name: String,
    /// Log lines in this step
    pub lines: Vec<LogLine>,
    /// Error count in this step
    pub error_count: usize,
}

impl LogTree {
    /// Create an empty log tree
    pub fn new() -> Self {
        Self {
            workflows: Vec::new(),
        }
    }

    /// Count total errors across all workflows
    pub fn total_errors(&self) -> usize {
        self.workflows.iter().map(|w| w.total_errors).sum()
    }
}

impl Default for LogTree {
    fn default() -> Self {
        Self::new()
    }
}
