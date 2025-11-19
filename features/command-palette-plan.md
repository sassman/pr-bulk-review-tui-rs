# Command Palette Implementation Plan

## Executive Summary

This document proposes 3 architectural approaches for implementing a command palette in the gh-pr-tui application. All solutions provide fuzzy search over actions with context-aware filtering, but differ in extensibility and complexity.

**Recommendation**: Start with **Solution 2 (Trait-Based Provider System)** - it balances simplicity with extensibility, making it easy to add tasks/background commands later.

---

## Current Architecture Analysis

### Existing Components

**Actions** (`actions.rs`):
- 119 action variants in `Action` enum
- Pure domain logic - represents "what can happen"
- Already separated from UI concerns

**Shortcuts** (`shortcuts.rs`):
- `Shortcut` struct with `key_display`, `description`, `action`
- Context filtering via matcher functions
- Organized into categories
- Already has rendering logic (shortcuts panel)

**Redux Pattern**:
- `Action` → `reduce()` → `(State, Vec<Effect>)`
- Clean separation of state and side effects
- Perfect foundation for command execution

### Key Insights

1. **Shortcuts ARE the metadata layer** - they already have searchable text (`key_display`, `description`)
2. **Context handling exists** - `ShortcutMatcher` functions check key events
3. **Rendering patterns established** - shortcuts panel shows how to do centered popups
4. **Fuzzy search needed** - nucleo is the best choice (6x faster than alternatives)

---

## Solution 1: Direct Shortcut Indexing (Simplest)

### Overview

Directly reuse existing `Shortcut` structs as command palette items. Add fuzzy search and a new UI state for the palette.

### Architecture

```
┌─────────────────────────────────────┐
│  Command Palette State              │
│  - input: String                    │
│  - filtered_shortcuts: Vec<Shortcut>│
│  - selected_index: usize            │
└─────────────────────────────────────┘
           │
           ├─> Fuzzy Search (nucleo)
           │   Input: shortcuts.rs::get_all_shortcuts_flat()
           │   Output: Sorted by score
           │
           └─> Context Filter
               Input: AppState
               Output: Shortcuts available in current context
```

### Implementation Plan

#### Step 1: Add Dependencies (`Cargo.toml`)

```toml
[dependencies]
nucleo-matcher = "0.3"
```

#### Step 2: New State (`state.rs`)

```rust
/// Command palette state
#[derive(Debug, Clone)]
pub struct CommandPaletteState {
    pub input: String,
    pub selected_index: usize,
    pub filtered_shortcuts: Vec<(crate::shortcuts::Shortcut, i64)>, // (shortcut, score)
}

impl CommandPaletteState {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            selected_index: 0,
            filtered_shortcuts: Vec::new(),
        }
    }
}

// Add to AppState.ui
pub struct UiState {
    // ... existing fields
    pub command_palette: Option<CommandPaletteState>,
}
```

#### Step 3: New Actions (`actions.rs`)

```rust
pub enum Action {
    // ... existing variants

    // Command palette
    ShowCommandPalette,
    HideCommandPalette,
    CommandPaletteInput(char),
    CommandPaletteBackspace,
    CommandPaletteSelectNext,
    CommandPaletteSelectPrev,
    CommandPaletteExecute, // Execute selected command
}
```

#### Step 4: Command Palette Module (`command_palette.rs`)

```rust
use nucleo_matcher::{Matcher, Config, pattern::Pattern, pattern::CaseMatching};
use crate::shortcuts::{Shortcut, get_all_shortcuts_flat};
use crate::state::AppState;

/// Filter and score shortcuts based on query and context
pub fn filter_shortcuts(
    query: &str,
    state: &AppState,
) -> Vec<(Shortcut, i64)> {
    let all_shortcuts = get_all_shortcuts_flat();

    // Early return for empty query - show all context-appropriate shortcuts
    if query.is_empty() {
        return all_shortcuts
            .into_iter()
            .filter(|s| is_shortcut_available(s, state))
            .map(|s| (s, 0))
            .collect();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Smart);

    all_shortcuts
        .into_iter()
        .filter(|s| is_shortcut_available(s, state))
        .filter_map(|shortcut| {
            // Search in both key_display and description
            let haystack = format!("{} {}",
                shortcut.key_display,
                shortcut.description
            );

            matcher.fuzzy_match(&haystack, &pattern)
                .map(|score| (shortcut, score))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .sorted_by_key(|(_, score)| -score)
        .collect()
}

/// Check if a shortcut is available in current context
fn is_shortcut_available(shortcut: &Shortcut, state: &AppState) -> bool {
    // Context-based filtering
    let has_prs = !state.repos.prs.is_empty();
    let has_selection = state.repos.repo_data
        .get(&state.repos.selected_repo)
        .map(|d| !d.selected_pr_numbers.is_empty())
        .unwrap_or(false);
    let log_panel_open = state.log_panel.panel.is_some();

    // Check if action makes sense in current context
    use crate::actions::Action;
    match &shortcut.action {
        Action::MergeSelectedPrs | Action::ApprovePrs | Action::Rebase
            => has_selection,
        Action::OpenBuildLogs | Action::OpenCurrentPrInBrowser | Action::OpenInIDE
            => has_prs,
        Action::CloseLogPanel | Action::SelectNextJob | Action::ToggleTimestamps
            => log_panel_open,
        // Most actions are always available
        _ => true,
    }
}
```

#### Step 5: Reducer (`reducer.rs`)

```rust
fn ui_reducer(mut state: UiState, action: &Action) -> (UiState, Vec<Effect>) {
    let mut effects = Vec::new();

    match action {
        Action::ShowCommandPalette => {
            state.command_palette = Some(CommandPaletteState::new());
        }
        Action::HideCommandPalette => {
            state.command_palette = None;
        }
        Action::CommandPaletteInput(c) => {
            if let Some(palette) = &mut state.command_palette {
                palette.input.push(*c);
                palette.selected_index = 0; // Reset selection

                // Trigger filter update
                effects.push(Effect::UpdateCommandPaletteFilter);
            }
        }
        Action::CommandPaletteBackspace => {
            if let Some(palette) = &mut state.command_palette {
                palette.input.pop();
                palette.selected_index = 0;
                effects.push(Effect::UpdateCommandPaletteFilter);
            }
        }
        Action::CommandPaletteSelectNext => {
            if let Some(palette) = &mut state.command_palette {
                if !palette.filtered_shortcuts.is_empty() {
                    palette.selected_index =
                        (palette.selected_index + 1) % palette.filtered_shortcuts.len();
                }
            }
        }
        Action::CommandPaletteSelectPrev => {
            if let Some(palette) = &mut state.command_palette {
                if !palette.filtered_shortcuts.is_empty() {
                    palette.selected_index = palette.selected_index
                        .checked_sub(1)
                        .unwrap_or(palette.filtered_shortcuts.len() - 1);
                }
            }
        }
        Action::CommandPaletteExecute => {
            if let Some(palette) = &state.command_palette {
                if let Some((shortcut, _)) = palette.filtered_shortcuts.get(palette.selected_index) {
                    // Execute the selected action
                    effects.push(Effect::DispatchAction(shortcut.action.clone()));
                    // Close palette
                    state.command_palette = None;
                }
            }
        }
        // ... existing cases
    }

    (state, effects)
}
```

#### Step 6: Effect (`effect.rs`)

```rust
pub enum Effect {
    // ... existing variants
    UpdateCommandPaletteFilter,
}

// In execute_effect:
Effect::UpdateCommandPaletteFilter => {
    if let Some(palette_state) = &app.store.state().ui.command_palette {
        let filtered = filter_shortcuts(
            &palette_state.input,
            app.store.state()
        );

        // Update state with filtered results
        follow_up_actions.push(Action::UpdateCommandPaletteResults(filtered));
    }
}
```

#### Step 7: Rendering (`main.rs`)

```rust
fn render_command_palette(f: &mut Frame, app: &App, area: Rect) {
    if let Some(palette) = &app.store.state().ui.command_palette {
        // Calculate centered area (similar to shortcuts panel)
        let popup_width = (area.width * 60 / 100).min(80);
        let popup_height = 15.min(area.height - 4);
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = 2; // Near top

        let popup_area = Rect { x: popup_x, y: popup_y, width: popup_width, height: popup_height };

        // Clear and render background
        f.render_widget(Clear, popup_area);

        // Split into input area and results area
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),   // Input box
                Constraint::Min(0),      // Results list
            ])
            .split(popup_area.inner(&Margin { horizontal: 1, vertical: 1 }));

        // Render input box
        let input_widget = Paragraph::new(format!("> {}", palette.input))
            .block(Block::default()
                .borders(Borders::ALL)
                .title(" Command Palette ")
                .border_style(Style::default().fg(app.store.state().theme.accent_primary)))
            .style(Style::default().fg(app.store.state().theme.text_primary));
        f.render_widget(input_widget, chunks[0]);

        // Render results list
        let results: Vec<ListItem> = palette.filtered_shortcuts
            .iter()
            .enumerate()
            .map(|(i, (shortcut, score))| {
                let style = if i == palette.selected_index {
                    Style::default()
                        .bg(app.store.state().theme.selected_bg)
                        .fg(app.store.state().theme.selected_fg)
                } else {
                    Style::default().fg(app.store.state().theme.text_secondary)
                };

                let text = format!(
                    "{:15} {}",
                    shortcut.key_display,
                    shortcut.description
                );
                ListItem::new(text).style(style)
            })
            .collect();

        let results_widget = List::new(results)
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(results_widget, chunks[1]);
    }
}
```

#### Step 8: Key Handling (`main.rs`)

```rust
fn handle_key_event(...) -> Action {
    // Handle command palette keys if palette is open
    if app.store.state().ui.command_palette.is_some() {
        match key.code {
            KeyCode::Esc => return Action::HideCommandPalette,
            KeyCode::Enter => return Action::CommandPaletteExecute,
            KeyCode::Down | KeyCode::Char('j') if !key.modifiers.contains(KeyModifiers::CONTROL)
                => return Action::CommandPaletteSelectNext,
            KeyCode::Up | KeyCode::Char('k') if !key.modifiers.contains(KeyModifiers::CONTROL)
                => return Action::CommandPaletteSelectPrev,
            KeyCode::Backspace => return Action::CommandPaletteBackspace,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
                => return Action::CommandPaletteInput(c),
            _ => return Action::None,
        }
    }

    // ... rest of key handling
}
```

#### Step 9: Shortcut to Trigger (`shortcuts.rs`)

```rust
ShortcutCategory {
    name: "General",
    shortcuts: vec![
        Shortcut {
            key_display: "Ctrl+P",
            description: "Open command palette",
            action: Action::ShowCommandPalette,
            matcher: ShortcutMatcher::SingleKey(|key| {
                matches!(key.code, KeyCode::Char('p'))
                    && key.modifiers.contains(KeyModifiers::CONTROL)
            }),
        },
        // ... existing shortcuts
    ]
}
```

### Files to Create/Modify

**New Files**:
- `crates/gh-pr-tui/src/command_palette.rs` - Fuzzy search logic

**Modified Files**:
- `crates/gh-pr-tui/Cargo.toml` - Add nucleo-matcher
- `crates/gh-pr-tui/src/main.rs` - Add module, rendering, key handling
- `crates/gh-pr-tui/src/state.rs` - Add CommandPaletteState
- `crates/gh-pr-tui/src/actions.rs` - Add command palette actions
- `crates/gh-pr-tui/src/reducer.rs` - Add command palette reducer
- `crates/gh-pr-tui/src/effect.rs` - Add filter update effect
- `crates/gh-pr-tui/src/shortcuts.rs` - Add Ctrl+P shortcut

### Pros

✅ **Simplest to implement** - reuses existing shortcuts directly
✅ **Zero duplication** - shortcuts already have all needed metadata
✅ **Context filtering built-in** - leverages existing matcher logic
✅ **Minimal code** - ~300 lines total

### Cons

❌ **Not extensible** - can't add non-shortcut commands (tasks, background operations)
❌ **Tight coupling** - command palette depends on shortcuts module
❌ **Limited filtering** - matcher functions designed for key events, not state

### Extensibility Assessment

**Score: 2/5** - Can only search shortcuts. Would need major refactoring to add:
- Background tasks (e.g., "Reload all repositories")
- One-off commands (e.g., "Clear all caches")
- Dynamic commands (e.g., "Switch to repo: {name}")

---

## Solution 2: Trait-Based Provider System (Recommended)

### Overview

Create a `CommandProvider` trait that different modules can implement. Shortcuts are one provider, but tasks/background operations can be added later without changing core logic.

### Architecture

```
┌─────────────────────────────────────┐
│  CommandPalette                     │
│  - providers: Vec<Box<dyn Provider>>│
│  - input: String                    │
│  - selected: usize                  │
└─────────────────────────────────────┘
           │
           ├─> ShortcutCommandProvider
           │   ├─> get_all_shortcuts_flat()
           │   └─> filter by context
           │
           ├─> TaskCommandProvider (future)
           │   ├─> "Reload all repos"
           │   └─> "Clear cache"
           │
           └─> Fuzzy Search (nucleo)
               Searches across all providers
```

### Implementation Plan

#### Step 1: Add Dependencies

```toml
[dependencies]
nucleo-matcher = "0.3"
```

#### Step 2: Command Provider Trait (`command_palette.rs`)

```rust
use crate::actions::Action;
use crate::state::AppState;

/// A command that can be executed from the command palette
#[derive(Debug, Clone)]
pub struct CommandItem {
    /// Display text for the command (e.g., "Merge PR")
    pub title: String,

    /// Longer description (e.g., "Merge selected pull requests")
    pub description: String,

    /// Category for grouping (e.g., "PR Actions", "Navigation")
    pub category: String,

    /// Keyboard shortcut hint (e.g., "m" or "Ctrl+P")
    pub shortcut_hint: Option<String>,

    /// The action to dispatch when this command is executed
    pub action: Action,

    /// Check if this command is available in the current state
    pub available: fn(&AppState) -> bool,
}

impl CommandItem {
    /// Get searchable text (title + description)
    pub fn searchable_text(&self) -> String {
        format!("{} {} {}", self.title, self.description, self.category)
    }
}

/// Trait for providing commands to the palette
pub trait CommandProvider: std::fmt::Debug {
    /// Get all commands from this provider
    fn commands(&self, state: &AppState) -> Vec<CommandItem>;

    /// Provider name for debugging
    fn name(&self) -> &str;
}

/// Registry of command providers
#[derive(Debug)]
pub struct CommandPaletteRegistry {
    providers: Vec<Box<dyn CommandProvider>>,
}

impl CommandPaletteRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn CommandProvider>) {
        self.providers.push(provider);
    }

    /// Get all commands from all providers
    pub fn all_commands(&self, state: &AppState) -> Vec<CommandItem> {
        self.providers
            .iter()
            .flat_map(|p| p.commands(state))
            .collect()
    }
}

impl Default for CommandPaletteRegistry {
    fn default() -> Self {
        let mut registry = Self::new();

        // Register built-in providers
        registry.register(Box::new(ShortcutCommandProvider));

        registry
    }
}
```

#### Step 3: Shortcut Command Provider (`command_palette.rs`)

```rust
use crate::shortcuts::{get_all_shortcuts_flat, Shortcut};

/// Provides commands from keyboard shortcuts
#[derive(Debug)]
struct ShortcutCommandProvider;

impl CommandProvider for ShortcutCommandProvider {
    fn commands(&self, state: &AppState) -> Vec<CommandItem> {
        get_all_shortcuts_flat()
            .into_iter()
            .filter_map(|shortcut| {
                // Skip shortcuts that are context-specific and not available
                if !is_shortcut_available(&shortcut, state) {
                    return None;
                }

                Some(CommandItem {
                    title: shortcut.description.to_string(),
                    description: format!("Keyboard shortcut: {}", shortcut.key_display),
                    category: extract_category(&shortcut),
                    shortcut_hint: Some(shortcut.key_display.to_string()),
                    action: shortcut.action.clone(),
                    available: Box::new(move |s| is_shortcut_available(&shortcut, s)),
                })
            })
            .collect()
    }

    fn name(&self) -> &str {
        "Shortcuts"
    }
}

/// Extract category from shortcut (based on action type)
fn extract_category(shortcut: &Shortcut) -> String {
    use crate::actions::Action;
    match &shortcut.action {
        Action::MergeSelectedPrs | Action::ApprovePrs | Action::Rebase | Action::RerunFailedJobs
            => "PR Actions".to_string(),
        Action::SelectNextRepo | Action::SelectPreviousRepo | Action::NavigateToNextPr
            => "Navigation".to_string(),
        Action::ToggleShortcuts | Action::Quit | Action::ShowAddRepoPopup
            => "General".to_string(),
        Action::OpenBuildLogs | Action::ToggleTimestamps | Action::NextError
            => "Log Viewer".to_string(),
        _ => "Other".to_string(),
    }
}

fn is_shortcut_available(shortcut: &Shortcut, state: &AppState) -> bool {
    let has_prs = !state.repos.prs.is_empty();
    let has_selection = state.repos.repo_data
        .get(&state.repos.selected_repo)
        .map(|d| !d.selected_pr_numbers.is_empty())
        .unwrap_or(false);
    let log_panel_open = state.log_panel.panel.is_some();

    use crate::actions::Action;
    match &shortcut.action {
        Action::MergeSelectedPrs | Action::ApprovePrs | Action::ShowClosePrPopup
            => has_selection,
        Action::Rebase => has_selection || has_prs, // Auto-rebase if no selection
        Action::OpenBuildLogs | Action::OpenCurrentPrInBrowser | Action::OpenInIDE
            => has_prs || log_panel_open,
        Action::CloseLogPanel | Action::SelectNextJob | Action::ToggleTimestamps
            => log_panel_open,
        // Most actions always available
        _ => true,
    }
}
```

#### Step 4: Fuzzy Search (`command_palette.rs`)

```rust
use nucleo_matcher::{Matcher, Config, pattern::Pattern, pattern::CaseMatching};

/// Filter and score commands based on query
pub fn filter_commands(
    commands: &[CommandItem],
    query: &str,
) -> Vec<(CommandItem, i64)> {
    // Empty query - return all commands
    if query.is_empty() {
        return commands.iter()
            .map(|c| (c.clone(), 0))
            .collect();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Smart);

    commands
        .iter()
        .filter_map(|cmd| {
            let haystack = cmd.searchable_text();
            matcher.fuzzy_match(&haystack, &pattern)
                .map(|score| (cmd.clone(), score))
        })
        .sorted_by_key(|(_, score)| -score)
        .collect()
}
```

#### Step 5: State (`state.rs`)

```rust
use crate::command_palette::{CommandItem, CommandPaletteRegistry};

#[derive(Debug, Clone)]
pub struct CommandPaletteState {
    pub input: String,
    pub selected_index: usize,
    pub filtered_commands: Vec<(CommandItem, i64)>, // (command, score)
}

// Add to UiState
pub struct UiState {
    // ... existing fields
    pub command_palette: Option<CommandPaletteState>,
    /// Command registry (not part of UI state, but lives here for convenience)
    /// This is initialized once and reused
    #[serde(skip)]
    pub command_registry: Arc<CommandPaletteRegistry>,
}
```

#### Step 6: Actions (`actions.rs`)

```rust
pub enum Action {
    // ... existing variants
    ShowCommandPalette,
    HideCommandPalette,
    CommandPaletteInput(char),
    CommandPaletteBackspace,
    CommandPaletteSelectNext,
    CommandPaletteSelectPrev,
    CommandPaletteExecute,
    UpdateCommandPaletteFilter, // Trigger re-filter
}
```

#### Step 7: Reducer (`reducer.rs`)

```rust
fn ui_reducer(mut state: UiState, action: &Action) -> (UiState, Vec<Effect>) {
    match action {
        Action::ShowCommandPalette => {
            // Get all available commands
            let app_state = /* need full AppState here - see note below */;
            let all_commands = state.command_registry.all_commands(&app_state);

            state.command_palette = Some(CommandPaletteState {
                input: String::new(),
                selected_index: 0,
                filtered_commands: filter_commands(&all_commands, ""),
            });
        }
        Action::CommandPaletteInput(c) => {
            if let Some(palette) = &mut state.command_palette {
                palette.input.push(*c);
                palette.selected_index = 0;
                effects.push(Effect::UpdateCommandPaletteFilter);
            }
        }
        Action::CommandPaletteExecute => {
            if let Some(palette) = &state.command_palette {
                if let Some((cmd, _)) = palette.filtered_commands.get(palette.selected_index) {
                    effects.push(Effect::DispatchAction(cmd.action.clone()));
                    state.command_palette = None;
                }
            }
        }
        // ... similar to Solution 1
    }
}
```

**Note**: Reducers need access to full `AppState` for context filtering. This requires slight refactoring of reducer signatures to pass full state.

#### Step 8: Future Extension - Task Command Provider (Example)

```rust
/// Provides one-off task commands
#[derive(Debug)]
struct TaskCommandProvider;

impl CommandProvider for TaskCommandProvider {
    fn commands(&self, state: &AppState) -> Vec<CommandItem> {
        vec![
            CommandItem {
                title: "Reload All Repositories".to_string(),
                description: "Refresh PR data from all configured repositories".to_string(),
                category: "Tasks".to_string(),
                shortcut_hint: None,
                action: Action::ReloadAllRepositories, // New action
                available: Box::new(|_| true),
            },
            CommandItem {
                title: "Clear Debug Logs".to_string(),
                description: "Clear all debug console logs".to_string(),
                category: "Tasks".to_string(),
                shortcut_hint: Some("c (in console)".to_string()),
                action: Action::ClearDebugLogs,
                available: Box::new(|_| true),
            },
        ]
    }

    fn name(&self) -> &str {
        "Tasks"
    }
}

// Register in default():
registry.register(Box::new(TaskCommandProvider));
```

### Files to Create/Modify

**New Files**:
- `crates/gh-pr-tui/src/command_palette.rs` - Provider trait, registry, fuzzy search

**Modified Files**:
- Same as Solution 1, plus:
  - `crates/gh-pr-tui/src/reducer.rs` - Pass full AppState to sub-reducers (refactor)

### Pros

✅ **Highly extensible** - Easy to add new command providers
✅ **Clean separation** - Commands decoupled from shortcuts
✅ **Future-proof** - Can add tasks, background operations, dynamic commands
✅ **Discoverable** - Users can find actions they didn't know existed
✅ **Testable** - Each provider can be unit tested independently

### Cons

⚠️ **More complex** - Trait objects, registry pattern
⚠️ **Requires refactoring** - Reducers need access to full AppState
⚠️ **Slight performance cost** - Dynamic dispatch for provider calls

### Extensibility Assessment

**Score: 5/5** - Can easily extend to:
- ✅ Background tasks ("Reload all repos", "Clear caches")
- ✅ Repository-specific commands ("Switch to repo: {name}")
- ✅ PR-specific commands ("Merge PR #123")
- ✅ Plugin system (external crates can provide commands)
- ✅ Macro recording ("Record macro", "Play macro")

---

## Solution 3: Hybrid Static Registry (Middle Ground)

### Overview

Create a static registry of `CommandMetadata` structs that wrap Actions with searchable metadata. Simpler than trait objects but more extensible than using shortcuts directly.

### Architecture

```
┌─────────────────────────────────────┐
│  COMMAND_REGISTRY                   │
│  static [CommandMetadata]           │
│  ├─> From shortcuts                 │
│  ├─> From tasks                     │
│  └─> From custom commands           │
└─────────────────────────────────────┘
           │
           └─> Fuzzy Search + Context Filter
               ├─> filter by `when` predicate
               └─> score by title/description
```

### Implementation Plan

#### Step 1: Command Metadata (`command_palette.rs`)

```rust
use crate::actions::Action;
use crate::state::AppState;

/// Metadata for a command that can be executed from the palette
#[derive(Debug, Clone)]
pub struct CommandMetadata {
    /// Unique identifier (for deduplication)
    pub id: &'static str,

    /// Display title (e.g., "Merge PR")
    pub title: &'static str,

    /// Description (e.g., "Merge selected pull requests")
    pub description: &'static str,

    /// Category (e.g., "PR Actions")
    pub category: CommandCategory,

    /// Keyboard shortcut hint
    pub shortcut_hint: Option<&'static str>,

    /// The action to execute
    pub action: Action,

    /// Context predicate - when is this command available?
    pub when: fn(&AppState) -> bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Navigation,
    PrActions,
    LogViewer,
    Tasks,
    General,
}

impl CommandCategory {
    pub fn label(&self) -> &'static str {
        match self {
            CommandCategory::Navigation => "Navigation",
            CommandCategory::PrActions => "PR Actions",
            CommandCategory::LogViewer => "Log Viewer",
            CommandCategory::Tasks => "Tasks",
            CommandCategory::General => "General",
        }
    }
}
```

#### Step 2: Static Command Registry (`command_palette.rs`)

```rust
use once_cell::sync::Lazy;

/// Global command registry - initialized once at startup
pub static COMMAND_REGISTRY: Lazy<Vec<CommandMetadata>> = Lazy::new(|| {
    vec![
        // Navigation
        CommandMetadata {
            id: "nav.next_pr",
            title: "Next PR",
            description: "Navigate to next pull request",
            category: CommandCategory::Navigation,
            shortcut_hint: Some("j / ↓"),
            action: Action::NavigateToNextPr,
            when: |s| !s.repos.prs.is_empty(),
        },
        CommandMetadata {
            id: "nav.prev_pr",
            title: "Previous PR",
            description: "Navigate to previous pull request",
            category: CommandCategory::Navigation,
            shortcut_hint: Some("k / ↑"),
            action: Action::NavigateToPreviousPr,
            when: |s| !s.repos.prs.is_empty(),
        },
        CommandMetadata {
            id: "nav.next_repo",
            title: "Next Repository",
            description: "Switch to next repository tab",
            category: CommandCategory::Navigation,
            shortcut_hint: Some("Tab / /"),
            action: Action::SelectNextRepo,
            when: |_| true,
        },

        // PR Actions
        CommandMetadata {
            id: "pr.merge",
            title: "Merge PR",
            description: "Merge selected pull requests",
            category: CommandCategory::PrActions,
            shortcut_hint: Some("m"),
            action: Action::MergeSelectedPrs,
            when: |s| {
                s.repos.repo_data
                    .get(&s.repos.selected_repo)
                    .map(|d| !d.selected_pr_numbers.is_empty())
                    .unwrap_or(false)
            },
        },
        CommandMetadata {
            id: "pr.approve",
            title: "Approve PR",
            description: "Approve selected pull requests",
            category: CommandCategory::PrActions,
            shortcut_hint: Some("a"),
            action: Action::ApprovePrs,
            when: |s| {
                s.repos.repo_data
                    .get(&s.repos.selected_repo)
                    .map(|d| !d.selected_pr_numbers.is_empty())
                    .unwrap_or(false)
            },
        },
        CommandMetadata {
            id: "pr.close",
            title: "Close PR",
            description: "Close selected pull requests",
            category: CommandCategory::PrActions,
            shortcut_hint: Some("c"),
            action: Action::ShowClosePrPopup,
            when: |s| {
                s.repos.repo_data
                    .get(&s.repos.selected_repo)
                    .map(|d| !d.selected_pr_numbers.is_empty())
                    .unwrap_or(false)
            },
        },
        CommandMetadata {
            id: "pr.rebase",
            title: "Rebase PR",
            description: "Rebase selected PRs (or auto-rebase if none selected)",
            category: CommandCategory::PrActions,
            shortcut_hint: Some("r"),
            action: Action::Rebase,
            when: |s| !s.repos.prs.is_empty(),
        },

        // Log Viewer
        CommandMetadata {
            id: "logs.open",
            title: "View Build Logs",
            description: "Open build logs for current PR",
            category: CommandCategory::LogViewer,
            shortcut_hint: Some("l"),
            action: Action::OpenBuildLogs,
            when: |s| !s.repos.prs.is_empty(),
        },
        CommandMetadata {
            id: "logs.next_error",
            title: "Next Error",
            description: "Jump to next error in build logs",
            category: CommandCategory::LogViewer,
            shortcut_hint: Some("n"),
            action: Action::NextError,
            when: |s| s.log_panel.panel.is_some(),
        },

        // Tasks
        CommandMetadata {
            id: "task.refresh_repo",
            title: "Refresh Repository",
            description: "Reload PRs for current repository",
            category: CommandCategory::Tasks,
            shortcut_hint: Some("Ctrl+r"),
            action: Action::RefreshCurrentRepo,
            when: |_| true,
        },

        // General
        CommandMetadata {
            id: "general.shortcuts",
            title: "Show Keyboard Shortcuts",
            description: "Display all keyboard shortcuts",
            category: CommandCategory::General,
            shortcut_hint: Some("?"),
            action: Action::ToggleShortcuts,
            when: |_| true,
        },
        CommandMetadata {
            id: "general.quit",
            title: "Quit",
            description: "Exit the application",
            category: CommandCategory::General,
            shortcut_hint: Some("q"),
            action: Action::Quit,
            when: |_| true,
        },
    ]
});

/// Get all commands available in current context
pub fn get_available_commands(state: &AppState) -> Vec<&'static CommandMetadata> {
    COMMAND_REGISTRY
        .iter()
        .filter(|cmd| (cmd.when)(state))
        .collect()
}
```

#### Step 3: Fuzzy Search (`command_palette.rs`)

```rust
use nucleo_matcher::{Matcher, Config, pattern::Pattern, pattern::CaseMatching};

pub fn filter_commands(
    state: &AppState,
    query: &str,
) -> Vec<(&'static CommandMetadata, i64)> {
    let available = get_available_commands(state);

    if query.is_empty() {
        return available.into_iter()
            .map(|cmd| (cmd, 0))
            .collect();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Smart);

    available
        .into_iter()
        .filter_map(|cmd| {
            let haystack = format!(
                "{} {} {}",
                cmd.title,
                cmd.description,
                cmd.category.label()
            );

            matcher.fuzzy_match(&haystack, &pattern)
                .map(|score| (cmd, score))
        })
        .sorted_by_key(|(_, score)| -score)
        .collect()
}
```

#### Step 4: State, Actions, Reducer, Rendering

Similar to Solution 1 and 2, but using `CommandMetadata` instead of `Shortcut` or `CommandItem`.

### Files to Create/Modify

**New Files**:
- `crates/gh-pr-tui/src/command_palette.rs` - Registry, metadata, fuzzy search

**Modified Files**:
- Same as Solution 1

**Additional Dependency**:
```toml
once_cell = "1.19" # For Lazy static initialization
```

### Pros

✅ **Good balance** - More extensible than Solution 1, simpler than Solution 2
✅ **Type-safe** - No trait objects, all static
✅ **Easy to add commands** - Just add to registry array
✅ **Good performance** - No dynamic dispatch
✅ **Centralized** - All commands in one place

### Cons

⚠️ **Requires duplication** - Commands defined separately from shortcuts
⚠️ **Static only** - Can't add dynamic commands at runtime
⚠️ **Manual sync** - Must keep shortcuts.rs and command_palette.rs in sync

### Extensibility Assessment

**Score: 3/5** - Can add:
- ✅ Static task commands
- ✅ One-off actions
- ⚠️ Dynamic commands (requires refactoring to Box<dyn Fn()>)
- ❌ Runtime plugins (needs trait objects)

---

## Comparison Matrix

| Feature | Solution 1<br/>(Direct Shortcuts) | Solution 2<br/>(Trait Providers) | Solution 3<br/>(Static Registry) |
|---------|----------------------------------|----------------------------------|----------------------------------|
| **Complexity** | ⭐ Simple | ⭐⭐⭐ Complex | ⭐⭐ Medium |
| **Extensibility** | ⭐⭐ Limited | ⭐⭐⭐⭐⭐ Excellent | ⭐⭐⭐ Good |
| **Performance** | ⭐⭐⭐⭐⭐ Excellent | ⭐⭐⭐⭐ Very Good | ⭐⭐⭐⭐⭐ Excellent |
| **Maintenance** | ⭐⭐ Coupled | ⭐⭐⭐⭐⭐ Decoupled | ⭐⭐⭐ Moderate |
| **Code Size** | ~300 lines | ~600 lines | ~450 lines |
| **Future Tasks** | ❌ Hard | ✅ Easy | ⚠️ Possible |
| **Future Plugins** | ❌ No | ✅ Yes | ❌ No |
| **Duplication** | ✅ None | ✅ Minimal | ❌ Some |

---

## Recommendations

### For Your Use Case: **Solution 2 (Trait-Based Providers)**

**Why:**
1. **You mentioned extensibility** - "flexible to build upon it, like hooking in not only actions but also other things like tasks or background tasks later on"
2. **Clean architecture** - Separates command definitions from execution
3. **Plugin-ready** - External crates could provide commands
4. **Best practices** - Follows patterns from Zed, VSCode, Helix

**Implementation Timeline:**
- **Week 1**: Core trait system + ShortcutCommandProvider (MVP)
- **Week 2**: UI rendering + fuzzy search
- **Week 3**: TaskCommandProvider (example extension)
- **Week 4**: Polish + documentation

### Quick Start: **Solution 1 (Direct Shortcuts)**

If you want something working in <2 hours:
- Reuses existing shortcuts directly
- Minimal changes to codebase
- Can always refactor to Solution 2 later

### Middle Ground: **Solution 3 (Static Registry)**

If you want extensibility without trait complexity:
- Static registry is easy to understand
- Can add task commands easily
- No dynamic dispatch overhead

---

## Next Steps

1. **Choose a solution** based on extensibility needs
2. **Add nucleo-matcher** dependency
3. **Create command_palette.rs** module
4. **Add state, actions, reducer** logic
5. **Implement rendering** (similar to shortcuts panel)
6. **Add Ctrl+P keybinding** to trigger
7. **Test with existing shortcuts**
8. **(Solution 2 only)** Add TaskCommandProvider example

---

## References

- **Nucleo**: https://github.com/helix-editor/nucleo (fuzzy matcher)
- **Zed Command Palette**: https://github.com/zed-industries/zed/tree/main/crates/command_palette
- **Helix Picker**: https://github.com/helix-editor/helix/blob/master/helix-term/src/ui/picker.rs
- **VSCode Commands**: https://code.visualstudio.com/api/extension-guides/command

---

**Author**: Claude Code
**Date**: 2025-11-19
**Status**: Proposed - awaiting implementation decision
