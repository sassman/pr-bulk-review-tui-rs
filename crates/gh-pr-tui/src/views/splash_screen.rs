use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    prelude::*,
    style::palette::tailwind,
    widgets::*,
};

use crate::App;
use crate::state::BootstrapState;

/// Render the fancy splash screen shown during application bootstrap
pub fn render_splash_screen(f: &mut Frame, app: &App) {
    const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    let area = f.area();

    // Calculate a centered area for the splash screen content
    let centered_area = {
        let width = 50.min(area.width);
        let height = 12.min(area.height);
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;
        Rect {
            x,
            y,
            width,
            height,
        }
    };

    // Clear background
    f.render_widget(
        Block::default().style(Style::default().bg(tailwind::SLATE.c950)),
        area,
    );

    // Render the centered content box
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(tailwind::BLUE.c500))
        .style(Style::default().bg(tailwind::SLATE.c900));

    f.render_widget(block, centered_area);

    // Split the centered area into sections for content
    let inner = centered_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title
            Constraint::Length(1), // Title underline
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // Spinner
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // Progress bar
            Constraint::Length(1), // Spacing
            Constraint::Min(2),    // Status message
        ])
        .split(inner);

    // Determine stage info
    let (stage_message, progress, is_error) = match &app.store.state().infrastructure.bootstrap_state {
        BootstrapState::NotStarted => ("Initializing application...", 0, false),
        BootstrapState::LoadingRepositories => ("Loading repositories...", 25, false),
        BootstrapState::RestoringSession => ("Restoring session...", 50, false),
        BootstrapState::LoadingFirstRepo => {
            // Loading the selected repo first
            if let Some(repo) = app
                .store
                .state()
                .repos
                .recent_repos
                .get(app.store.state().repos.selected_repo)
            {
                (&format!("Loading {}...", repo.repo)[..], 75, false)
            } else {
                ("Loading repository...", 75, false)
            }
        }
        BootstrapState::UIReady
        | BootstrapState::LoadingRemainingRepos
        | BootstrapState::Completed => {
            // This state shouldn't be shown in splash screen as UI should be visible
            unreachable!()
        }
        BootstrapState::Error(err) => (&format!("Error: {}", err)[..], 0, true),
    };

    // Title
    let title = Paragraph::new("PR Bulk Review TUI")
        .style(
            Style::default()
                .fg(tailwind::BLUE.c400)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(title, chunks[0]);

    // Title underline
    let underline = Paragraph::new("─────────────────")
        .style(Style::default().fg(tailwind::BLUE.c600))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(underline, chunks[1]);

    // Spinner (animated)
    if !is_error {
        let spinner = SPINNER_FRAMES[app.store.state().ui.spinner_frame % SPINNER_FRAMES.len()];
        let spinner_text = format!("{} Loading...", spinner);
        let spinner_widget = Paragraph::new(spinner_text)
            .style(
                Style::default()
                    .fg(app.store.state().theme.status_warning)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(spinner_widget, chunks[3]);
    } else {
        let error_icon = Paragraph::new("✗ Error")
            .style(
                Style::default()
                    .fg(app.store.state().theme.status_error)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(error_icon, chunks[3]);
    }

    // Progress bar
    if !is_error {
        let bar_width = chunks[5].width.saturating_sub(10) as usize; // Reserve space for percentage
        let filled = (bar_width * progress) / 100;
        let empty = bar_width.saturating_sub(filled);

        let progress_bar = format!("{}{}  {}%", "▰".repeat(filled), "▱".repeat(empty), progress);

        let bar_widget = Paragraph::new(progress_bar)
            .style(Style::default().fg(app.store.state().theme.status_info))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(bar_widget, chunks[5]);
    }

    // Status message
    let message_style = if is_error {
        Style::default().fg(app.store.state().theme.status_error)
    } else {
        Style::default().fg(app.store.state().theme.text_secondary)
    };

    let message_widget = Paragraph::new(stage_message)
        .style(message_style)
        .alignment(ratatui::layout::Alignment::Center)
        .wrap(Wrap { trim: true });
    f.render_widget(message_widget, chunks[7]);
}
