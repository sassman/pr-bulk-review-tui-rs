use crate::theme::Theme;
use crate::view_models::log_panel::{LogPanelViewModel, RowStyle};
use ratatui::{prelude::*, widgets::*};

/// Render the log panel from view model
/// Pure presentation - no business logic, no data transformation
///
/// Returns the calculated viewport height for page down scrolling
pub fn render_log_panel(
    f: &mut Frame,
    view_model: &LogPanelViewModel,
    theme: &Theme,
    available_area: Rect,
) -> usize {
    // Use Clear widget to completely clear the underlying content
    f.render_widget(Clear, available_area);

    // Then render a solid background to ensure complete coverage
    let background = Block::default().style(Style::default().bg(theme.bg_panel));
    f.render_widget(background, available_area);

    // Split area into PR header (3 lines) and log content
    let card_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // PR context header
            Constraint::Min(0),    // Log content
        ])
        .split(available_area);

    // Render PR context header (data is pre-formatted in view model)
    render_pr_header(f, view_model, theme, card_chunks[0]);

    // Render log content and return viewport height
    render_log_tree(f, view_model, theme, card_chunks[1])
}

/// Render PR context header
fn render_pr_header(f: &mut Frame, view_model: &LogPanelViewModel, theme: &Theme, area: Rect) {
    let pr_header_text = vec![
        Line::from(vec![
            Span::styled(
                view_model.pr_header.number_text.clone(),
                Style::default()
                    .fg(view_model.pr_header.number_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                view_model.pr_header.title.clone(),
                Style::default()
                    .fg(view_model.pr_header.title_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            view_model.pr_header.author_text.clone(),
            Style::default().fg(view_model.pr_header.author_color),
        )),
    ];

    let pr_header = Paragraph::new(pr_header_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(
                Style::default()
                    .fg(theme.accent_primary)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(theme.bg_panel)),
    );

    f.render_widget(pr_header, area);
}

/// Render the tree view - simple iteration over pre-computed rows
fn render_log_tree(f: &mut Frame, view_model: &LogPanelViewModel, theme: &Theme, area: Rect) -> usize {
    let visible_height = area.height.saturating_sub(2) as usize;

    if view_model.rows.is_empty() {
        let empty_msg = Paragraph::new("No build logs found")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Build Logs ")
                    .border_style(Style::default().fg(theme.accent_primary))
                    .style(Style::default().bg(theme.bg_panel)),
            )
            .style(Style::default().fg(theme.text_muted).bg(theme.bg_panel))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(empty_msg, area);
        return 0;
    }

    // Build table rows - simple iteration, no complex logic!
    let mut rows = Vec::new();
    let start = view_model.scroll_offset;
    let end = (start + visible_height).min(view_model.rows.len());

    for row_vm in &view_model.rows[start..end] {
        // Apply style based on pre-determined row style
        let style = match row_vm.style {
            RowStyle::Normal => {
                if row_vm.is_cursor {
                    Style::default()
                        .fg(theme.text_primary)
                        .bg(theme.selected_bg)
                } else {
                    Style::default().fg(theme.text_primary).bg(theme.bg_panel)
                }
            }
            RowStyle::Error => {
                if row_vm.is_cursor {
                    // Use yellow text for selected error lines (for visibility against pink background)
                    Style::default()
                        .fg(theme.active_fg)
                        .bg(theme.selected_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(theme.status_error)
                        .bg(theme.bg_panel)
                        .add_modifier(Modifier::BOLD)
                }
            }
            RowStyle::Success => {
                if row_vm.is_cursor {
                    Style::default()
                        .fg(theme.text_primary)
                        .bg(theme.selected_bg)
                } else {
                    Style::default().fg(theme.text_primary).bg(theme.bg_panel)
                }
            }
            RowStyle::Selected => Style::default()
                .fg(theme.text_primary)
                .bg(theme.selected_bg),
        };

        // Text is pre-formatted - just display it!
        rows.push(Row::new(vec![Cell::from(row_vm.text.clone())]).style(style));
    }

    let table = Table::new(rows, vec![Constraint::Percentage(100)])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Build Logs | j/k: navigate, Enter: toggle, n: next error, x: close ")
                .border_style(Style::default().fg(theme.accent_primary))
                .style(Style::default().bg(theme.bg_panel)),
        )
        .style(Style::default().bg(theme.bg_panel));

    f.render_widget(table, area);
    visible_height
}
