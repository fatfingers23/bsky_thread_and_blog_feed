use atrium_api::app::bsky::feed::defs::{PostView, PostViewEmbedRefs};
use atrium_api::client::AtpServiceClient;
use atrium_api::types::{Union, Unknown};
use atrium_xrpc_client::reqwest::ReqwestClient;
use bsky_thread_and_blog_feed::db::{delete_post, load_feed_from_db};
use color_eyre::Result;
use ipld_core::ipld::Ipld;
use log::info;
use ratatui::style::palette::tailwind;
use ratatui::text::Text;
use ratatui::widgets::Cell;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{Event, EventStream, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, HighlightSpacing, Row, StatefulWidget, Table, TableState, Widget},
    DefaultTerminal, Frame,
};
use skyfeed::Uri;
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::Mutex;
use tokio_rusqlite::Connection;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let client = AtpServiceClient::new(ReqwestClient::new("https://public.api.bsky.app"));
    let connection = Connection::open("./feed.db").await?;

    let app = App {
        should_quit: false,
        feed_display: FeedDisplayWidget {
            state: Arc::new(RwLock::new(FeedPostState::default())),
            db: connection,
            bsky_client: Arc::new(Mutex::new(client)),
            feed_offset: 0,
            feed_limit: 25,
        },
    };
    let app_result = app.run(terminal).await;
    ratatui::restore();
    app_result
}

// #[derive(Debug)]
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
                Some(Ok(event)) = events.next() => self.handle_event(&event).await,
            }
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        let vertical = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]);
        let [title_area, body_area] = vertical.areas(frame.area());
        let title = Line::from("🧵 and Blog Feed Admin").centered().bold();
        frame.render_widget(title, title_area);
        frame.render_widget(&self.feed_display, body_area);
    }

    async fn handle_event(&mut self, event: &Event) {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.feed_display.clone().scroll_down().await
                    }
                    KeyCode::Char('k') | KeyCode::Up => self.feed_display.scroll_up(),
                    KeyCode::Char('r') => self.feed_display.clone().fetch_posts().await,
                    KeyCode::Enter => self.feed_display.open_post(),
                    KeyCode::Char('d') | KeyCode::Delete => {
                        self.feed_display.clone().delete_post().await;
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Clone)]
struct FeedDisplayWidget {
    state: Arc<RwLock<FeedPostState>>,
    db: Connection,
    bsky_client: Arc<Mutex<AtpServiceClient<ReqwestClient>>>,
    feed_offset: u64,
    feed_limit: u64,
}

#[derive(Debug, Default)]
struct FeedPostState {
    posts: Vec<PostView>,
    loading_state: LoadingState,
    table_state: TableState,
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
        tokio::task::spawn_blocking(move || {
            tokio::runtime::Handle::current().block_on(this.fetch_posts())
        });
    }

    pub async fn fetch_posts(self) {
        self.set_loading_state(LoadingState::Loading);

        let posts = load_feed_from_db(&self.db, self.feed_limit, self.feed_offset).await;

        let posts_uris: Vec<String> = posts.iter().map(|post| post.uri.clone()).collect();

        let client = self.bsky_client.lock().await;
        //TODO need pagination can only get 25 at a time
        let get_posts_call = client
            .service
            .app
            .bsky
            .feed
            .get_posts(
                atrium_api::app::bsky::feed::get_posts::ParametersData {
                    uris: posts_uris.clone(),
                }
                .into(),
            )
            .await;

        match get_posts_call {
            Ok(result) => self.on_load(result.posts.clone()),
            Err(error) => {
                self.on_err(error.to_string());
                return;
            }
        }
        self.set_loading_state(LoadingState::Loaded);
        info!("Loaded {} posts from the feed", posts.len());
    }
    fn on_load(&self, posts: Vec<PostView>) {
        let mut state = self.state.write().unwrap();
        state.loading_state = LoadingState::Loaded;
        state.posts = posts;
        if !state.posts.is_empty() {
            state.table_state.select(Some(0));
        }
    }

    fn on_err(&self, error_message: String) {
        self.set_loading_state(LoadingState::Error(error_message));
    }

    fn set_loading_state(&self, state: LoadingState) {
        self.state.write().unwrap().loading_state = state;
    }

    pub async fn scroll_down(&mut self) {
        self.state.write().unwrap().table_state.scroll_down_by(1);
        let last_post_index = self.state.read().unwrap().posts.len() - 1;
        let current = self.state.read().unwrap().table_state.selected().unwrap() - 1;
        if current >= last_post_index {
            self.feed_offset += self.feed_limit;
            self.clone().fetch_posts().await;
        }
    }

    fn scroll_up(&self) {
        self.state.write().unwrap().table_state.scroll_up_by(1);
    }

    pub fn open_post(&self) {
        let selected = self.state.write().unwrap().table_state.selected();
        match selected {
            None => {}
            Some(selected) => {
                let selected_post_view = self.state.read().unwrap().posts[selected].clone();
                let post_uri = selected_post_view.uri.clone();
                let _ = webbrowser::open(format!("https://atp.tools/{post_uri}").as_str()).is_ok();
                println!("{:?}", selected);
            }
        }
    }

    async fn delete_post(&self) {
        let selected = self.state.write().unwrap().table_state.selected();
        match selected {
            None => {}
            Some(selected) => {
                let selected_post_view = self.state.read().unwrap().posts[selected].clone();
                let post_uri = selected_post_view.uri.clone();
                delete_post(&self.db, post_uri).await;
            }
        }
        // self.fetch_posts().await;
    }
}

impl Widget for &FeedDisplayWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = self.state.write().unwrap();

        // a block with a right aligned title with the loading state on the right
        let loading_state = Line::from(format!("{:?}", state.loading_state)).right_aligned();
        let block = Block::bordered()
            .title("Posts currently showing in the feed")
            .title(loading_state)
            .title_bottom("j/k to scroll | r to refresh | d to delete | q to quit");

        let post_content = state.posts.iter().enumerate().map(|(i, post_view)| {
            let post_text: String = match &post_view.record {
                Unknown::Object(map) => match map.get("text") {
                    Some(data_model) => match &**data_model {
                        Ipld::String(text) => text.clone(),
                        Ipld::Null => "(Null content)".to_string(),
                        other => format!("(Unexpected format: {:?})", other),
                    },
                    None => "(No text content)".to_string(),
                },
                Unknown::Null => "No post content".to_string(),
                Unknown::Other(_) => "Other?".to_string(),
            };
            let one_liner = &post_text.replace("\n", "\\n");
            let author = post_view.author.handle.clone();
            let author_string = author.as_str();
            let likes = post_view.like_count.unwrap_or(0);

            // let mut pinned = "📌";
            // if i > 0 {
            //     pinned = "";
            // }

            let media_type = match post_view.embed.clone() {
                None => "🗒️",
                Some(embedded_ref) => match embedded_ref {
                    Union::Refs(refs) => match refs {
                        PostViewEmbedRefs::AppBskyEmbedImagesView(_) => "📸",
                        PostViewEmbedRefs::AppBskyEmbedVideoView(_) => "📹",
                        PostViewEmbedRefs::AppBskyEmbedExternalView(_) => "🌐",
                        PostViewEmbedRefs::AppBskyEmbedRecordView(_) => "Record",
                        PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(_) => {
                            "Record with media"
                        }
                    },
                    Union::Unknown(_) => "",
                },
            };
            let color = match i % 2 {
                0 => tailwind::SLATE.c800,
                _ => tailwind::SLATE.c900,
            };
            let url = format!("https://atp.tools/{}", post_view.uri);
            [Cell::from(Text::from(format!(
                "{one_liner}\n@{author_string} | {likes} likes | {media_type}\n{url}"
            )))]
            .into_iter()
            .collect::<Row>()
            .style(Style::new().bg(color))
            .height(4)
        });

        let widths = [
            Constraint::Length(1000),
            Constraint::Fill(1),
            Constraint::Max(49),
        ];
        let table = Table::new(
            // post_content.into_iter().map(|text| Row::new(vec![text])),
            post_content,
            widths,
        )
        .block(block)
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_symbol(">>")
        .row_highlight_style(Style::new().on_blue());

        StatefulWidget::render(table, area, buf, &mut state.table_state);
    }
}
