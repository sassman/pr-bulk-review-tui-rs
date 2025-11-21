use ratatui::style::Color;

/// View model for command palette - all presentation data pre-computed
#[derive(Debug, Clone)]
pub struct CommandPaletteViewModel {
    /// Pre-formatted input text with prompt
    pub input_text: String,
    /// Total number of filtered commands
    pub total_commands: usize,
    /// Pre-computed visible rows with all formatting applied
    pub visible_rows: Vec<CommandRow>,
    /// Selected command details (if any)
    pub selected_command: Option<SelectedCommand>,
    /// Pre-calculated scroll offset
    pub scroll_offset: usize,
}

/// A single row in the command palette list
#[derive(Debug, Clone)]
pub struct CommandRow {
    /// Whether this row is the selected one
    pub is_selected: bool,
    /// Selection indicator: "> " or "  "
    pub indicator: String,
    /// Pre-formatted shortcut hint (13 chars: 12 for hint + 1 space)
    pub shortcut_hint: String,
    /// Title text (pre-truncated if needed)
    pub title: String,
    /// Category text with brackets: "[Category]"
    pub category: String,
    /// Pre-computed padding spaces for alignment
    pub padding: String,
    /// Foreground color
    pub fg_color: Color,
    /// Background color
    pub bg_color: Color,
}

/// Details about the selected command
#[derive(Debug, Clone)]
pub struct SelectedCommand {
    /// Command description
    pub description: String,
    /// Optional context information
    pub context: Option<String>,
}

impl CommandPaletteViewModel {
    /// Build view model from command palette state
    pub fn from_state(
        input: &str,
        selected_index: usize,
        filtered_commands: &[(gh_pr_tui_command_palette::CommandItem<crate::actions::Action>, u16)],
        visible_height: usize,
        available_width: usize,
        theme: &crate::theme::Theme,
    ) -> Self {
        let total_commands = filtered_commands.len();

        // Calculate scroll offset to keep selected item visible
        let scroll_offset = if total_commands == 0 {
            0
        } else if selected_index < visible_height / 2 {
            0
        } else if selected_index >= total_commands.saturating_sub(visible_height / 2) {
            total_commands.saturating_sub(visible_height)
        } else {
            selected_index.saturating_sub(visible_height / 2)
        };

        // Build visible rows
        let visible_rows = filtered_commands
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_height)
            .map(|(i, (cmd, _score))| {
                let is_selected = i == selected_index;

                // Selection indicator
                let indicator = if is_selected {
                    "> ".to_string()
                } else {
                    "  ".to_string()
                };

                // Shortcut hint (13 chars: 12 for hint + 1 space)
                let shortcut_hint = if let Some(ref hint) = cmd.shortcut_hint {
                    format!("{:12} ", hint)
                } else {
                    "             ".to_string()
                };

                // Calculate title truncation
                // Total width - indicator(2) - shortcut(13) - category(len+2 for brackets) - padding(3)
                let category_text = format!("[{}]", cmd.category);
                let fixed_width = 2 + 13 + category_text.len() + 3;
                let max_title_width = available_width.saturating_sub(fixed_width);

                // Truncate title if needed
                let title = if cmd.title.len() > max_title_width && max_title_width > 3 {
                    format!("{}...", &cmd.title[..max_title_width.saturating_sub(3)])
                } else {
                    cmd.title.clone()
                };

                // Calculate padding for right-aligned category
                let used_width = 2 + 13 + title.len() + category_text.len();
                let padding = if available_width > used_width {
                    " ".repeat(available_width.saturating_sub(used_width))
                } else {
                    " ".to_string()
                };

                // Colors
                let (fg_color, bg_color) = if is_selected {
                    (theme.selected_fg, theme.selected_bg)
                } else {
                    (theme.text_primary, Color::Reset)
                };

                CommandRow {
                    is_selected,
                    indicator,
                    shortcut_hint,
                    title,
                    category: category_text,
                    padding,
                    fg_color,
                    bg_color,
                }
            })
            .collect();

        // Extract selected command details
        let selected_command = filtered_commands.get(selected_index).map(|(cmd, _)| {
            SelectedCommand {
                description: cmd.description.clone(),
                context: cmd.context.clone(),
            }
        });

        Self {
            input_text: format!("> {}", input),
            total_commands,
            visible_rows,
            selected_command,
            scroll_offset,
        }
    }
}
