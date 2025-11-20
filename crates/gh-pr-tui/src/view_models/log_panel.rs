use crate::log::{JobMetadata, LogPanel};
use crate::theme::Theme;
use ratatui::style::Color;

/// Display-ready view model for the log panel
#[derive(Debug, Clone)]
pub struct LogPanelViewModel {
    /// PR header information (already formatted)
    pub pr_header: PrHeaderViewModel,

    /// Flattened list of visible tree rows, ready to render
    pub rows: Vec<TreeRowViewModel>,

    /// Scroll state
    pub scroll_offset: usize,
    pub viewport_height: usize,
}

#[derive(Debug, Clone)]
pub struct PrHeaderViewModel {
    pub number_text: String,  // "#123"
    pub title: String,         // "Fix: broken tests"
    pub author_text: String,   // "by sassman"
    pub number_color: Color,   // theme.status_info
    pub title_color: Color,    // theme.text_primary
    pub author_color: Color,   // theme.text_muted
}

#[derive(Debug, Clone)]
pub struct TreeRowViewModel {
    /// Complete display text (already formatted with indent, icon, status)
    pub text: String,

    /// Indentation level (for manual indent if needed)
    pub indent_level: usize,

    /// Whether this row is under cursor
    pub is_cursor: bool,

    /// Pre-determined style
    pub style: RowStyle,

    /// Additional metadata for interactions (not displayed)
    pub path: Vec<usize>, // For handling events in future
    pub node_type: NodeType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStyle {
    Normal,
    Error,   // Red text for errors
    Success, // Green for success
    Selected, // Highlighted background
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    Workflow,
    Job,
    Step,
    LogLine,
}

impl LogPanelViewModel {
    /// Transform LogPanel state into display-ready view model
    pub fn from_log_panel(panel: &LogPanel, theme: &Theme) -> Self {
        let pr_header = PrHeaderViewModel {
            number_text: format!("#{}", panel.pr_context.number),
            title: panel.pr_context.title.clone(),
            author_text: format!("by {}", panel.pr_context.author),
            number_color: theme.status_info,
            title_color: theme.text_primary,
            author_color: theme.text_muted,
        };

        let visible_paths = panel.flatten_visible_nodes();
        let mut rows = Vec::new();

        for path in visible_paths.iter() {
            let row = Self::build_row_view_model(panel, path, theme);
            rows.push(row);
        }

        Self {
            pr_header,
            rows,
            scroll_offset: panel.scroll_offset,
            viewport_height: panel.viewport_height,
        }
    }

    fn build_row_view_model(panel: &LogPanel, path: &[usize], _theme: &Theme) -> TreeRowViewModel {
        let indent_level = path.len().saturating_sub(1);
        let indent = "  ".repeat(indent_level);

        match path.len() {
            1 => {
                // Workflow node
                let workflow = &panel.workflows[path[0]];
                let expanded = panel.is_expanded(path);

                let icon = if workflow.jobs.is_empty() {
                    " "
                } else if expanded {
                    "▼"
                } else {
                    "▶"
                };

                let status_icon = if workflow.has_failures { "✗" } else { "✓" };

                let error_info = if workflow.total_errors > 0 {
                    format!(" ({} errors)", workflow.total_errors)
                } else {
                    String::new()
                };

                let text = format!(
                    "{}{} {} {}{}",
                    indent, icon, status_icon, workflow.name, error_info
                );

                TreeRowViewModel {
                    text,
                    indent_level,
                    is_cursor: path == panel.cursor_path,
                    style: if workflow.has_failures {
                        RowStyle::Error
                    } else {
                        RowStyle::Success
                    },
                    path: path.to_vec(),
                    node_type: NodeType::Workflow,
                }
            }

            2 => {
                // Job node
                let workflow = &panel.workflows[path[0]];
                let job = &workflow.jobs[path[1]];
                let expanded = panel.is_expanded(path);

                let icon = if job.steps.is_empty() {
                    " "
                } else if expanded {
                    "▼"
                } else {
                    "▶"
                };

                // Get actual job status from metadata (or infer from error count)
                let key = format!("{}:{}", workflow.name, job.name);
                let status = panel
                    .job_metadata
                    .get(&key)
                    .map(|m| m.status)
                    .unwrap_or_else(|| {
                        // Fallback: infer from error count if no metadata
                        if job.error_count > 0 {
                            crate::log::JobStatus::Failure
                        } else {
                            crate::log::JobStatus::Success
                        }
                    });

                let status_icon = Self::job_status_icon(status);

                let error_info = if job.error_count > 0 {
                    format!(" ({} errors)", job.error_count)
                } else {
                    String::new()
                };

                // Format duration (view model responsibility)
                let duration_info = Self::format_job_duration(&panel.job_metadata, workflow, job);

                let text = format!(
                    "{}├─ {} {} {}{}{}",
                    indent, icon, status_icon, job.name, error_info, duration_info
                );

                TreeRowViewModel {
                    text,
                    indent_level,
                    is_cursor: path == panel.cursor_path,
                    style: Self::job_status_style(status),
                    path: path.to_vec(),
                    node_type: NodeType::Job,
                }
            }

            3 => {
                // Step node
                let workflow = &panel.workflows[path[0]];
                let job = &workflow.jobs[path[1]];
                let step = &job.steps[path[2]];
                let expanded = panel.is_expanded(path);

                let icon = if step.lines.is_empty() {
                    " "
                } else if expanded {
                    "▼"
                } else {
                    "▶"
                };

                let status_icon = if step.error_count > 0 { "✗" } else { "✓" };

                let error_info = if step.error_count > 0 {
                    format!(" ({} errors)", step.error_count)
                } else {
                    String::new()
                };

                let text = format!(
                    "{}│  ├─ {} {}{}{}",
                    indent, icon, status_icon, step.name, error_info
                );

                TreeRowViewModel {
                    text,
                    indent_level,
                    is_cursor: path == panel.cursor_path,
                    style: if step.error_count > 0 {
                        RowStyle::Error
                    } else {
                        RowStyle::Normal
                    },
                    path: path.to_vec(),
                    node_type: NodeType::Step,
                }
            }

            4 => {
                // Log line (leaf node - no icon)
                let workflow = &panel.workflows[path[0]];
                let job = &workflow.jobs[path[1]];
                let step = &job.steps[path[2]];
                let line = &step.lines[path[3]];

                // Check if this is an error line
                let is_error = if let Some(ref cmd) = line.command {
                    matches!(cmd, gh_actions_log_parser::WorkflowCommand::Error { .. })
                } else {
                    line.display_content.to_lowercase().contains("error:")
                };

                let _is_command = line.is_command;

                // Build display text with tree prefix
                let prefix = format!("{}│     ", indent);

                // Add timestamp if available
                let timestamp_part = if panel.show_timestamps {
                    if let Some(ref timestamp) = line.timestamp {
                        format!("[{}] ", timestamp)
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                // Apply horizontal scroll to content
                let content = if panel.horizontal_scroll > 0 {
                    line.display_content
                        .chars()
                        .skip(panel.horizontal_scroll)
                        .collect::<String>()
                } else {
                    line.display_content.clone()
                };

                let text = format!("{}{}{}", prefix, timestamp_part, content);

                let style = if is_error {
                    RowStyle::Error
                } else {
                    RowStyle::Normal
                };

                TreeRowViewModel {
                    text,
                    indent_level,
                    is_cursor: path == panel.cursor_path,
                    style,
                    path: path.to_vec(),
                    node_type: NodeType::LogLine,
                }
            }

            _ => TreeRowViewModel {
                text: String::new(),
                indent_level: 0,
                is_cursor: false,
                style: RowStyle::Normal,
                path: path.to_vec(),
                node_type: NodeType::LogLine,
            },
        }
    }

    /// Format job duration for display
    /// This is view model responsibility - preparing display strings
    fn format_job_duration(
        metadata: &std::collections::HashMap<String, JobMetadata>,
        workflow: &gh_actions_log_parser::WorkflowNode,
        job: &gh_actions_log_parser::JobNode,
    ) -> String {
        let key = format!("{}:{}", workflow.name, job.name);

        if let Some(meta) = metadata.get(&key)
            && let Some(duration) = meta.duration
        {
            let secs = duration.as_secs();
            return if secs >= 60 {
                format!(", {}m {}s", secs / 60, secs % 60)
            } else {
                format!(", {}s", secs)
            };
        }

        String::new()
    }

    /// Get display icon for job status
    /// View model responsibility - translating domain status to presentation
    fn job_status_icon(status: crate::log::JobStatus) -> &'static str {
        use crate::log::JobStatus;
        match status {
            JobStatus::Success => "✓",
            JobStatus::Failure => "✗",
            JobStatus::Cancelled => "⊘",
            JobStatus::Skipped => "⊝",
            JobStatus::InProgress => "⋯",
            JobStatus::Unknown => "?",
        }
    }

    /// Get row style for job status
    /// View model responsibility - mapping domain status to display style
    fn job_status_style(status: crate::log::JobStatus) -> RowStyle {
        use crate::log::JobStatus;
        match status {
            JobStatus::Success => RowStyle::Success,
            JobStatus::Failure => RowStyle::Error,
            JobStatus::Cancelled | JobStatus::Skipped => RowStyle::Normal,
            JobStatus::InProgress => RowStyle::Normal,
            JobStatus::Unknown => RowStyle::Normal,
        }
    }
}
