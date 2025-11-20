use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::*,
    widgets::*,
};

use crate::shortcuts::get_shortcuts;

/// Render the shortcuts help panel as a centered floating window
/// Returns the maximum scroll offset
pub fn render_shortcuts_panel(
    f: &mut Frame,
    area: Rect,
    scroll_offset: usize,
    theme: &crate::theme::Theme,
) -> usize {
    // Calculate centered area (80% width, 90% height)
    let popup_width = (area.width * 80 / 100).min(100);
    let popup_height = (area.height * 90 / 100).min(40);
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

    // Calculate inner area and split into content and sticky footer
    let inner = popup_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    // Split inner area: content area and 1-line footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Scrollable content
            Constraint::Length(1), // Sticky footer
        ])
        .split(inner);

    let content_area = chunks[0];
    let footer_area = chunks[1];

    // Build text content (without footer - it will be rendered separately)
    let mut text_lines = Vec::new();

    for category in get_shortcuts() {
        // Category header
        text_lines.push(Line::from(vec![Span::styled(
            category.name,
            Style::default()
                .fg(theme.status_warning)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )]));
        text_lines.push(Line::from(""));

        // Items in this category
        for shortcut in category.shortcuts {
            text_lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:18}", shortcut.key_display),
                    Style::default()
                        .fg(theme.status_success)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    shortcut.description,
                    Style::default().fg(theme.text_secondary),
                ),
            ]));
        }

        text_lines.push(Line::from(""));
    }

    // Calculate visible area and apply scrolling
    let total_lines = text_lines.len();
    let visible_height = content_area.height as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let actual_scroll = scroll_offset.min(max_scroll);

    // Add scroll indicator to title if scrollable
    let title = if total_lines > visible_height {
        format!(
            " Keyboard Shortcuts  [{}/{}] ",
            actual_scroll + 1,
            total_lines
        )
    } else {
        " Keyboard Shortcuts ".to_string()
    };

    // Render block with updated title
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

    // Render scrollable content
    let paragraph = Paragraph::new(text_lines)
        .wrap(Wrap { trim: false })
        .scroll((actual_scroll as u16, 0))
        .style(Style::default().bg(theme.bg_panel));

    f.render_widget(paragraph, content_area);

    // Render sticky footer at the bottom
    let footer_line = Line::from(vec![
        Span::styled("Press ", Style::default().fg(theme.text_muted)),
        Span::styled(
            "x",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(theme.text_muted)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to close this help", Style::default().fg(theme.text_muted)),
    ]);

    let footer = Paragraph::new(footer_line)
        .style(Style::default().bg(theme.bg_panel))
        .alignment(ratatui::layout::Alignment::Center);

    f.render_widget(footer, footer_area);

    // Return the max scroll value so it can be stored in app state
    max_scroll
}
