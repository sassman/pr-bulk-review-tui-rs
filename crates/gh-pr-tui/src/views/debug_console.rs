use ratatui::{prelude::*, widgets::*};

use crate::App;

/// Render the debug console as a Quake-style drop-down panel
/// Returns the visible viewport height for page down scrolling
pub fn render_debug_console(f: &mut Frame, area: Rect, app: &App) -> usize {
    use ratatui::widgets::{Clear, List, ListItem};

    let console_state = &app.store.state().debug_console;
    let theme = &app.store.state().theme;

    // Calculate console height based on percentage
    let console_height = (area.height * console_state.height_percent) / 100;
    let console_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: console_height.min(area.height),
    };

    // Clear the area
    f.render_widget(Clear, console_area);

    // Read logs from buffer
    let logs = console_state.logs.lock().unwrap();
    let log_count = logs.len();

    // Calculate visible range based on scroll offset
    let visible_height = console_height.saturating_sub(3) as usize; // Subtract border and header
    let total_logs = logs.len();

    let scroll_offset = if console_state.auto_scroll {
        // Auto-scroll: show most recent logs
        total_logs.saturating_sub(visible_height)
    } else {
        // Manual scroll: use scroll_offset
        console_state
            .scroll_offset
            .min(total_logs.saturating_sub(visible_height))
    };

    // Convert logs to ListItems with color coding
    let log_items: Vec<ListItem> = logs
        .iter()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|entry| {
            use ::log::Level;

            let level_color = match entry.level {
                Level::Error => theme.status_error,
                Level::Warn => theme.status_warning,
                Level::Info => theme.text_primary,
                Level::Debug => theme.text_secondary,
                Level::Trace => theme.text_muted,
            };

            let timestamp = entry.timestamp.format("%H:%M:%S%.3f");
            let level_str = format!("{:5}", entry.level.to_string().to_uppercase());
            let target_short = if entry.target.len() > 20 {
                format!("{}...", &entry.target[..17])
            } else {
                format!("{:20}", entry.target)
            };

            let text = format!(
                "{} {} {} {}",
                timestamp, level_str, target_short, entry.message
            );

            ListItem::new(text).style(Style::default().fg(level_color))
        })
        .collect();

    // Create the list widget
    let logs_list = List::new(log_items).block(
        Block::bordered()
            .title(format!(
                " Debug Console ({}/{}) {} ",
                scroll_offset + visible_height.min(total_logs),
                log_count,
                if console_state.auto_scroll {
                    "[AUTO]"
                } else {
                    "[MANUAL]"
                }
            ))
            .title_bottom(" `~` Close | j/k Scroll | a Auto-scroll | c Clear ")
            .border_style(Style::default().fg(theme.accent_primary))
            .style(Style::default().bg(theme.bg_secondary)),
    );

    f.render_widget(logs_list, console_area);

    visible_height
}
