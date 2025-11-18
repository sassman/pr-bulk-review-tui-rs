# GitHub PR TUI (`gh-pr-tui`)

A powerful Terminal User Interface (TUI) tool for efficiently reviewing and managing GitHub Pull Requests across multiple repositories.

## Disclaimer

This tool is experimental, solves my personal pain points and vibe coded with the help of claude. Before contriubuting a PR please open an issue to discuss the change, to not waste your time.

Also note that every feature is a subject of change as I continue to use and refine the tool.

## Features at a Glance

- **Multi-Repository Management** - Switch between multiple repos with tabs
- **Smart PR Filtering** - Filter by type (feat/fix/chore) or status (ready/build failed)
- **Bulk Operations** - Select and merge/rebase multiple PRs at once
- **Merge Bot** - Automated merge queue with intelligent rebase handling
- **CI Integration** - View build status, logs, and rerun failed jobs
- **Hierarchical Log Viewer** - Tree-based log navigation with smart error jumping
- **Live Status Updates** - Background checks for merge status and rebase needs
- **IDE Integration** - Open PRs directly in your IDE for quick review
- **Keyboard-Driven** - Complete workflow without touching the mouse
- **Session Persistence** - Resume exactly where you left off
- **Redux Architecture** - Predictable state management with pure reducers

## Installation

```bash
# Clone the repository
git clone https://github.com/sassman/gh-pr-tui-rs.git
cd gh-pr-tui-rs

# Build and run
cargo build --release
./target/release/gh-pr-tui

# Or run directly with cargo
cargo run --bin gh-pr-tui
```

## Configuration

Create a `.env` file in the project root or set environment variables:

```bash
GITHUB_TOKEN=your_github_personal_access_token
```

Create a `.recent-repositories.json` file to configure your repositories:

```json
[
  {
    "org": "your-org",
    "repo": "your-repo"
  },
  {
    "org": "another-org",
    "repo": "another-repo"
  }
]
```

## Quick Start

1. Launch the tool: `./target/release/gh-pr-tui` (or `cargo run --bin gh-pr-tui`)
2. Use `Tab` or `/` to switch between repositories
3. Use `↑/↓` or `j/k` to navigate PRs
4. Press `Space` to select/deselect PRs (automatically advances to next PR)
5. Press `m` to merge selected PRs
6. Press `l` to view build logs with smart error navigation
7. Press `i` to open PR in IDE (or main branch if no PRs)
8. Press `?` for complete keyboard shortcuts

---

## Detailed Features

### Multi-Repository Management

**The Problem:** As a developer or maintainer working across multiple repositories, you constantly switch between browser tabs, lose context, and waste time navigating GitHub's interface. Checking PRs across 5 repositories means opening 5+ browser tabs, clicking through each repo's PR list, and mentally tracking which repos you've already reviewed. This context switching takes **3-5 minutes per repository** just to get oriented.

**The Solution:** This tool provides a tabbed interface where you can switch between repositories instantly with a single keypress (`Tab` or `/`). Your position in each repo's PR list is preserved, so you never lose context. Review 10 repositories in the time it used to take to review 2.

### Smart PR Filtering

**The Problem:** Large repositories can have 50-100 open PRs at any time. Finding the PRs that need your attention means manually scrolling through the list, checking labels, reading titles, and filtering mentally. Want to see only PRs with failing builds? That requires clicking filters, waiting for page loads, and GitHub's filter syntax. This manual filtering wastes **2-3 minutes per search**.

**The Solution:** Press `f` to instantly cycle through filters: None → Ready to Merge → Build Failed → All. The filter applies immediately with zero latency, showing exactly the PRs that match your criteria. No clicking, no page loads, no typing filter queries.

### Bulk Merge Operations

**The Problem:** Merging multiple dependabot PRs or approved feature PRs is tedious in GitHub's web interface. Each merge requires: click the PR → scroll to bottom → click "Merge" → confirm → wait for page load → navigate back → repeat. Merging 10 PRs manually takes **5-10 minutes** of repetitive clicking.

**The Solution:** Select multiple PRs with `Space`, then press `m` to merge all selected PRs in one operation. The tool handles the GitHub API calls in parallel and provides real-time feedback. Merge 20 PRs in under 30 seconds.

### Automated Merge Bot

**The Problem:** Maintaining a repository with CI checks means PRs often become stale while waiting for builds, or they need rebasing before they can merge. The manual workflow is: check if PR is ready → check if it needs rebase → rebase if needed → wait for CI → check again → merge → repeat for next PR. This supervision can take **hours of intermittent checking** across a day.

**The Solution:** Press `Ctrl+m` to start the merge bot. It automatically monitors selected PRs, rebases them when needed, waits for CI to pass, and merges them when ready. The bot works autonomously, handling the entire queue while you focus on code review or other work.

### Rebase PRs

**The Problem:** GitHub's web interface requires manual rebasing: click the PR → scroll down → find the rebase button (if available) → click → confirm → wait for the page to reload. For repositories with many dependabot PRs or fast-moving main branches, you might need to rebase **dozens of PRs daily**. Each rebase takes **30-60 seconds** in the browser.

**The Solution:** Select PRs and press `r` to rebase all selected PRs at once. The tool uses dependabot's comment-based rebase for bot PRs and GitHub's API for regular PRs. Rebase 15 PRs in the time it takes to manually rebase 2.

### CI Status and Build Logs

**The Problem:** When a PR build fails, you need to: click the PR → click "Details" next to the failed check → navigate through GitHub Actions UI → find the failed job → expand the error section → scroll through logs. For complex builds with multiple jobs, finding the actual error can take **5-10 minutes per failed PR**.

**The Solution:** Press `l` on any PR to instantly view parsed build logs in a hierarchical tree view. The tool fetches all workflow runs, parses GitHub Actions logs with ANSI color preservation, and presents them in an expandable workflow → job → step → log line tree structure.

**Smart Error Navigation:** Press `n` to jump to the next error. The navigation is intelligent:
- First, it jumps through error lines within the current step (e.g., `error[E0425]`, `error: could not compile`)
- When no more errors in the step, it jumps to the next failed step
- When no more failed steps in the job, it jumps to the next failed job
- Press `p` to navigate backwards through errors the same way

**Log Viewer Features:**
- Tree navigation with `j/k` to move through workflows/jobs/steps/log lines
- Expand/collapse nodes with `Enter` to focus on specific sections
- Page down with `Space` for quick log browsing
- Horizontal scrolling with `h/l` for long lines
- Toggle timestamps with `t` for cleaner view
- Command invocations highlighted in yellow
- Error messages highlighted in red and bold
- Proper ANSI color rendering from build tools (cargo, rustc, etc.)

### Rerun Failed CI Jobs

**The Problem:** Flaky tests or transient CI failures require rerunning workflows. In GitHub's web interface: click the PR → click "Checks" tab → find the failed workflow → click "Re-run jobs" → select "Re-run failed jobs" → confirm. For multiple PRs with flaky tests, this becomes **2-3 minutes per PR**.

**The Solution:** Select PRs and press `Shift+R` to rerun all failed CI jobs for selected PRs (or just the current PR). The tool finds all failed workflow runs and triggers reruns via the GitHub API, handling multiple PRs in seconds.

### IDE Integration

**The Problem:** Reviewing code in GitHub's web interface is limiting - no syntax highlighting from your preferred theme, no code intelligence, no ability to run tests locally. To review in your IDE: copy the PR branch name → open terminal → git fetch → git checkout → wait for IDE to index. This takes **1-2 minutes per PR** and breaks your flow. Similarly, working on the main branch requires manual git commands.

**The Solution:** Press `i` on any PR to open it directly in your IDE. The tool handles the git operations and IDE invocation, getting you into the code in seconds. When no PR is selected (empty list), pressing `i` opens the main branch with latest changes - perfect for starting new work or reviewing the current state of the repository.

### Background Status Checks

**The Problem:** PR status changes constantly - CI finishes, conflicts appear, reviews are approved. Keeping PR information up-to-date means manually refreshing the page every few minutes or getting stale information. You might start merging a PR only to discover it now has conflicts, wasting time.

**The Solution:** The tool automatically checks merge status and rebase requirements in the background, updating the display in real-time. You always see current information without manual refreshing.

### Keyboard-Driven Workflow

**The Problem:** Using GitHub's web interface requires constant mouse movement: scroll, click, scroll more, click again. The hand movement between keyboard and mouse, combined with page loads and animations, creates micro-delays that add up. A typical PR review session involves **hundreds of mouse clicks** and significant hand movement.

**The Solution:** Every action has a keyboard shortcut. Navigate with `j/k`, select with `Space`, merge with `m`, rebase with `r`, open in browser with `Enter`. Your hands stay on the keyboard, actions are instant, and muscle memory develops quickly. Press `?` to see all shortcuts.

### Session Persistence

**The Problem:** You're reviewing PRs across 8 repositories. You've selected 5 PRs for merging, filtered to show only failing builds in one repo, and you're halfway through another repo's list. Then you need to close your terminal or restart. When you come back, you have to: reopen GitHub → navigate to each repo → try to remember which PRs you'd selected → try to remember where you were. This context loss wastes **5-10 minutes** reconstructing your state.

**The Solution:** The tool saves your entire session state - selected PRs, current position in each repo, active filters, and which repo tab you're on. When you restart, you're exactly where you left off, ready to continue immediately.

---

## Keyboard Shortcuts

### Navigation
- `↑/↓` or `j/k` - Navigate through PRs
- `Tab` or `/` - Switch to next repository
- `Shift+Tab` - Switch to previous repository
- `1-9` - Jump to repository by number

### PR Actions
- `Space` - Select/deselect PR (auto-advances to next)
- `m` - Merge selected PRs
- `Ctrl+m` - Start merge bot (auto-merge + rebase queue)
- `r` - Rebase selected PRs (or auto-rebase if none selected)
- `Shift+R` - Rerun failed CI jobs for current/selected PRs
- `i` - Open PR in IDE (or main branch if no PRs)
- `l` - View build logs
- `Enter` - Open PR in browser

### Filters & Views
- `f` - Cycle PR filter (None/Ready/Build Failed)
- `Ctrl+r` - Refresh current repository

### Log Panel (when open)
- `↑/↓` or `j/k` - Navigate through tree (workflows/jobs/steps/logs)
- `Enter` - Expand/collapse tree node
- `Space` - Page down (scroll by screen height)
- `←/→` or `h/l` - Scroll horizontally
- `n` - Jump to next failed step/job (smart error navigation)
- `p` - Jump to previous failed step/job (smart error navigation)
- `t` - Toggle timestamps
- `x` or `Esc` - Close log panel

### Debug Console
- `` ` `` or `~` - Toggle debug console (Quake-style drop-down)
- `j/k` (when console open) - Scroll debug console
- `a` (when console open) - Toggle auto-scroll
- `c` (when console open) - Clear debug logs

### General
- `?` - Toggle keyboard shortcuts help
- `p → a` - Add new repository
- `p → d` - Drop/remove current repository
- `q` - Quit application

---

## Architecture

This tool follows the Redux/Elm architecture pattern:

- **Pure Reducers** - All state transitions are pure functions that take (state, action) and return (new state, effects)
- **Effect System** - Side effects are declarative values that the runtime executes
- **Centralized State** - Single source of truth for application state
- **Predictable Updates** - State changes only through actions dispatched to reducers

This architecture ensures:
- Testability: Pure reducers are easy to test
- Predictability: State changes are explicit and traceable
- Maintainability: Clear separation between logic and side effects

---

## Development

```bash
# Run in development mode with logging
RUST_LOG=debug cargo run --bin gh-pr-tui

# Run tests
cargo test

# Build optimized release binary
cargo build --release

# Run the release binary
./target/release/gh-pr-tui
```

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR.
