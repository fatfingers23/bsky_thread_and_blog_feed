use crate::models::DbPost;
use anyhow::Result;
use crossterm::ExecutableCommand;
use log::info;
use tokio_rusqlite::Connection;

pub async fn load_feed_from_db(db: &Connection, limit: u64, offset: u64) -> Vec<DbPost> {
    //TODO just move to order by timestamp
    //BUT do a pull on pinned first or above x scoring and put them first?
    //May long get away with timestamp. Getting too wild
    db.call(move |db| {
        let mut stmt = db
            .prepare(
                "
               SELECT
                    posts.uri,
                    posts.text,
                    posts.pinned,
                    main.posts.deleted,
                    posts.priority

                FROM posts
                where posts.deleted = 0
                GROUP BY posts.uri, posts.text, posts.pinned, posts.deleted, posts.priority
                ORDER BY  posts.timestamp desc
               LIMIT ?1 OFFSET ?2
                 ",
            )
            .expect("Failed to prepare statement");
        Ok(stmt
            .query_map([&limit.clone(), &offset.clone()], |row| {
                Ok(DbPost {
                    uri: row.get(0)?,
                    text: row.get(1)?,
                    pinned: row.get(2)?,
                    deleted: row.get(3)?,
                    priority: row.get(4)?,
                    // timestamp: DateTime::<Utc>::now
                    // timestamp: Utc.timestamp(row.get(5)?, 0),
                })
            })?
            .collect::<Result<Vec<DbPost>, _>>()?)
    })
    .await
    .unwrap()
}

pub async fn get_posts_count(db: &Connection) -> u64 {
    let count = db
        .call(|db| {
            db.query_row("SELECT COUNT(uri) FROM posts", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(|err| err.into())
        })
        .await
        .expect("Failed to get posts count");
    count
}

pub async fn delete_post(db: &Connection, uri: String) {
    let _ = db
        .call(move |db| {
            db.execute("DELETE FROM likes WHERE post_uri = ?1", &[&uri])
                .unwrap();

            db.execute("DELETE FROM posts WHERE uri = ?1", &[&uri])
                .map_err(|err| err.into())
        })
        .await
        .expect("Failed to delete post");
}

pub async fn initialize_db(db: &Connection) {
    let _ = db
        .call(|db| {
            db.execute(
                "CREATE TABLE IF NOT EXISTS posts (
            uri TEXT PRIMARY KEY,
            text TEXT,
            pinned INTEGER,
            deleted INTEGER,
            priority INTEGER,
            timestamp INTEGER
        )",
                [],
            )
            .expect("Failed to create posts table");

            db.execute(
                "CREATE TABLE IF NOT EXISTS likes (
            post_uri TEXT,
            like_uri TEXT,
            PRIMARY KEY (post_uri, like_uri),
            FOREIGN KEY (post_uri) REFERENCES posts(uri) ON DELETE CASCADE
        )",
                [],
            )
            .expect("Failed to create likes table");

            db.execute(
                "CREATE INDEX IF NOT EXISTS idx_likes_post_uri ON likes(post_uri)",
                [],
            )
            .map_err(|err| err.into())
        })
        .await
        .expect("Failed to initialize database");
}
