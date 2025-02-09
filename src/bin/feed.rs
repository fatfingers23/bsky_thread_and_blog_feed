use atrium_api::app::bsky::feed::defs::PostView;
use atrium_api::app::bsky::feed::get_posts::{Error, Output};
use atrium_api::client::AtpServiceClient;
use atrium_api::types::{LimitedNonZeroU8, Unknown};
use atrium_xrpc_client::reqwest::ReqwestClient;
use bsky_thread_and_blog_feed::db::{get_posts_count, initialize_db, load_feed_from_db};
use bsky_thread_and_blog_feed::does_the_post_belong_to_the_feed;
use bsky_thread_and_blog_feed::models::{PostScoring, TextInPost};
use chrono::Utc;
use dotenv::dotenv;
use ipld_core::ipld::Ipld;
use log::{error, info};
use skyfeed::{Did, Embed, Feed, FeedHandler, FeedResult, MediaEmbed, Post, Request, Uri};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_rusqlite::{Connection, params};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let db = Connection::open("./feed.db").await?;
    initialize_db(&db).await;
    let client = AtpServiceClient::new(ReqwestClient::new("https://public.api.bsky.app"));
    let publisher_did = std::env::var("PUBLISHER_DID").expect("PUBLISHER_DID not set");
    let mut feed = MyFeed {
        handler: MyFeedHandler {
            db: db.clone(),
            bsky_client: Arc::new(Mutex::new(client)),
            feed_author_did: publisher_did,
        },
    };

    let mut cleanup_interval = tokio::time::interval(Duration::from_secs(10));
    let cleanup_task = tokio::spawn(async move {
        loop {
            cleanup_interval.tick().await;
            cleanup_posts(&db).await;
        }
    });

    tokio::join!(
        feed.start("TechThreadsAndMore", ([0, 0, 0, 0], 3030)),
        cleanup_task
    )
    .1
    .expect("Starting tasks failed")
}

struct MyFeed {
    handler: MyFeedHandler,
}

impl Feed<MyFeedHandler> for MyFeed {
    fn handler(&mut self) -> MyFeedHandler {
        self.handler.clone()
    }
}

#[derive(Clone)]
struct MyFeedHandler {
    db: Connection,
    bsky_client: Arc<Mutex<AtpServiceClient<ReqwestClient>>>,
    feed_author_did: String,
}

impl FeedHandler for MyFeedHandler {
    async fn insert_post(&mut self, post: Post) {
        //Extracting all the Text from the post
        let mut text_types: Vec<TextInPost> = vec![TextInPost::Post(post.text.clone())];
        match &post.embed {
            None => {}
            Some(embed) => match embed {
                Embed::Video(video) => {
                    text_types.push(TextInPost::Video(video.alt_text.clone()));
                }
                Embed::External(external) => {
                    text_types.push(TextInPost::External(external.title.clone()));
                    text_types.push(TextInPost::External(external.description.clone()));
                }
                Embed::Quote(_) => {}
                Embed::QuoteWithMedia(_, media_embedded) => match media_embedded {
                    MediaEmbed::Images(images) => {
                        for image in images {
                            text_types.push(TextInPost::Picture(image.alt_text.clone()));
                        }
                    }
                    MediaEmbed::Video(_) => {}
                    MediaEmbed::External(_) => {}
                },
                Embed::Images(images) => {
                    for image in images {
                        text_types.push(TextInPost::Picture(image.alt_text.clone()));
                    }
                }
            },
        }

        let save_post = does_the_post_belong_to_the_feed(text_types.clone());
        match save_post {
            None => {}
            Some(scoring) => {
                info!("Storing {post:?}");
                let _ = self.db.call(move |db| {
                    db.execute(
                        "INSERT OR REPLACE INTO posts (uri, text, pinned, deleted, priority, timestamp) VALUES (?1, ?2, 0, 0, ?3, ?4)",
                        params![ &post.uri.0, &post.text, scoring.priority, &post.timestamp.timestamp()],
                    ).map_err(|err| err.into())
                }).await;
            }
        }
    }

    async fn delete_post(&mut self, uri: Uri) {
        self.db
            .call(move |db| {
                db.execute("DELETE FROM posts WHERE uri = ?1", params![&uri.0])
                    .map_err(|err| err.into())
            })
            .await
            .unwrap();
    }

    async fn like_post(&mut self, like_uri: Uri, liked_post_uri: Uri, user_who_liked: Did) {
        if user_who_liked.0 == self.feed_author_did {
            info!("Hey you just liked something");

            let client = self.bsky_client.lock().await;
            let get_posts_call = client
                .service
                .app
                .bsky
                .feed
                .get_posts(
                    atrium_api::app::bsky::feed::get_posts::ParametersData {
                        uris: vec![liked_post_uri.0.clone()],
                    }
                    .into(),
                )
                .await;

            match get_posts_call {
                Ok(result) => {
                    for post in result.posts.clone() {
                        let post_text: String = match &post.record {
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

                        let scoring = does_the_post_belong_to_the_feed(vec![TextInPost::Post(
                            post_text.clone(),
                        )]);

                        //TODO DRY
                        match scoring {
                            None => {}
                            Some(score) => {
                                let dt = Utc::now();
                                let timestamp: i64 = dt.timestamp();
                                self.db
                                    .call(move |db| {
                                        db.execute(
                                            "INSERT OR REPLACE INTO posts (uri, text, pinned, deleted, priority, timestamp) VALUES (?1, ?2, 0, 0, ?3, ?4)",
                                            params![ &post.uri, &post_text, score.priority, &timestamp],
                                        ).map_err(|err| err.into())
                                    })
                                    .await
                                    .unwrap();
                            }
                        }
                    }
                }
                Err(err) => {
                    error!("{}", err);
                }
            }
        }

        self.db
            .call(move |db| {
                db.execute(
                    "INSERT INTO likes (post_uri, like_uri)
             SELECT ?1, ?2
             WHERE EXISTS (SELECT 1 FROM posts WHERE uri = ?1)",
                    params![&liked_post_uri.0, &like_uri.0],
                )
                .map_err(|err| err.into())
            })
            .await
            .unwrap();
    }

    async fn delete_like(&mut self, like_uri: Uri) {
        self.db
            .call(move |db| {
                db.execute("DELETE FROM likes WHERE like_uri = ?1", params![
                    &like_uri.0
                ])
                .map_err(|err| err.into())
            })
            .await
            .unwrap();
    }

    async fn serve_feed(&self, request: Request) -> FeedResult {
        // http://0.0.0.0:3030/xrpc/app.bsky.feed.getFeedSkeleton?feed=at://did:plc:rnpkyqnmsw4ipey6eotbdnnf/app.bsky.feed.generator/TechThreadsAndMore&limit=5
        info!("Serving {request:?}");
        let posts_per_page: u8 = match request.limit {
            None => 0,
            Some(limit) => u8::from(limit),
        };

        let start_index = request
            .cursor
            .as_deref()
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(0);

        let post_uris =
            load_feed_from_db(&self.db, posts_per_page as u64, start_index as u64).await;
        let mut posts: Vec<Uri> = post_uris.into_iter().map(|post| Uri(post.uri)).collect();
        //TODO prepane the pinned post? Manually? idk

        let total_posts: u64 = get_posts_count(&self.db).await;
        let next_cursor = if (start_index as u64) + (posts_per_page as u64) < total_posts {
            Some(((start_index as u64) + (posts_per_page as u64)).to_string())
        } else {
            None
        };
        info!("Served {} posts", posts.len());
        FeedResult {
            cursor: next_cursor,
            feed: posts,
        }
    }
}

async fn cleanup_posts(db: &Connection) {
    const MAX_POSTS: usize = 10_000;
    let count = db
        .call(|db| {
            db.execute(
                &format!(
                    "
                DELETE FROM posts
                WHERE uri NOT IN (
                    SELECT uri
                    FROM posts
                    ORDER BY timestamp DESC
                    LIMIT {MAX_POSTS}
                );
                "
                ),
                [],
            )
            .map_err(|err| err.into())
        })
        .await;
    match count {
        Ok(cleaned_posts) => {
            info!("Cleaned up {cleaned_posts} posts");
        }
        Err(err) => {
            info!("Failed to cleanup posts: {err:?}");
        }
    }
}
