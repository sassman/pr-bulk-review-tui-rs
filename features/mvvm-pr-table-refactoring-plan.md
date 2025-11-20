# MVVM Pattern for PR Table View - Refactoring Plan

## Status: 📋 PLANNED

## Overview

Apply MVVM (Model-View-ViewModel) pattern to the PR table view, following the successful implementation for the log panel. This will separate presentation logic from business logic and simplify the view layer.

---

## Current Architecture Issues

### 1. Presentation Logic in Domain Model

**Problem:** `MergeableStatus` has presentation methods (similar to `JobStatus` we just fixed)

```rust
// pr.rs - VIOLATES MVVM
impl MergeableStatus {
    pub fn icon(&self) -> &str { /* ... */ }
    pub fn color(&self) -> Color { /* ... */ }
    pub fn label(&self) -> &str { /* ... */ }
}
```

**Issues:**
- ❌ Domain model knows about display concerns (icons, colors, labels)
- ❌ Can't test formatting without model dependencies
- ❌ Hardcoded colors instead of using theme system
- ❌ Violates separation of concerns

### 2. View Rendering in Model Layer

**Problem:** `From<&Pr> for Row` trait implements view logic in model

```rust
// pr.rs - VIOLATES MVVM
impl From<&Pr> for Row<'static> {
    fn from(val: &Pr) -> Self {
        let status_text = format!("{} {}", val.mergeable.icon(), val.mergeable.label());
        Row::new(vec![
            Cell::from(val.number.to_string()),
            Cell::from(val.title.clone()),
            Cell::from(val.author.clone()),
            Cell::from(val.no_comments.to_string()),
            Cell::from(status_text).style(Style::default().fg(val.mergeable.color())),
        ])
    }
}
```

**Issues:**
- ❌ Model directly creates ratatui widgets
- ❌ Formatting decisions in model (number.to_string(), formatting status)
- ❌ No access to theme colors
- ❌ Tight coupling between model and view library

### 3. Complex View Logic

**Problem:** `views/pull_requests.rs` has business and formatting logic

```rust
// views/pull_requests.rs
let status_text = match &repo_data.loading_state {
    LoadingState::Idle => "Idle [Ctrl+r to refresh]".to_string(),
    LoadingState::Loading => "Loading...".to_string(),
    // ... complex formatting logic
};

let rows = repo_data.prs.iter().enumerate().map(|(i, item)| {
    let color = match i % 2 {
        0 => app.store.state().repos.colors.normal_row_color,
        _ => app.store.state().repos.colors.alt_row_color,
    };
    let color = if repo_data.selected_pr_numbers.contains(&PrNumber::from_pr(item)) {
        app.store.state().theme.selected_bg
    } else {
        color
    };
    // ... more logic
});
```

**Issues:**
- ❌ View navigates model structures (`repo_data.loading_state`, `repo_data.prs`)
- ❌ View makes display decisions (status text, colors, alternating rows)
- ❌ View computes selection state
- ❌ Business logic mixed with rendering

---

## Proposed Solution: MVVM Architecture

### Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                    Redux Layer                               │
│                                                              │
│  State contains:                                            │
│  - Vec<Pr> (domain models)                                  │
│  - LoadingState, selected indices, filters                  │
│  - RepoData with PR list and metadata                       │
└─────────────────────────────────────────────────────────────┘
                           │
                           ↓ Transform (in reducer)
┌─────────────────────────────────────────────────────────────┐
│              View Model Layer (NEW)                          │
│                                                              │
│  PrTableViewModel {                                         │
│    header: PrTableHeaderViewModel                           │
│    rows: Vec<PrRowViewModel>                                │
│  }                                                           │
│                                                              │
│  - Pre-compute all display text                             │
│  - Pre-select colors from theme                             │
│  - Pre-determine row styles                                 │
│  - Format numbers, dates, status messages                   │
└─────────────────────────────────────────────────────────────┘
                           │
                           ↓ Render
┌─────────────────────────────────────────────────────────────┐
│                      View Layer                              │
│                                                              │
│  - Simple iteration over rows                                │
│  - Create widgets from pre-computed data                     │
│  - NO business logic, NO formatting decisions               │
└─────────────────────────────────────────────────────────────┘
```

---

## Implementation Plan

### Phase 1: Create View Model Types ✅

**File:** `view_models/pr_table.rs` (NEW)

```rust
use crate::pr::{Pr, MergeableStatus};
use crate::state::{LoadingState, PrNumber, RepoData};
use crate::theme::Theme;
use ratatui::style::Color;

/// View model for the entire PR table
#[derive(Debug, Clone)]
pub struct PrTableViewModel {
    /// Header with title and status
    pub header: PrTableHeaderViewModel,

    /// Pre-computed rows ready to display
    pub rows: Vec<PrRowViewModel>,

    /// Current cursor position (for keyboard navigation)
    pub cursor_index: Option<usize>,
}

/// View model for table header
#[derive(Debug, Clone)]
pub struct PrTableHeaderViewModel {
    /// Title text: "GitHub PRs: org/repo@branch"
    pub title: String,

    /// Status text: "Loaded [Ctrl+r to refresh]", etc.
    pub status_text: String,

    /// Status color (from theme)
    pub status_color: Color,
}

/// View model for a single PR row
#[derive(Debug, Clone)]
pub struct PrRowViewModel {
    /// Pre-formatted cell texts
    pub pr_number: String,      // "#123"
    pub title: String,           // "Fix: broken tests" (may be truncated)
    pub author: String,          // "sassman"
    pub comments: String,        // "5"
    pub status_text: String,     // "✓ Ready"

    /// Pre-computed styles
    pub bg_color: Color,         // Background (alternating, selected, etc.)
    pub fg_color: Color,         // Text color
    pub status_color: Color,     // Status-specific color

    /// Metadata for interactions (not displayed)
    pub pr_number_raw: usize,    // For opening PR
    pub is_selected: bool,       // Space key selection
    pub is_cursor: bool,         // Keyboard navigation position
    pub row_style: RowStyle,
}

/// Pre-determined row style
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStyle {
    Normal,           // Regular row
    Selected,         // Space-selected
    Cursor,           // Keyboard focus
    SelectedCursor,   // Both selected and focused
}

impl PrTableViewModel {
    /// Transform state into display-ready view model
    pub fn from_repo_data(
        repo_data: &RepoData,
        selected_repo: &crate::state::Repo,
        cursor_index: Option<usize>,
        theme: &Theme,
    ) -> Self {
        // Build header
        let header = Self::build_header(repo_data, selected_repo, theme);

        // Build rows
        let rows = repo_data
            .prs
            .iter()
            .enumerate()
            .map(|(index, pr)| {
                Self::build_row(pr, index, cursor_index, &repo_data.selected_pr_numbers, theme)
            })
            .collect();

        Self {
            header,
            rows,
            cursor_index,
        }
    }

    fn build_header(
        repo_data: &RepoData,
        selected_repo: &crate::state::Repo,
        theme: &Theme,
    ) -> PrTableHeaderViewModel {
        let title = format!(
            "GitHub PRs: {}/{}@{}",
            selected_repo.org, selected_repo.repo, selected_repo.branch
        );

        let (status_text, status_color) = Self::format_loading_state(&repo_data.loading_state, theme);

        PrTableHeaderViewModel {
            title,
            status_text,
            status_color,
        }
    }

    fn build_row(
        pr: &Pr,
        index: usize,
        cursor_index: Option<usize>,
        selected_prs: &std::collections::HashSet<PrNumber>,
        theme: &Theme,
    ) -> PrRowViewModel {
        // Pre-compute display text
        let pr_number = pr.number.to_string();
        let title = pr.title.clone(); // May truncate in future
        let author = pr.author.clone();
        let comments = pr.no_comments.to_string();

        // Format status with icon and label
        let status_icon = Self::mergeable_status_icon(pr.mergeable);
        let status_label = Self::mergeable_status_label(pr.mergeable);
        let status_text = format!("{} {}", status_icon, status_label);
        let status_color = Self::mergeable_status_color(pr.mergeable, theme);

        // Determine row state
        let is_selected = selected_prs.contains(&PrNumber::from_pr(pr));
        let is_cursor = cursor_index == Some(index);

        // Compute background color
        let bg_color = if is_cursor && is_selected {
            theme.active_bg // Both cursor and selected
        } else if is_cursor {
            theme.active_bg // Just cursor
        } else if is_selected {
            theme.selected_bg // Just selected (Space key)
        } else {
            // Alternating row colors
            if index % 2 == 0 {
                theme.table_row_bg_normal
            } else {
                theme.table_row_bg_alt
            }
        };

        let fg_color = if is_cursor {
            theme.active_fg // Yellow for cursor
        } else {
            theme.table_row_fg
        };

        let row_style = match (is_cursor, is_selected) {
            (true, true) => RowStyle::SelectedCursor,
            (true, false) => RowStyle::Cursor,
            (false, true) => RowStyle::Selected,
            (false, false) => RowStyle::Normal,
        };

        PrRowViewModel {
            pr_number,
            title,
            author,
            comments,
            status_text,
            bg_color,
            fg_color,
            status_color,
            pr_number_raw: pr.number,
            is_selected,
            is_cursor,
            row_style,
        }
    }

    /// Format loading state for display (view model responsibility)
    fn format_loading_state(state: &LoadingState, theme: &Theme) -> (String, Color) {
        match state {
            LoadingState::Idle => (
                "Idle [Ctrl+r to refresh]".to_string(),
                theme.text_muted,
            ),
            LoadingState::Loading => (
                "Loading...".to_string(),
                theme.status_warning,
            ),
            LoadingState::Loaded => (
                "Loaded [Ctrl+r to refresh]".to_string(),
                theme.status_success,
            ),
            LoadingState::Error(err) => {
                let err_short = if err.len() > 30 {
                    format!("{}...", &err[..30])
                } else {
                    err.clone()
                };
                (
                    format!("Error: {} [Ctrl+r to retry]", err_short),
                    theme.status_error,
                )
            }
        }
    }

    // --- Presentation helpers for MergeableStatus ---
    // (Moved from MergeableStatus impl)

    fn mergeable_status_icon(status: MergeableStatus) -> &'static str {
        match status {
            MergeableStatus::Unknown => "?",
            MergeableStatus::BuildInProgress => "⋯",
            MergeableStatus::Ready => "✓",
            MergeableStatus::NeedsRebase => "↻",
            MergeableStatus::BuildFailed => "✗",
            MergeableStatus::Conflicted => "✗",
            MergeableStatus::Blocked => "⊗",
            MergeableStatus::Rebasing => "⟳",
            MergeableStatus::Merging => "⇒",
        }
    }

    fn mergeable_status_color(status: MergeableStatus, theme: &Theme) -> Color {
        match status {
            MergeableStatus::Unknown => theme.text_muted,
            MergeableStatus::BuildInProgress => theme.status_warning,
            MergeableStatus::Ready => theme.status_success,
            MergeableStatus::NeedsRebase => theme.status_warning,
            MergeableStatus::BuildFailed => theme.status_error,
            MergeableStatus::Conflicted => theme.status_error,
            MergeableStatus::Blocked => theme.status_error,
            MergeableStatus::Rebasing => theme.status_info,
            MergeableStatus::Merging => theme.status_info,
        }
    }

    fn mergeable_status_label(status: MergeableStatus) -> &'static str {
        match status {
            MergeableStatus::Unknown => "Unknown",
            MergeableStatus::BuildInProgress => "Checking...",
            MergeableStatus::Ready => "Ready",
            MergeableStatus::NeedsRebase => "Needs Rebase",
            MergeableStatus::BuildFailed => "Build Failed",
            MergeableStatus::Conflicted => "Conflicts",
            MergeableStatus::Blocked => "Blocked",
            MergeableStatus::Rebasing => "Rebasing...",
            MergeableStatus::Merging => "Merging...",
        }
    }
}
```

---

### Phase 2: Remove Presentation from Domain Models ✅

**File:** `pr.rs` (MODIFY)

```rust
// Remove presentation methods from MergeableStatus
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MergeableStatus {
    Unknown,
    BuildInProgress,
    Ready,
    NeedsRebase,
    BuildFailed,
    Conflicted,
    Blocked,
    Rebasing,
    Merging,
}
// NO icon(), color(), label() methods!

// Remove From<&Pr> for Row trait
// impl From<&Pr> for Row<'static> { ... } ← DELETE THIS
```

---

### Phase 3: Add View Model to State ✅

**File:** `state.rs` (MODIFY)

```rust
#[derive(Debug, Clone, Default)]
pub struct RepoData {
    pub prs: Vec<Pr>,
    pub table_state: TableState,
    pub selected_pr_numbers: HashSet<PrNumber>,
    pub loading_state: LoadingState,
    pub auto_merge_queue: Vec<AutoMergePR>,
    pub operation_monitor_queue: Vec<OperationMonitor>,

    /// Cached view model (recomputed when PR data changes)
    pub pr_table_view_model: Option<crate::view_models::pr_table::PrTableViewModel>,  // NEW
}
```

---

### Phase 4: Update Reducer to Compute View Model ✅

**File:** `reducer.rs` (MODIFY)

```rust
fn repos_reducer(
    mut state: ReposState,
    action: &Action,
    theme: &crate::theme::Theme,  // Add theme parameter
) -> (ReposState, Vec<Effect>) {
    match action {
        Action::PrsLoaded(prs) => {
            let repo_data = state.repo_data.entry(state.selected_repo).or_default();
            repo_data.prs = prs.clone();
            repo_data.loading_state = LoadingState::Loaded;

            // Recompute view model
            recompute_pr_table_view_model(&mut state, theme);  // NEW
        }

        Action::SelectNextPr | Action::SelectPrevPr => {
            // Cursor changed, recompute view model
            // ... update cursor in table_state ...
            recompute_pr_table_view_model(&mut state, theme);  // NEW
        }

        Action::TogglePrSelection => {
            // Selection changed, recompute view model
            // ... toggle selection ...
            recompute_pr_table_view_model(&mut state, theme);  // NEW
        }

        Action::CycleFilter => {
            // Filter changed, affects visible PRs
            // ... update filter ...
            recompute_pr_table_view_model(&mut state, theme);  // NEW
        }

        // ... all actions that change PR display
    }
}

/// Helper to recompute PR table view model
fn recompute_pr_table_view_model(state: &mut ReposState, theme: &crate::theme::Theme) {
    if let Some(selected_repo) = state.recent_repos.get(state.selected_repo) {
        let repo_data = state.repo_data.entry(state.selected_repo).or_default();

        let cursor_index = repo_data.table_state.selected();

        repo_data.pr_table_view_model = Some(
            crate::view_models::pr_table::PrTableViewModel::from_repo_data(
                repo_data,
                selected_repo,
                cursor_index,
                theme,
            )
        );
    }
}
```

---

### Phase 5: Simplify View to Pure Presentation ✅

**File:** `views/pull_requests.rs` (REFACTOR)

**Before:** ~125 lines with complex logic
**After:** ~60 lines of pure rendering

```rust
use crate::view_models::pr_table::PrTableViewModel;
use ratatui::{prelude::*, widgets::*};

/// Render the PR table from view model (pure presentation)
pub fn render_pr_table(f: &mut Frame, area: Rect, view_model: &PrTableViewModel, theme: &Theme) {
    // Build header
    let header_cells = ["#PR", "Description", "Author", "#Comments", "Status"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default()
            .fg(theme.table_header_fg)
            .bg(theme.table_header_bg)
        ));

    let header = Row::new(header_cells)
        .style(Style::default().bg(theme.table_header_bg))
        .height(1);

    // Build block with header and status
    let block = Block::default()
        .title(view_model.header.title.clone())
        .title(
            Line::from(view_model.header.status_text.clone())
                .style(Style::default().fg(view_model.header.status_color))
                .right_aligned()
        )
        .borders(Borders::ALL);

    // Check if empty
    if view_model.rows.is_empty() {
        let message = match &view_model.header.status_text {
            s if s.contains("Loading") => "Loading pull requests...",
            s if s.contains("Error") => "Error loading data. Press Ctrl+r to retry.",
            _ => "No pull requests found matching filter",
        };

        let paragraph = Paragraph::new(message)
            .block(block)
            .style(Style::default().fg(theme.text_muted))
            .alignment(ratatui::layout::Alignment::Center);

        f.render_widget(paragraph, area);
        return;
    }

    // Build rows - simple iteration, no logic!
    let rows = view_model.rows.iter().map(|row_vm| {
        Row::new(vec![
            Cell::from(row_vm.pr_number.clone()),
            Cell::from(row_vm.title.clone()),
            Cell::from(row_vm.author.clone()),
            Cell::from(row_vm.comments.clone()),
            Cell::from(row_vm.status_text.clone())
                .style(Style::default().fg(row_vm.status_color)),
        ])
        .style(Style::default()
            .fg(row_vm.fg_color)
            .bg(row_vm.bg_color))
        .height(1)
    });

    let widths = [
        Constraint::Percentage(8),
        Constraint::Percentage(50),
        Constraint::Percentage(15),
        Constraint::Percentage(10),
        Constraint::Percentage(17),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block);

    f.render_widget(table, area);
}
```

**Changes:**
- ✅ No model navigation
- ✅ No business logic
- ✅ No formatting decisions
- ✅ Simple iteration over pre-computed rows
- ✅ Just creates widgets from view model data

---

### Phase 6: Update Main Render Loop ✅

**File:** `main.rs` (MODIFY)

```rust
// Before:
crate::views::pull_requests::render_pr_table(f, table_area, app);

// After:
if let Some(ref view_model) = app.store.state().repos.repo_data
    .get(&app.store.state().repos.selected_repo)
    .and_then(|rd| rd.pr_table_view_model.as_ref())
{
    crate::views::pull_requests::render_pr_table(
        f,
        table_area,
        view_model,
        &app.store.state().theme,
    );
} else {
    // Fallback: show loading or error
    // ...
}
```

---

## Expected Benefits

### 1. Code Metrics
- **View complexity reduction**: ~125 lines → ~60 lines (52% reduction)
- **Separation of concerns**: Clear boundaries between layers
- **Testability**: Can test formatting without rendering

### 2. Maintainability
- ✅ Display format changes only touch view model
- ✅ Can change status icons/colors without touching model
- ✅ Theme colors properly used (not hardcoded)
- ✅ New contributors follow clear pattern

### 3. Performance
- ✅ View model cached in state
- ✅ Recomputed only when data changes
- ✅ No per-frame formatting/decision logic

### 4. Architecture
- ✅ Consistent with log panel MVVM implementation
- ✅ Model layer clean (no presentation concerns)
- ✅ View layer simple (pure presentation)
- ✅ All presentation in one place (view model)

---

## Implementation Checklist

### Phase 1: Setup
- [ ] Create `view_models/pr_table.rs`
- [ ] Implement `PrTableViewModel` struct
- [ ] Implement `PrTableHeaderViewModel` struct
- [ ] Implement `PrRowViewModel` struct
- [ ] Add presentation helper methods
- [ ] Export from `view_models/mod.rs`

### Phase 2: Clean Model Layer
- [ ] Remove `icon()` from `MergeableStatus`
- [ ] Remove `color()` from `MergeableStatus`
- [ ] Remove `label()` from `MergeableStatus`
- [ ] Remove `impl From<&Pr> for Row<'static>`

### Phase 3: State Integration
- [ ] Add `pr_table_view_model` field to `RepoData`
- [ ] Update `RepoData::default()` impl

### Phase 4: Reducer Updates
- [ ] Add theme parameter to `repos_reducer()`
- [ ] Create `recompute_pr_table_view_model()` helper
- [ ] Call recompute for `PrsLoaded` action
- [ ] Call recompute for cursor navigation actions
- [ ] Call recompute for selection toggle actions
- [ ] Call recompute for filter change actions
- [ ] Call recompute for all actions that change display

### Phase 5: Simplify View
- [ ] Refactor `render_pr_table()` signature (take view model)
- [ ] Remove model navigation from view
- [ ] Remove formatting logic from view
- [ ] Remove color/style decisions from view
- [ ] Simple iteration over `view_model.rows`

### Phase 6: Integration
- [ ] Update `main.rs` render call
- [ ] Handle missing view model case
- [ ] Test with empty PR list
- [ ] Test with loading state
- [ ] Test with error state

### Phase 7: Testing & Verification
- [ ] Build succeeds
- [ ] All tests pass
- [ ] Clippy clean
- [ ] Manual testing: navigation works
- [ ] Manual testing: selection works
- [ ] Manual testing: filtering works
- [ ] Manual testing: status display correct

---

## Migration Notes

### Breaking Changes
None - all changes are internal refactoring

### Compatibility
- Existing reducer actions unchanged
- Existing state structure compatible (only additions)
- View API changes contained to internal render functions

### Risk Assessment
- **Low Risk**: Following proven pattern from log panel
- **Low Impact**: Internal refactoring, no user-facing changes
- **High Benefit**: Major architecture improvement

---

## Open Questions

1. **Title truncation**: Should we truncate long PR titles in view model?
   - **Decision**: Yes, implement `truncate_with_ellipsis()` helper

2. **Date formatting**: Should we format `created_at`/`updated_at` dates?
   - **Decision**: Not in v1, add later if needed

3. **Sorting**: Should view model handle sorting?
   - **Decision**: No, sorting is business logic (stays in reducer)

4. **Filtering**: Should view model handle filtering?
   - **Decision**: No, filtering is business logic (stays in reducer)

---

## Success Criteria

After implementation:
- ✅ `MergeableStatus` has no presentation methods
- ✅ No `From<&Pr> for Row` trait
- ✅ View model cached in `RepoData`
- ✅ View is <100 lines and pure presentation
- ✅ All tests pass
- ✅ Clippy clean
- ✅ Theme colors properly used everywhere
- ✅ Manual testing confirms no regressions

---

## Timeline

**Estimated effort**: 2-3 hours

1. Phase 1-2 (Setup & Clean): 45 minutes
2. Phase 3-4 (State & Reducer): 30 minutes
3. Phase 5-6 (View & Integration): 30 minutes
4. Phase 7 (Testing): 30 minutes
5. Buffer for issues: 30 minutes

---

## References

- [MVVM Pattern Design Doc](./mvvm-view-model-pattern.md)
- [Log Panel MVVM Implementation](../crates/gh-pr-tui/src/view_models/log_panel.rs)
- Current code: `crates/gh-pr-tui/src/pr.rs`
- Current code: `crates/gh-pr-tui/src/views/pull_requests.rs`

---

## Next Steps

1. Review this plan with team
2. Get approval to proceed
3. Implement phase by phase
4. Test thoroughly
5. Commit with detailed message
6. Update MVVM design doc with PR table completion
