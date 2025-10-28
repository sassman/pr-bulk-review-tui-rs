use anyhow::Result;
use ratatui::{
    prelude::*,
    style::palette::tailwind,
    widgets::*,
};

#[derive(Debug, Clone)]
pub struct LogPanel {
    pub log_sections: Vec<LogSection>,
    pub scroll_offset: usize,
    pub current_section: usize,
    pub horizontal_scroll: usize,
    pub pr_context: PrContext,
    pub show_timestamps: bool,
}

#[derive(Debug, Clone)]
pub struct PrContext {
    pub number: usize,
    pub title: String,
    pub author: String,
}

#[derive(Debug, Clone)]
pub struct LogSection {
    pub step_name: String,
    pub error_lines: Vec<String>,
    pub has_extracted_errors: bool,
}

/// Job log entry containing the job name and its log content
#[derive(Debug)]
pub struct JobLog {
    pub job_name: String,
    pub content: Vec<String>,
}

/// Strip ANSI escape codes and other control characters from a string
pub fn strip_ansi_codes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Found escape sequence start
            // Consume the '[' if present
            if chars.peek() == Some(&'[') {
                chars.next();
                // Consume until we hit a letter (end of ANSI sequence)
                while let Some(&next_ch) = chars.peek() {
                    chars.next();
                    if next_ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                // Other escape sequences, consume next char
                chars.next();
            }
        } else if ch == '\r' {
            // Skip carriage returns
            continue;
        } else if ch.is_control() && ch != '\n' && ch != '\t' {
            // Skip other control characters except newlines and tabs
            continue;
        } else {
            result.push(ch);
        }
    }

    result
}

/// Parse workflow logs from a zip file downloaded from GitHub API
/// Returns a vector of job logs (one per job in the workflow)
pub fn parse_workflow_logs_zip(zip_data: &bytes::Bytes) -> Result<Vec<JobLog>> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let cursor = Cursor::new(zip_data.as_ref());
    let mut archive = ZipArchive::new(cursor)?;

    let mut job_logs = Vec::new();

    // Process each file in the zip (each file is a job log)
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let file_name = file.name().to_string();

        // Skip directories
        if file.is_dir() {
            continue;
        }

        // Read the log file content
        let mut content = String::new();
        file.read_to_string(&mut content)?;

        // Strip ANSI codes and clean up the content
        let cleaned_content = strip_ansi_codes(&content);
        let log_lines: Vec<String> = cleaned_content.lines().map(|s| s.to_string()).collect();

        job_logs.push(JobLog {
            job_name: file_name,
            content: log_lines,
        });
    }

    Ok(job_logs)
}

/// Extract error context from build logs
/// Returns lines around errors (±5 lines before and after)
/// Returns empty vector if no meaningful errors found (user will see full log instead)
pub fn extract_error_context(log_text: &str, _step_name: &str) -> Vec<String> {
    let lines: Vec<&str> = log_text.lines().collect();
    let mut error_indices = Vec::new();

    // Find lines that contain error indicators
    // Prioritize lines that START with "error" for build/compilation errors
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();

        // Skip lines that are clearly not errors (comments)
        if lower.starts_with("# error") || lower.starts_with("// error") {
            continue;
        }

        // PRIORITY 1: Lines that start with "error" (typical build errors)
        if lower.starts_with("error:")
            || lower.starts_with("error[")
            || lower.starts_with("error ")
        {
            error_indices.push(idx);
            continue;
        }

        // PRIORITY 2: Lines that start with other error indicators
        if lower.starts_with("failed:")
            || lower.starts_with("failure:")
            || lower.starts_with("fatal:")
        {
            error_indices.push(idx);
            continue;
        }

        // PRIORITY 3: Lines containing error in context (less reliable)
        if lower.contains("error:")
            || lower.contains("failed:")
            || lower.contains("✗")
            || lower.contains("❌")
            || (lower.contains("error") && (lower.contains("line") || lower.contains("at ")))
        {
            error_indices.push(idx);
        }
    }

    // Only return error context if we found at least 2 error lines
    if error_indices.len() < 2 {
        return Vec::new();
    }

    // For each error, extract context (±5 lines)
    let mut result = Vec::new();
    let mut covered_ranges = Vec::new();

    for (idx, &error_idx) in error_indices.iter().enumerate() {
        let start = error_idx.saturating_sub(5);
        let end = (error_idx + 10).min(lines.len()); // Keep 10 lines after for context

        // Check if this range overlaps with already covered ranges
        let mut should_add = true;
        for &(covered_start, covered_end) in &covered_ranges {
            if start <= covered_end && end >= covered_start {
                should_add = false;
                break;
            }
        }

        if should_add {
            covered_ranges.push((start, end));

            // Add skip indicator if we're not at the beginning and this is the first error
            if idx == 0 && start > 0 {
                result.push(format!("... [skipped {} lines] ...", start));
                result.push("".to_string());
            }

            for i in start..end {
                result.push(lines[i].to_string());
            }

            // Add separator between different error contexts
            if idx < error_indices.len() - 1 {
                result.push("".to_string());
                result.push("─".repeat(80));
                result.push("".to_string());
            }
        }
    }

    result
}

/// Render the log panel as a card overlay with PR context header
/// Takes the available area (excluding top tabs and bottom panels)
pub fn render_log_panel_card(f: &mut Frame, panel: &LogPanel, colors: &crate::state::TableColors, available_area: Rect) {
    // Use Clear widget to completely clear the underlying content
    f.render_widget(Clear, available_area);

    // Then render a solid background to ensure complete coverage
    let background = Block::default()
        .style(Style::default().bg(tailwind::SLATE.c800));
    f.render_widget(background, available_area);

    // Use the full available area (same dimensions as PR panel)
    let card_area = available_area;

    // Split card into PR header (3 lines) and log content
    let card_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // PR context header
            Constraint::Min(0),     // Log content
        ])
        .split(card_area);

    // Render PR context header
    let pr_header_text = vec![
        Line::from(vec![
            Span::styled(
                format!("#{} ", panel.pr_context.number),
                Style::default()
                    .fg(tailwind::BLUE.c400)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                panel.pr_context.title.clone(),
                Style::default()
                    .fg(tailwind::SLATE.c100)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            format!("by {}", panel.pr_context.author),
            Style::default().fg(tailwind::SLATE.c400),
        )),
    ];

    let pr_header = Paragraph::new(pr_header_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(
                Style::default()
                    .fg(tailwind::CYAN.c400)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(tailwind::SLATE.c800)),
    );

    f.render_widget(pr_header, card_chunks[0]);

    // Render log content in the remaining area
    render_log_panel_content(f, panel, card_chunks[1], colors);
}

/// Render the log panel showing build failure logs using a Table widget
fn render_log_panel_content(f: &mut Frame, panel: &LogPanel, area: Rect, _colors: &crate::TableColors) {
    // Build the log rows with timestamp extraction
    let mut log_rows: Vec<(String, String)> = Vec::new(); // (timestamp, content)

    for (idx, section) in panel.log_sections.iter().enumerate() {
        // Section header
        let header = if idx == panel.current_section {
            format!("━━━ {} (current) ━━━", section.step_name)
        } else {
            format!("━━━ {} ━━━", section.step_name)
        };
        log_rows.push((String::new(), header)); // No timestamp for headers

        // Error lines - parse timestamp if present
        for line in &section.error_lines {
            let (timestamp, content) = extract_timestamp(line);
            log_rows.push((timestamp, content));
        }

        // Separator between sections
        if idx < panel.log_sections.len() - 1 {
            log_rows.push((String::new(), "─".repeat(80)));
        }
    }

    // Calculate scrolling
    let visible_height = area.height.saturating_sub(2) as usize; // -2 for borders
    let start_line = panel
        .scroll_offset
        .min(log_rows.len().saturating_sub(visible_height));
    let end_line = (start_line + visible_height).min(log_rows.len());

    // Build table rows with styling
    let rows: Vec<Row> = log_rows[start_line..end_line]
        .iter()
        .map(|(timestamp, content)| {
            let trimmed = content.trim();
            let lower_trimmed = trimmed.to_lowercase();

            // Determine style based on content
            let style = if content.starts_with("━━━") {
                // Section headers - bright cyan
                Style::default()
                    .fg(tailwind::CYAN.c300)
                    .add_modifier(Modifier::BOLD)
            } else if lower_trimmed.starts_with("error:")
                || lower_trimmed.starts_with("error[")
                || lower_trimmed.starts_with("error ")
                || lower_trimmed.starts_with("failed:")
                || lower_trimmed.starts_with("failure:")
                || lower_trimmed.starts_with("fatal:")
            {
                // Lines STARTING with error indicators - bright red with bold
                Style::default()
                    .fg(tailwind::RED.c400)
                    .add_modifier(Modifier::BOLD)
            } else if content.contains("error") || content.contains("Error") || content.contains("ERROR") {
                // Lines containing error anywhere - softer red
                Style::default().fg(tailwind::RED.c500)
            } else if lower_trimmed.starts_with("warning:") || lower_trimmed.starts_with("warn:") {
                // Lines starting with warning - bright yellow bold
                Style::default()
                    .fg(tailwind::YELLOW.c400)
                    .add_modifier(Modifier::BOLD)
            } else if content.contains("warning") || content.contains("Warning") || content.contains("WARN") {
                // Lines containing warning - softer yellow
                Style::default().fg(tailwind::YELLOW.c500)
            } else {
                // Normal lines - light slate
                Style::default().fg(tailwind::SLATE.c100)
            };

            // Create cells based on timestamp visibility
            // IMPORTANT: Add background color to each cell to prevent bleed-through
            if panel.show_timestamps {
                Row::new(vec![
                    Cell::from(timestamp.clone()).style(
                        Style::default()
                            .fg(tailwind::SLATE.c500)
                            .bg(tailwind::SLATE.c800)
                    ),
                    Cell::from(content.clone()).style(style.bg(tailwind::SLATE.c800)),
                ])
            } else {
                // When timestamps hidden, use single column
                Row::new(vec![Cell::from(content.clone()).style(style.bg(tailwind::SLATE.c800))])
            }
        })
        .collect();

    let scroll_info = format!(
        " Build Logs [{}/{}] | n: next section, j/k or ↑/↓: scroll, t: toggle timestamps, x/Esc: close ",
        panel.current_section + 1,
        panel.log_sections.len()
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(scroll_info)
        .border_style(
            Style::default()
                .fg(tailwind::CYAN.c400)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(tailwind::SLATE.c800));

    // Configure table with or without timestamp column
    let widths = if panel.show_timestamps {
        vec![Constraint::Length(30), Constraint::Min(0)]
    } else {
        vec![Constraint::Percentage(100)]
    };

    let table = Table::new(rows, widths)
        .block(block)
        .column_spacing(0) // No spacing between columns to prevent gaps
        .style(
            Style::default()
                .fg(tailwind::SLATE.c100)
                .bg(tailwind::SLATE.c800),
        );

    f.render_widget(table, area);
}

/// Extract timestamp from log line if present
/// Returns (timestamp, content) tuple
fn extract_timestamp(line: &str) -> (String, String) {
    // GitHub Actions logs format: "2024-01-15T10:30:00.1234567Z some log line"
    if line.len() > 30 {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() > 30
            && chars[4] == '-'
            && chars[7] == '-'
            && chars[10] == 'T'
            && chars[13] == ':'
            && chars[16] == ':'
            && (chars[19] == '.' || chars[19] == 'Z')
        {
            // Find where timestamp ends (look for 'Z' followed by space)
            if let Some(pos) = line.find("Z ") {
                let timestamp = line[..pos + 1].to_string(); // Include the 'Z'
                let content = line[pos + 2..].to_string(); // Skip "Z " to get content
                return (timestamp, content);
            }
        }
    }
    // No timestamp found
    (String::new(), line.to_string())
}
