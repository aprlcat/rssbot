pub mod models;

use anyhow::Result;
use models::Feed;
use sqlx::SqlitePool;

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn add(
        &self,
        guild_id: u64,
        channel_id: u64,
        url: &str,
        title: Option<&str>,
        webhook_url: Option<&str>,
    ) -> Result<()> {
        let guild_id_i64 = guild_id as i64;
        let channel_id_i64 = channel_id as i64;

        sqlx::query!(
            r#"
            INSERT INTO feeds (guild_id, channel_id, url, title, webhook_url, last_updated)
            VALUES (?, ?, ?, ?, ?, datetime('now'))
            "#,
            guild_id_i64,
            channel_id_i64,
            url,
            title,
            webhook_url
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove(&self, guild_id: u64, url: &str) -> Result<bool> {
        let guild_id_i64 = guild_id as i64;

        let result = sqlx::query!(
            "DELETE FROM feeds WHERE guild_id = ? AND url = ?",
            guild_id_i64,
            url
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn guild(&self, guild_id: u64) -> Result<Vec<Feed>> {
        let guild_id_i64 = guild_id as i64;

        let rows = sqlx::query!(
            "SELECT id, guild_id, channel_id, url, title, webhook_url, last_updated, \
             last_item_date FROM feeds WHERE guild_id = ? ORDER BY id",
            guild_id_i64
        )
        .fetch_all(&self.pool)
        .await?;

        let feeds = rows
            .into_iter()
            .map(|row| Feed {
                id: row.id,
                guild_id: row.guild_id,
                channel_id: row.channel_id,
                url: row.url,
                title: row.title,
                webhook_url: row.webhook_url,
                last_updated: row.last_updated,
                last_item_date: row.last_item_date,
            })
            .collect();

        Ok(feeds)
    }

    pub async fn channel(&self, guild_id: u64, channel_id: u64) -> Result<Vec<Feed>> {
        let guild_id_i64 = guild_id as i64;
        let channel_id_i64 = channel_id as i64;

        let rows = sqlx::query!(
            "SELECT id, guild_id, channel_id, url, title, webhook_url, last_updated, \
             last_item_date FROM feeds WHERE guild_id = ? AND channel_id = ?",
            guild_id_i64,
            channel_id_i64
        )
        .fetch_all(&self.pool)
        .await?;

        let feeds = rows
            .into_iter()
            .map(|row| Feed {
                id: row.id,
                guild_id: row.guild_id,
                channel_id: row.channel_id,
                url: row.url,
                title: row.title,
                webhook_url: row.webhook_url,
                last_updated: row.last_updated,
                last_item_date: row.last_item_date,
            })
            .collect();

        Ok(feeds)
    }

    pub async fn feeds(&self) -> Result<Vec<Feed>> {
        let rows = sqlx::query!(
            "SELECT id, guild_id, channel_id, url, title, webhook_url, last_updated, \
             last_item_date FROM feeds"
        )
        .fetch_all(&self.pool)
        .await?;

        let feeds = rows
            .into_iter()
            .map(|row| Feed {
                id: row.id,
                guild_id: row.guild_id,
                channel_id: row.channel_id,
                url: row.url,
                title: row.title,
                webhook_url: row.webhook_url,
                last_updated: row.last_updated,
                last_item_date: row.last_item_date,
            })
            .collect();

        Ok(feeds)
    }

    pub async fn find(&self, url: &str) -> Result<Option<Feed>> {
        let row = sqlx::query!(
            "SELECT id, guild_id, channel_id, url, title, webhook_url, last_updated, \
             last_item_date FROM feeds WHERE url = ? LIMIT 1",
            url
        )
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            Ok(Some(Feed {
                id: row.id,
                guild_id: row.guild_id,
                channel_id: row.channel_id,
                url: row.url,
                title: row.title,
                webhook_url: row.webhook_url,
                last_updated: row.last_updated,
                last_item_date: row.last_item_date,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn update(&self, id: i64, last_item_date: Option<&str>) -> Result<()> {
        sqlx::query!(
            "UPDATE feeds SET last_updated = datetime('now'), last_item_date = ? WHERE id = ?",
            last_item_date,
            id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn exists(&self, guild_id: u64, url: &str) -> Result<bool> {
        let guild_id_i64 = guild_id as i64;

        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM feeds WHERE guild_id = ? AND url = ?",
            guild_id_i64,
            url
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn duplicate(&self, guild_id: u64, channel_id: u64, url: &str) -> Result<bool> {
        let guild_id_i64 = guild_id as i64;
        let channel_id_i64 = channel_id as i64;

        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM feeds WHERE guild_id = ? AND channel_id = ? AND url = ?",
            guild_id_i64,
            channel_id_i64,
            url
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }
}
