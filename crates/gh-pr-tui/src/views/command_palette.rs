use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::*,
    widgets::*,
};

use crate::App;

/// Render the command palette popup
/// Pure presentation - uses pre-computed view model
pub fn render_command_palette(f: &mut Frame, area: Rect, app: &App) {
    use ratatui::widgets::{Clear, Wrap};

    let palette = match &app.store.state().ui.command_palette {
        Some(p) => p,
        None => return,
    };

    // Get view model - if not ready yet, return early
    let Some(ref vm) = palette.view_model else {
        return;
    };

    let theme = &app.store.state().theme;

    // Calculate centered area (70% width, 60% height)
    let popup_width = (area.width * 70 / 100).min(100);
    let popup_height = (area.height * 60 / 100).min(30);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect {
        x: area.x + popup_x,
        y: area.y + popup_y,
        width: popup_width,
        height: popup_height,
    };

    // Clear the area and render background
    f.render_widget(Clear, popup_area);
    f.render_widget(
        Block::default().style(Style::default().bg(theme.bg_panel)),
        popup_area,
    );

    // Render border and title (using pre-computed total)
    let title = format!(" Command Palette ({} commands) ", vm.total_commands);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(theme.bg_panel));

    f.render_widget(block, popup_area);

    // Calculate inner area
    let inner = popup_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    // Split into input area, results area, details area, and footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Input box
            Constraint::Min(5),    // Results list
            Constraint::Length(2), // Details area (for selected command)
            Constraint::Length(1), // Footer
        ])
        .split(inner);

    // Render input box (pre-formatted in view model)
    let input_paragraph = Paragraph::new(vm.input_text.clone())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.accent_primary))
                .style(Style::default().bg(theme.bg_secondary)),
        )
        .style(
            Style::default()
                .fg(theme.text_primary)
                .bg(theme.bg_secondary),
        );
    f.render_widget(input_paragraph, chunks[0]);

    // Render results list
    if vm.visible_rows.is_empty() {
        // No results
        let no_results = Paragraph::new("No matching commands")
            .style(Style::default().fg(theme.text_muted))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(no_results, chunks[1]);
    } else {
        // Build result lines - simple iteration over pre-computed view models!
        let result_lines: Vec<Line> = vm
            .visible_rows
            .iter()
            .map(|row_vm| {
                let mut spans = Vec::new();

                // All text is pre-formatted in view model!
                spans.push(Span::styled(
                    row_vm.indicator.clone(),
                    Style::default()
                        .fg(if row_vm.is_selected {
                            theme.accent_primary
                        } else {
                            theme.text_primary
                        })
                        .add_modifier(if row_vm.is_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ));

                spans.push(Span::styled(
                    row_vm.shortcut_hint.clone(),
                    Style::default().fg(if row_vm.is_selected {
                        theme.accent_primary
                    } else {
                        theme.text_muted
                    }),
                ));

                spans.push(Span::styled(
                    row_vm.title.clone(),
                    Style::default()
                        .fg(row_vm.fg_color)
                        .bg(row_vm.bg_color)
                        .add_modifier(if row_vm.is_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ));

                spans.push(Span::raw(row_vm.padding.clone()));

                spans.push(Span::styled(
                    row_vm.category.clone(),
                    Style::default().fg(if row_vm.is_selected {
                        theme.text_secondary
                    } else {
                        theme.text_muted
                    }),
                ));

                Line::from(spans)
            })
            .collect();

        let results_paragraph = Paragraph::new(result_lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme.bg_panel));
        f.render_widget(results_paragraph, chunks[1]);
    }

    // Render details area with selected command info (pre-computed in view model)
    if let Some(ref selected_cmd) = vm.selected_command {
        let mut details_text = vec![];

        details_text.push(Span::styled(
            selected_cmd.description.clone(),
            Style::default().fg(theme.text_secondary),
        ));

        if let Some(ref context) = selected_cmd.context {
            details_text.push(Span::styled(
                format!("  ({})", context),
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ));
        }

        let details_line = Line::from(details_text);
        let details_paragraph = Paragraph::new(details_line)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme.bg_panel));
        f.render_widget(details_paragraph, chunks[2]);
    }

    // Render footer with keyboard hints
    let footer_line = Line::from(vec![
        Span::styled(
            "Enter",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" execute  ", Style::default().fg(theme.text_muted)),
        Span::styled(
            "j/k",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" navigate  ", Style::default().fg(theme.text_muted)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" close", Style::default().fg(theme.text_muted)),
    ]);

    let footer = Paragraph::new(footer_line)
        .style(Style::default().fg(theme.text_secondary))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(footer, chunks[3]);
}
