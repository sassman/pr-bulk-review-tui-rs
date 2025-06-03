use anyhow::{Context, Result, bail};
use log::debug;
use octocrab::{Octocrab, issues::IssueHandler, params};
use pr::Pr;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        event::{KeyEvent, KeyModifiers},
        terminal,
    },
    layout::Constraint,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Cell, Row, Table, TableState},
};
use ratatui::{
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
    style::palette::tailwind,
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::File,
    io::{self, BufReader},
};
use tokio::runtime::Runtime;

mod pr;

const PALETTES: [tailwind::Palette; 4] = [
    tailwind::BLUE,
    tailwind::EMERALD,
    tailwind::INDIGO,
    tailwind::RED,
];

struct TableColors {
    buffer_bg: Color,
    header_bg: Color,
    header_fg: Color,
    row_fg: Color,
    selected_row_style_fg: Color,
    selected_column_style_fg: Color,
    selected_cell_style_fg: Color,
    normal_row_color: Color,
    alt_row_color: Color,
    footer_border_color: Color,
}

impl TableColors {
    const fn new(color: &tailwind::Palette) -> Self {
        Self {
            buffer_bg: tailwind::SLATE.c950,
            header_bg: color.c900,
            header_fg: tailwind::SLATE.c200,
            row_fg: tailwind::SLATE.c200,
            selected_row_style_fg: color.c400,
            selected_column_style_fg: color.c400,
            selected_cell_style_fg: color.c600,
            normal_row_color: tailwind::SLATE.c950,
            alt_row_color: tailwind::SLATE.c900,
            footer_border_color: color.c400,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum LoadingState {
    #[default]
    Idle,
    Loading,
    Loaded,
    Error(String),
}

struct App {
    state: TableState,
    prs: Vec<Pr>,
    recent_repos: Vec<Repo>,
    selected_repo: usize,
    filter: PrFilter,
    selected_prs: Vec<usize>,
    colors: TableColors,
    loading_state: LoadingState,
}

#[derive(Debug, Serialize, Deserialize, Eq, Clone, PartialEq)]
struct PersistedState {
    selected_repo: Repo,
}

#[derive(Debug, Serialize, Deserialize, Eq, Clone, PartialEq)]
struct Repo {
    org: String,
    repo: String,
    branch: String,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PrFilter {
    title: String,
}

impl Repo {
    fn new(org: &str, repo: &str, branch: &str) -> Repo {
        Repo {
            org: org.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
        }
    }
}

impl App {
    fn new(recent_repos: Vec<Repo>) -> App {
        App {
            state: TableState::default(),
            prs: Vec::new(),
            recent_repos,
            selected_repo: 0,
            filter: PrFilter {
                title: "chore".to_string(),
            },
            selected_prs: Vec::new(),
            colors: TableColors::new(&PALETTES[0]),
            loading_state: LoadingState::Idle,
        }
    }

    fn octocrab(&self) -> Result<Octocrab> {
        Ok(Octocrab::builder()
            .personal_token(
                env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN environment variable must be set"),
            )
            .build()?)
    }

    fn repo(&self) -> &Repo {
        &self.recent_repos[self.selected_repo]
    }

    async fn restore_session(&mut self, state: PersistedState) -> Result<()> {
        // Restore the selected repository from the persisted state
        if let Some(index) = self
            .recent_repos
            .iter()
            .position(|r| r == &state.selected_repo)
        {
            self.selected_repo = index;
            self.fetch_data().await?;
            self.state.select(Some(0));
        } else {
            bail!("Selected repository not found in recent repositories");
        }

        Ok(())
    }

    /// Fetch data from GitHub for the selected repository and filter
    async fn fetch_data(&mut self) -> Result<()> {
        self.loading_state = LoadingState::Loading;
        // TODO: use tokio::spawn to fetch data in the background
        // TODO: this requires Arc and RwLock to share state between threads
        let github_data = fetch_github_data(&self.octocrab()?, &self.repo(), &self.filter).await?;
        self.prs = github_data;

        self.loading_state = LoadingState::Loaded;

        Ok(())
    }

    /// Move to the next PR in the list
    fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i < self.prs.len() - 1 {
                    i + 1
                } else {
                    i
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    /// Move to the previous PR in the list
    fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i > 0 {
                    i - 1
                } else {
                    i
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    /// Toggle the selection of the currently selected PR
    fn select_toggle(&mut self) {
        let i = self.state.selected().unwrap_or(0);
        if self.selected_prs.contains(&i) {
            self.selected_prs.retain(|&x| x != i);
        } else {
            self.selected_prs.push(i);
        }
    }

    /// todo: This should be opening a pop-up dialog to let the user type in a org, repo, and branch
    /// Here is the cheap version that just cycles through the recent repos
    async fn select_next_repo(&mut self) -> Result<()> {
        let next_repo_index = (self.selected_repo + 1) % self.recent_repos.len();

        self.selected_repo = next_repo_index;
        self.select_repo().await?;

        Ok(())
    }

    async fn select_repo(&mut self) -> Result<()> {
        // This function is a placeholder for future implementation
        // It could be used to select a specific repo from a list or input
        self.selected_prs.clear();
        self.fetch_data().await?;
        self.state.select(Some(0));
        debug!("Selecting repo: {}", self.repo().repo);
        Ok(())
    }

    /// Exit the application
    fn exit(&mut self) -> Result<()> {
        bail!("Exiting the application")
    }

    /// Rebase the selected PRs
    async fn rebase(&mut self) -> Result<()> {
        // for all selected PRs, authored by `dependabot` we rebase by adding the commend `@dependabot rebase`

        let octocrab = self.octocrab()?;
        for &pr_index in &self.selected_prs {
            if let Some(pr) = self.prs.get(pr_index) {
                if pr.author.starts_with("dependabot") {
                    debug!("Rebasing PR #{}", pr.number);

                    comment(&octocrab, self.repo(), pr, "@dependabot rebase").await?;
                } else {
                    debug!("Skipping PR #{} authored by {}", pr.number, pr.author);
                }
            } else {
                debug!("No PR found at index {}", pr_index);
            }
        }
        debug!("Rebasing selected PRs: {:?}", self.selected_prs);

        Ok(())
    }
}

async fn fetch_github_data<'a>(
    octocrab: &Octocrab,
    repo: &Repo,
    filter: &PrFilter,
) -> Result<Vec<Pr>> {
    // Fetch some repos from the Rust organization as an example
    let page = octocrab
        .pulls(&repo.org, &repo.repo)
        .list()
        .state(params::State::Open)
        .head(&repo.branch)
        .sort(params::pulls::Sort::Updated)
        .direction(params::Direction::Ascending)
        .per_page(100)
        .send()
        .await?;

    let mut prs = Vec::new();

    for pr in page.items.into_iter().filter(|pr| {
        pr.title
            .as_ref()
            .unwrap_or(&"".to_string())
            .contains(&filter.title)
    }) {
        let pr = Pr::from_pull_request(&pr, repo, &octocrab).await;
        prs.push(pr);
    }

    Ok(prs)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let recent_repos = loading_recent_repos()?;
    let persisted_state = load_persisted_state();
    let mut app = App::new(recent_repos);

    if let Ok(state) = persisted_state {
        app.restore_session(state).await?;
    } else {
        app.select_repo().await?;
    }

    // Main loop
    loop {
        terminal.draw(|f| {
            let selected_repo = app.repo();
            let size = f.area();

            let loading_state = Line::from(format!("{:?}", app.loading_state)).right_aligned();

            let block = Block::default()
                .title(format!(
                    "[/] GitHub PRs: {}/{}@{}",
                    &selected_repo.org, &selected_repo.repo, &selected_repo.branch
                ))
                .title(loading_state)
                .borders(Borders::ALL);

            let header_style = Style::default()
                .fg(app.colors.header_fg)
                .bg(app.colors.header_bg);

            let header_cells = ["#PR", "Description", "Author", "#Comments", "Mergable"]
                .iter()
                .map(|h| Cell::from(*h).style(header_style));

            let header = Row::new(header_cells)
                .style(Style::default().bg(Color::Blue))
                .height(1);

            let selected_row_style = Style::default()
                .add_modifier(Modifier::REVERSED)
                .fg(app.colors.selected_row_style_fg);

            let rows = app.prs.iter().enumerate().map(|(i, item)| {
                let color = match i % 2 {
                    0 => app.colors.normal_row_color,
                    _ => app.colors.alt_row_color,
                };
                let color = if app.selected_prs.contains(&i) {
                    app.colors.selected_cell_style_fg
                } else {
                    color
                };
                let row: Row = item.into();
                row.style(Style::new().fg(app.colors.row_fg).bg(color))
                    .height(1)
            });

            let widths = [
                Constraint::Percentage(10),
                Constraint::Percentage(70),
                Constraint::Percentage(10),
                Constraint::Percentage(10),
            ];

            let table = Table::new(rows, widths)
                .header(header)
                .block(block)
                .row_highlight_style(selected_row_style);

            f.render_stateful_widget(table, size, &mut app.state);
        })?;

        if let Err(e) = handle_events(&mut app).await {
            debug!("Error handling events: {}", e);
            break;
        }
    }

    // Store recent repositories to a file
    if let Err(e) = store_recent_repos(&app.recent_repos) {
        debug!("Error storing recent repositories: {}", e);
    }

    // Store the current state to a file
    let state = PersistedState {
        selected_repo: app.repo().clone(),
    };
    if let Err(e) = store_persisted_state(&state) {
        debug!("Error storing persisted state: {}", e);
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

async fn handle_events(app: &mut App) -> Result<()> {
    match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key_event(app, key).await,
        _ => Ok(()),
    }
}

async fn handle_key_event(app: &mut App, key: KeyEvent) -> Result<()> {
    // let shift_pressed = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Char('q') => app.exit(),
        KeyCode::Char('r') => app.rebase().await,
        KeyCode::Char('/') => app.select_next_repo().await,
        KeyCode::Char('j') | KeyCode::Down => {
            app.next();
            Ok(())
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.previous();
            Ok(())
        }
        KeyCode::Char(' ') => {
            app.select_toggle();
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn comment(octocrab: &Octocrab, repo: &Repo, pr: &Pr, body: &str) -> Result<()> {
    let issue = octocrab.issues(&repo.org, &repo.repo);
    issue.create_comment(pr.number as _, body).await?;

    Ok(())
}

/// loading recent repositories from a local config file, that is just json file
fn loading_recent_repos() -> Result<Vec<Repo>> {
    let repos = if let Ok(recent_repos) = File::open(".recent-repositories.json") {
        let reader = BufReader::new(recent_repos);
        serde_json::from_reader(reader).context("Failed to parse recent repositories from file")?
    } else {
        debug!("No recent repositories file found, using default repositories");
        vec![
            Repo::new("cargo-generate", "cargo-generate", "main"),
            Repo::new("steganogram", "stegano-rs", "main"),
            Repo::new("rust-lang", "rust", "master"),
        ]
    };

    debug!("Loaded recent repositories: {:?}", repos);

    Ok(repos)
}

/// Storing recent repositories to a local json config file
fn store_recent_repos(repos: &[Repo]) -> Result<()> {
    let file = File::create(".recent-repositories.json")
        .context("Failed to create recent repositories file")?;
    serde_json::to_writer_pretty(file, &repos)
        .context("Failed to write recent repositories to file")?;

    debug!("Stored recent repositories: {:?}", repos);

    Ok(())
}

fn store_persisted_state(state: &PersistedState) -> Result<()> {
    let file = File::create(".session.json").context("Failed to create persisted state file")?;
    serde_json::to_writer_pretty(file, state).context("Failed to write persisted state to file")?;

    debug!("Stored persisted state: {:?}", state);

    Ok(())
}

fn load_persisted_state() -> Result<PersistedState> {
    let file = File::open(".session.json").context("Failed to open persisted state file")?;
    let reader = BufReader::new(file);
    let state: PersistedState =
        serde_json::from_reader(reader).context("Failed to parse persisted state from file")?;

    debug!("Loaded persisted state: {:?}", state);

    Ok(state)
}
