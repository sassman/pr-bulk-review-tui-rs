use ratatui::{
    prelude::*,
    style::palette::tailwind,
};

/// Application theme - centralized color and style management
#[derive(Debug, Clone)]
pub struct Theme {
    // Background colors
    pub bg_primary: Color,
    pub bg_secondary: Color,
    pub bg_tertiary: Color,
    pub bg_panel: Color,

    // Text colors
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub text_header: Color,

    // Accent colors
    pub accent_primary: Color,
    pub accent_secondary: Color,

    // Status colors
    pub status_success: Color,
    pub status_error: Color,
    pub status_warning: Color,
    pub status_info: Color,
    pub status_checking: Color,

    // Action colors (for keybindings)
    pub action_navigate: Color,
    pub action_select: Color,
    pub action_open: Color,
    pub action_refresh: Color,
    pub action_filter: Color,
    pub action_merge: Color,
    pub action_rebase: Color,
    pub action_danger: Color,
    pub action_help: Color,

    // Selection colors
    pub selected_bg: Color,
    pub selected_fg: Color,

    // Table colors
    pub table_header_bg: Color,
    pub table_header_fg: Color,
    pub table_row_fg: Color,
    pub table_row_bg_normal: Color,
    pub table_row_bg_alt: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Dark theme (default) - Cyan/Magenta color scheme
    pub fn dark() -> Self {
        Self {
            // Backgrounds
            bg_primary: tailwind::SLATE.c950,
            bg_secondary: tailwind::SLATE.c900,
            bg_tertiary: tailwind::SLATE.c800,
            bg_panel: tailwind::SLATE.c800,

            // Text - slightly cyan-tinted
            text_primary: tailwind::CYAN.c50,
            text_secondary: tailwind::CYAN.c200,
            text_muted: tailwind::CYAN.c700,
            text_header: tailwind::CYAN.c100,

            // Accents - bright cyan and magenta
            accent_primary: tailwind::CYAN.c400,
            accent_secondary: tailwind::FUCHSIA.c500,

            // Status - cyan/magenta themed
            status_success: tailwind::CYAN.c400,
            status_error: tailwind::FUCHSIA.c400,
            status_warning: tailwind::PURPLE.c400,
            status_info: tailwind::CYAN.c500,
            status_checking: tailwind::PURPLE.c400,

            // Actions - cyan/magenta/purple palette
            action_navigate: tailwind::CYAN.c500,
            action_select: tailwind::CYAN.c600,
            action_open: tailwind::FUCHSIA.c500,
            action_refresh: tailwind::PURPLE.c500,
            action_filter: tailwind::VIOLET.c500,
            action_merge: tailwind::FUCHSIA.c600,
            action_rebase: tailwind::PURPLE.c600,
            action_danger: tailwind::FUCHSIA.c600,
            action_help: tailwind::CYAN.c700,

            // Selection - magenta highlight
            selected_bg: tailwind::FUCHSIA.c500,
            selected_fg: Color::White,

            // Table - cyan header with magenta accents
            table_header_bg: tailwind::CYAN.c700,
            table_header_fg: tailwind::SLATE.c50,
            table_row_fg: tailwind::CYAN.c100,
            table_row_bg_normal: tailwind::SLATE.c950,
            table_row_bg_alt: tailwind::SLATE.c900,
        }
    }

    // Prebuilt styles for common use cases

    /// Style for panel backgrounds (shortcuts, logs, etc.)
    pub fn panel_background(&self) -> Style {
        Style::default().bg(self.bg_panel)
    }

    /// Style for panel borders
    pub fn panel_border(&self) -> Style {
        Style::default()
            .fg(self.accent_primary)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for panel titles
    pub fn panel_title(&self) -> Style {
        Style::default()
            .fg(self.accent_primary)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for section headers
    pub fn section_header(&self) -> Style {
        Style::default()
            .fg(self.status_warning)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    }

    /// Style for key hints (e.g., "Space" in "Press Space to...")
    pub fn key_hint(&self) -> Style {
        Style::default()
            .fg(self.accent_primary)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for key descriptions
    pub fn key_description(&self) -> Style {
        Style::default().fg(self.text_secondary)
    }

    /// Style for action buttons/badges
    pub fn action_badge(&self, bg_color: Color) -> Style {
        Style::default()
            .fg(Color::White)
            .bg(bg_color)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for table headers
    pub fn table_header(&self) -> Style {
        Style::default()
            .fg(self.table_header_fg)
            .bg(self.table_header_bg)
    }

    /// Style for selected table rows
    pub fn table_selected(&self) -> Style {
        Style::default()
            .fg(self.selected_fg)
            .bg(self.selected_bg)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for normal table rows
    pub fn table_row(&self) -> Style {
        Style::default().fg(self.table_row_fg)
    }

    /// Style for error messages
    pub fn error(&self) -> Style {
        Style::default()
            .fg(self.status_error)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for success messages
    pub fn success(&self) -> Style {
        Style::default()
            .fg(self.status_success)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for warning messages
    pub fn warning(&self) -> Style {
        Style::default()
            .fg(self.status_warning)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for muted/helper text
    pub fn muted(&self) -> Style {
        Style::default().fg(self.text_muted)
    }

    /// Style for primary text
    pub fn text(&self) -> Style {
        Style::default().fg(self.text_primary)
    }

    /// Style for log line numbers/timestamps
    pub fn log_metadata(&self) -> Style {
        Style::default()
            .fg(self.text_muted)
            .bg(self.bg_panel)
    }

    /// Style for error sections in logs
    pub fn log_error(&self) -> Style {
        Style::default().fg(self.status_error)
    }

    /// Style for warning sections in logs
    pub fn log_warning(&self) -> Style {
        Style::default().fg(self.status_warning)
    }
}
