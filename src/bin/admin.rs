//! # [Ratatui] Async example
//!
//! This example demonstrates how to use Ratatui with widgets that fetch data asynchronously. It
//! uses the `octocrab` crate to fetch a list of pull requests from the GitHub API.
//!
//! <https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#creating-a-fine-grained-personal-access-token>
//! <https://github.com/settings/tokens/new> to create a new token (select classic, and no scopes)
//!
//! This example does not cover message passing between threads, it only demonstrates how to manage
//! shared state between the main thread and a background task, which acts mostly as a one-shot
//! fetcher. For more complex scenarios, you may need to use channels or other synchronization
//! primitives.
//!
//! A simple app might have multiple widgets that fetch data from different sources, and each widget
//! would have its own background task to fetch the data. The main thread would then render the
//! widgets with the latest data.
//!
//! The latest version of this example is available in the [examples] folder in the repository.
//!
//! Please note that the examples are designed to be run against the `main` branch of the Github
//! repository. This means that you may not be able to compile with the latest release version on
//! crates.io, or the one that you have installed locally.
//!
//! See the [examples readme] for more information on finding examples that match the version of the
//! library you are using.
//!
//! [Ratatui]: https://github.com/ratatui/ratatui
//! [examples]: https://github.com/ratatui/ratatui/blob/main/examples
//! [examples readme]: https://github.com/ratatui/ratatui/blob/main/examples/README.md
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use color_eyre::Result;
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    crossterm::event::{Event, EventStream, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::Line,
    widgets::{Block, HighlightSpacing, Row, StatefulWidget, Table, TableState, Widget},
};
use rusqlite::Connection;
use skyfeed::Uri;
use tokio::sync::Mutex;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let app = App {
        should_quit: false,
        feed_display: FeedDisplayWidget {
            state: Arc::new(RwLock::new(PullRequestListState::default())),
            db: Arc::new(Mutex::new(
                Connection::open("./feed.db").expect("Failed to open database"),
            )),
        },
    };
    let app_result = app.run(terminal).await;
    ratatui::restore();
    app_result
}

#[derive(Debug)]
struct App {
    should_quit: bool,
    feed_display: FeedDisplayWidget,
}

impl App {
    const FRAMES_PER_SECOND: f32 = 60.0;

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.feed_display.run();

        let period = Duration::from_secs_f32(1.0 / Self::FRAMES_PER_SECOND);
        let mut interval = tokio::time::interval(period);
        let mut events = EventStream::new();

        while !self.should_quit {
            tokio::select! {
                _ = interval.tick() => { terminal.draw(|frame| self.draw(frame))?; },
                Some(Ok(event)) = events.next() => self.handle_event(&event),
            }
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        let vertical = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]);
        let [title_area, body_area] = vertical.areas(frame.area());
        let title = Line::from("Ratatui async example").centered().bold();
        frame.render_widget(title, title_area);
        frame.render_widget(&self.feed_display, body_area);
    }

    fn handle_event(&mut self, event: &Event) {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                    KeyCode::Char('j') | KeyCode::Down => self.feed_display.scroll_down(),
                    KeyCode::Char('k') | KeyCode::Up => self.feed_display.scroll_up(),
                    _ => {}
                }
            }
        }
    }
}

/// A widget that displays a list of pull requests.
///
/// This is an async widget that fetches the list of pull requests from the GitHub API. It contains
/// an inner `Arc<RwLock<PullRequestListState>>` that holds the state of the widget. Cloning the
/// widget will clone the Arc, so you can pass it around to other threads, and this is used to spawn
/// a background task to fetch the pull requests.
#[derive(Debug, Clone)]
struct FeedDisplayWidget {
    state: Arc<RwLock<PullRequestListState>>,
    db: Arc<Mutex<Connection>>,
}

#[derive(Debug, Default)]
struct PullRequestListState {
    uris: Vec<Uri>,
    loading_state: LoadingState,
    table_state: TableState,
}

#[derive(Debug, Clone)]
struct PullRequest {
    id: String,
    title: String,
    url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum LoadingState {
    #[default]
    Idle,
    Loading,
    Loaded,
    Error(String),
}

impl FeedDisplayWidget {
    /// Start fetching the pull requests in the background.
    ///
    /// This method spawns a background task that fetches the pull requests from the GitHub API.
    /// The result of the fetch is then passed to the `on_load` or `on_err` methods.
    fn run(&self) {
        let this = self.clone(); // clone the widget to pass to the background task
        tokio::spawn(this.fetch_posts());
    }

    async fn fetch_posts(self) {
        let db = self.db.lock().await;
        let mut stmt = db
            .prepare(
                "
             WITH ranked_posts AS (
                    SELECT
                        uri,
                        timestamp,
                        COUNT(like_uri) AS likes
                    FROM posts
                    LEFT JOIN likes ON posts.uri = likes.post_uri
                    GROUP BY posts.uri
                    HAVING COUNT(like_uri) > 0
                ),
                sorted_posts AS (
                    SELECT
                        uri,
                        timestamp,
                        likes,
                        PERCENT_RANK() OVER (ORDER BY likes DESC) AS rank
                    FROM ranked_posts
                )
                SELECT uri, likes
                FROM sorted_posts
                WHERE rank <= 0.05
                ORDER BY timestamp DESC;
             ",
            )
            .expect("Failed to prepare statement");

        let post_iter = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("Failed to query posts");
        // let test = post_iter.map(|x| x.unwrap()).collect::<Vec<_>>();
        let posts: Vec<Uri> = post_iter.map(|x| x.unwrap()).map(Uri).collect();

        // this runs once, but you could also run this in a loop, using a channel that accepts
        // messages to refresh on demand, or with an interval timer to refresh every N seconds
        self.set_loading_state(LoadingState::Loading);
        // match octocrab::instance()
        //     .pulls("ratatui", "ratatui")
        //     .list()
        //     .sort(Sort::Updated)
        //     .direction(Direction::Descending)
        //     .send()
        //     .await
        // {

        self.on_load(posts)
        // Err(err) => self.on_err(&err),
        // }
    }
    fn on_load(&self, test: Vec<Uri>) {
        // let prs = page.items.iter().map(Into::into);
        let mut state = self.state.write().unwrap();
        state.loading_state = LoadingState::Loaded;
        state.uris.extend(test);
        if !state.uris.is_empty() {
            state.table_state.select(Some(0));
        }
    }

    fn on_err(&self) {
        // self.set_loading_state(LoadingState::Error(err.to_string()));
    }

    fn set_loading_state(&self, state: LoadingState) {
        self.state.write().unwrap().loading_state = state;
    }

    fn scroll_down(&self) {
        self.state.write().unwrap().table_state.scroll_down_by(1);
    }

    fn scroll_up(&self) {
        self.state.write().unwrap().table_state.scroll_up_by(1);
    }
}

// impl From<&OctoPullRequest> for PullRequest {
//     fn from(pr: &OctoPullRequest) -> Self {
//         Self {
//             id: pr.number.to_string(),
//             title: pr.title.as_ref().unwrap().to_string(),
//             url: pr
//                 .html_url
//                 .as_ref()
//                 .map(ToString::to_string)
//                 .unwrap_or_default(),
//         }
//     }
// }

impl Widget for &FeedDisplayWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = self.state.write().unwrap();

        // a block with a right aligned title with the loading state on the right
        let loading_state = Line::from(format!("{:?}", state.loading_state)).right_aligned();
        let block = Block::bordered()
            .title("Pull Requests")
            .title(loading_state)
            .title_bottom("j/k to scroll, q to quit");

        // a table with the list of pull requests
        // let rows = state.uris.iter().map(|uri| Row::new(vec![uri.]));
        let rows = state
            .uris
            .iter()
            .map(|uri| Row::new(vec![uri.0.to_string()])); // Each URI gets converted to a single-column string row.
        // println!("{:?}", rows);
        let widths = [
            Constraint::Length(5),
            Constraint::Fill(1),
            Constraint::Max(49),
        ];
        let table = Table::new(rows, widths)
            .block(block)
            .highlight_spacing(HighlightSpacing::Always)
            .highlight_symbol(">>")
            .row_highlight_style(Style::new().on_blue());

        StatefulWidget::render(table, area, buf, &mut state.table_state);
    }
}

impl From<&PullRequest> for Row<'_> {
    fn from(pr: &PullRequest) -> Self {
        let pr = pr.clone();
        Row::new(vec![pr.id, pr.title, pr.url])
    }
}
