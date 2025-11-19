//! Main parsing logic for GitHub Actions workflow logs

use crate::ansi::parse_ansi_line;
use crate::commands::parse_command;
use crate::types::{JobLog, LogLine, ParsedLog, WorkflowCommand};
use std::io::{Cursor, Read};
use thiserror::Error;
use zip::ZipArchive;

/// Errors that can occur during log parsing
#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Failed to read ZIP archive: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("Failed to read file from ZIP: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid UTF-8 in log content")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// Parse workflow logs from a ZIP file
///
/// GitHub Actions provides logs as a ZIP file where each job has its own log file.
/// This function extracts and parses all job logs with ANSI color preservation
/// and workflow command recognition.
///
/// # Arguments
///
/// * `zip_data` - Raw bytes of the ZIP file from GitHub Actions API
///
/// # Returns
///
/// A `ParsedLog` containing all jobs and their parsed log lines, or an error.
///
/// # Example
///
/// ```no_run
/// # use gh_actions_log_parser::parse_workflow_logs;
/// let zip_data: &[u8] = &[]; // From GitHub API
/// let parsed = parse_workflow_logs(zip_data)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn parse_workflow_logs(zip_data: &[u8]) -> Result<ParsedLog, ParseError> {
    let cursor = Cursor::new(zip_data);
    let mut archive = ZipArchive::new(cursor)?;

    let mut jobs = Vec::new();

    // Process each file in the ZIP (each file is a job log)
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let file_name = file.name().to_string();

        // Skip directories
        if file.is_dir() {
            continue;
        }

        // Read the log file content
        let mut content_bytes = Vec::new();
        file.read_to_end(&mut content_bytes)?;
        let content = String::from_utf8(content_bytes)?;

        // Clean the job name (remove .txt extension and number prefix)
        let clean_name = clean_job_name(&file_name);

        // Parse the job log
        let job_log = parse_job_log(&clean_name, &content);
        jobs.push(job_log);
    }

    Ok(ParsedLog { jobs })
}

/// Clean job name from GitHub Actions ZIP file
/// Removes `.txt` extension and leading `{number}_` prefix
/// Example: `2_check (ubuntu-latest).txt` -> `check (ubuntu-latest)`
fn clean_job_name(file_name: &str) -> String {
    let mut name = file_name.to_string();

    // Remove .txt extension
    if name.ends_with(".txt") {
        name = name[..name.len() - 4].to_string();
    }

    // Remove leading number prefix (e.g., "2_" from "2_check (ubuntu-latest)")
    if let Some(underscore_pos) = name.find('_') {
        // Check if everything before underscore is a number
        if name[..underscore_pos].chars().all(|c| c.is_ascii_digit()) {
            name = name[underscore_pos + 1..].to_string();
        }
    }

    name
}

/// Parse a single job's log content
fn parse_job_log(job_name: &str, content: &str) -> JobLog {
    let mut lines = Vec::new();
    let mut group_tracker = GroupTracker::new();

    for raw_line in content.lines() {
        // Extract timestamp if present (GitHub Actions format)
        let (timestamp, line_content) = extract_timestamp(raw_line);

        // Check for [command] prefix and remove it
        let (is_command, line_after_command_prefix) =
            if let Some(stripped) = line_content.strip_prefix("[command]") {
                (true, stripped) // Remove "[command]" prefix
            } else {
                (false, line_content)
            };

        // Parse ANSI codes to get styled segments
        let styled_segments = parse_ansi_line(line_after_command_prefix);

        // Get plain text for command parsing (without ANSI)
        let plain_text: String = styled_segments
            .iter()
            .map(|seg| seg.text.as_str())
            .collect();

        // Parse workflow command if present
        let (command, display_content, is_metadata) = match parse_command(&plain_text) {
            Some((cmd, cleaned_msg)) => {
                // Update group tracker based on command
                match &cmd {
                    WorkflowCommand::GroupStart { title } => {
                        group_tracker.enter_group(title.clone());
                    }
                    WorkflowCommand::GroupEnd => {
                        group_tracker.exit_group();
                    }
                    _ => {}
                }

                // Determine if this is pure metadata (should be hidden)
                let is_metadata = match &cmd {
                    WorkflowCommand::GroupStart { .. } => true, // Hide ##[group] lines
                    WorkflowCommand::GroupEnd => cleaned_msg.is_empty(),
                    WorkflowCommand::Debug { message } if message.is_empty() => true,
                    _ => false,
                };

                (Some(cmd), cleaned_msg, is_metadata)
            }
            None => (None, plain_text.clone(), false),
        };

        // Get current group state
        let (group_level, group_title) = group_tracker.current_group();

        // Create log line
        lines.push(LogLine {
            content: line_content.to_string(), // Keep raw content with ANSI
            display_content,
            timestamp,
            styled_segments,
            command,
            group_level,
            group_title,
            is_metadata,
            is_command,
        });
    }

    JobLog {
        name: job_name.to_string(),
        lines,
    }
}

/// Convert a JobLog to a hierarchical JobNode with steps
pub fn job_log_to_tree(job_log: JobLog) -> crate::types::JobNode {
    let mut steps: Vec<crate::types::StepNode> = Vec::new();
    let mut current_step_lines: Vec<LogLine> = Vec::new();
    let mut current_step_name: Option<String> = None;

    for line in job_log.lines {
        // Check for step boundaries - only GroupStart creates a new step
        // GroupEnd is just metadata, actual content continues after it
        if let Some(ref cmd) = line.command
            && let WorkflowCommand::GroupStart { title } = cmd
        {
            // Save previous step if exists
            if let Some(step_name) = current_step_name.take() {
                let error_count = count_step_errors(&current_step_lines);
                steps.push(crate::types::StepNode {
                    name: step_name,
                    lines: current_step_lines.clone(),
                    error_count,
                });
                current_step_lines.clear();
            }

            // Start new step
            current_step_name = Some(title.clone());
        }

        // Add non-metadata lines to current step
        // This includes all lines AFTER ##[endgroup] until next ##[group]
        if !line.is_metadata {
            current_step_lines.push(line);
        }
    }

    // Save final step if exists
    if let Some(step_name) = current_step_name {
        let error_count = count_step_errors(&current_step_lines);
        steps.push(crate::types::StepNode {
            name: step_name,
            lines: current_step_lines,
            error_count,
        });
    }

    // Sort steps alphabetically by name
    steps.sort_by(|a, b| a.name.cmp(&b.name));

    // Calculate total job error count
    let error_count: usize = steps.iter().map(|s| s.error_count).sum();

    crate::types::JobNode {
        name: job_log.name,
        steps,
        error_count,
    }
}

/// Count errors in a list of log lines
fn count_step_errors(lines: &[LogLine]) -> usize {
    lines
        .iter()
        .filter(|line| {
            if let Some(ref cmd) = line.command {
                matches!(cmd, WorkflowCommand::Error { .. })
            } else {
                line.display_content.to_lowercase().contains("error:")
            }
        })
        .count()
}

/// Extract timestamp from GitHub Actions log line format
///
/// GitHub Actions logs have timestamps in the format:
/// `2024-01-15T10:30:00.1234567Z some log line`
///
/// Returns (timestamp, content) where timestamp is Some if found, None otherwise.
fn extract_timestamp(line: &str) -> (Option<String>, &str) {
    // Check if line starts with ISO 8601 timestamp
    if line.len() >= 28 {
        // Minimum length for timestamp: "2024-01-15T10:30:00.123456Z"
        let chars: Vec<char> = line.chars().collect();
        if chars.len() >= 28
            && chars[4] == '-'
            && chars[7] == '-'
            && chars[10] == 'T'
            && chars[13] == ':'
            && chars[16] == ':'
            && (chars[19] == '.' || chars[19] == 'Z')
        {
            // Find where timestamp ends
            if let Some(pos) = line.find("Z ") {
                // 'Z' followed by space (normal case)
                let timestamp = line[..=pos].to_string(); // Include the 'Z'
                let content = &line[pos + 2..]; // Skip "Z " to get content
                return (Some(timestamp), content);
            } else if let Some(pos) = line.find('Z') {
                // 'Z' at end of line (empty line case)
                // Verify this is actually the timestamp 'Z' by checking position
                if (20..30).contains(&pos) {
                    // 'Z' should be around position 20-29 in timestamp
                    let timestamp = line[..=pos].to_string(); // Include the 'Z'
                    let content = &line[pos + 1..]; // Everything after 'Z' (should be empty or whitespace)
                    return (Some(timestamp), content);
                }
            }
        }
    }

    // No timestamp found
    (None, line)
}

/// Tracks group nesting state during parsing
struct GroupTracker {
    /// Stack of active groups (LIFO)
    stack: Vec<String>,
}

impl GroupTracker {
    fn new() -> Self {
        Self { stack: Vec::new() }
    }

    fn enter_group(&mut self, title: String) {
        self.stack.push(title);
    }

    fn exit_group(&mut self) {
        self.stack.pop();
    }

    fn current_group(&self) -> (usize, Option<String>) {
        let level = self.stack.len();
        let title = self.stack.last().cloned();
        (level, title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_timestamp() {
        let line = "2024-01-15T10:30:00.1234567Z Running tests";
        let (ts, content) = extract_timestamp(line);
        assert_eq!(ts, Some("2024-01-15T10:30:00.1234567Z".to_string()));
        assert_eq!(content, "Running tests");
    }

    #[test]
    fn test_no_timestamp() {
        let line = "Just a regular log line";
        let (ts, content) = extract_timestamp(line);
        assert_eq!(ts, None);
        assert_eq!(content, "Just a regular log line");
    }

    #[test]
    fn test_extract_timestamp_empty_line() {
        // Test timestamp at end of line with no content (empty line case)
        let line = "2025-11-15T19:57:15.2102930Z";
        let (ts, content) = extract_timestamp(line);
        assert_eq!(ts, Some("2025-11-15T19:57:15.2102930Z".to_string()));
        assert_eq!(content, "");
    }

    #[test]
    fn test_extract_timestamp_with_trailing_whitespace() {
        // Test timestamp followed by whitespace only
        let line = "2025-11-15T19:57:15.2102930Z   ";
        let (ts, content) = extract_timestamp(line);
        assert_eq!(ts, Some("2025-11-15T19:57:15.2102930Z".to_string()));
        assert_eq!(content, "  "); // Two spaces after the Z
    }

    #[test]
    fn test_group_tracker() {
        let mut tracker = GroupTracker::new();
        assert_eq!(tracker.current_group(), (0, None));

        tracker.enter_group("Build".to_string());
        assert_eq!(tracker.current_group(), (1, Some("Build".to_string())));

        tracker.enter_group("Tests".to_string());
        assert_eq!(tracker.current_group(), (2, Some("Tests".to_string())));

        tracker.exit_group();
        assert_eq!(tracker.current_group(), (1, Some("Build".to_string())));

        tracker.exit_group();
        assert_eq!(tracker.current_group(), (0, None));
    }

    #[test]
    fn test_clean_job_name() {
        // Test removing .txt extension and number prefix
        assert_eq!(
            clean_job_name("2_check (ubuntu-latest).txt"),
            "check (ubuntu-latest)"
        );

        // Test removing .txt only
        assert_eq!(clean_job_name("build.txt"), "build");

        // Test removing number prefix only
        assert_eq!(clean_job_name("1_test"), "test");

        // Test no changes needed
        assert_eq!(clean_job_name("my-job"), "my-job");

        // Test underscore in job name (not a number prefix)
        assert_eq!(clean_job_name("my_job.txt"), "my_job");
    }
}
