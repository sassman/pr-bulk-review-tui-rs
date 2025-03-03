use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use octocrab::{Octocrab, params};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::Constraint,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table, TableState},
};
use std::{error::Error, io};
use tokio::runtime::Runtime;

struct App {
    state: TableState,
    items: Vec<Vec<String>>,
    repo: (String, String),
}

impl App {
    fn new(items: Vec<Vec<String>>) -> App {
        App {
            state: TableState::default(),
            items,
            repo: ("cargo-generate".to_string(), "cargo-generate".to_string()),
        }
    }

    fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }
}

async fn fetch_github_data() -> Result<Vec<Vec<String>>, Box<dyn Error>> {
    let octocrab = Octocrab::builder().build()?;

    // Fetch some repos from the Rust organization as an example
    let page = octocrab
        .pulls("cargo-generate", "cargo-generate")
        .list()
        .state(params::State::Open)
        .head("main")
        .sort(params::pulls::Sort::Updated)
        .direction(params::Direction::Ascending)
        .per_page(100)
        .send()
        .await?;

    let mut items = Vec::new();

    for pr in page.items.into_iter().filter(|pr| {
        pr.title
            .as_ref()
            .unwrap_or(&"".to_string())
            .contains("chore")
    }) {
        let row = vec![
            pr.id.to_string(),
            pr.title.unwrap_or_default(),
            pr.comments.unwrap_or_default().to_string(),
            pr.mergeable_state
                .map(|merge_state| match merge_state {
                    octocrab::models::pulls::MergeableState::Behind => "n",
                    octocrab::models::pulls::MergeableState::Blocked => "n",
                    octocrab::models::pulls::MergeableState::Clean => "y",
                    octocrab::models::pulls::MergeableState::Dirty => "n",
                    octocrab::models::pulls::MergeableState::Draft => "n",
                    octocrab::models::pulls::MergeableState::HasHooks => "n",
                    octocrab::models::pulls::MergeableState::Unknown => "n",
                    octocrab::models::pulls::MergeableState::Unstable => "n",
                    _ => todo!(),
                })
                .unwrap_or("na")
                .to_string(),
        ];
        items.push(row);
    }

    Ok(items)
}

fn main() -> Result<(), Box<dyn Error>> {
    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create runtime and fetch data
    let rt = Runtime::new()?;
    let github_data = rt.block_on(fetch_github_data())?;

    // Create app state
    let mut app = App::new(github_data);
    app.state.select(Some(0));

    // Main loop
    loop {
        terminal.draw(|f| {
            let size = f.area();
            let block = Block::default()
                .title(format!(
                    "GitHub Repositories ({}/{})",
                    &app.repo.0, &app.repo.1
                ))
                .borders(Borders::ALL);

            let header_cells = ["#PR", "Description", "#Comments", "Mergable"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));

            let header = Row::new(header_cells)
                .style(Style::default().bg(Color::Blue))
                .height(1);

            let rows = app.items.iter().map(|item| {
                let cells = item.iter().map(|c| Cell::from(c.clone()));
                Row::new(cells).height(1)
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
                .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            f.render_stateful_widget(table, size, &mut app.state);
        })?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Down => app.next(),
                KeyCode::Up => app.previous(),
                _ => {}
            }
        }
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
