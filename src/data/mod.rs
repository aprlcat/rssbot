pub mod models;

use anyhow::Result;
use deadpool_postgres::Pool;
use models::Feed;
use tokio_postgres::{Config, NoTls};
use tracing::{error, info};

pub struct Database {
    pool: Pool,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self> {
        let config = database_url.parse::<Config>()?;
        let mgr_config = deadpool_postgres::ManagerConfig {
            recycling_method: deadpool_postgres::RecyclingMethod::Fast,
        };
        let mgr = deadpool_postgres::Manager::from_config(config, NoTls, mgr_config);
        let pool = Pool::builder(mgr).build()?;
        let client = pool.get().await?;

        client
            .execute(
                "CREATE TABLE IF NOT EXISTS feeds (
                id BIGSERIAL PRIMARY KEY,
                guild_id BIGINT NOT NULL,
                channel_id BIGINT NOT NULL,
                url TEXT NOT NULL,
                title TEXT,
                webhook_url TEXT,
                last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                last_item_date TIMESTAMPTZ,
                UNIQUE(guild_id, channel_id, url)
            )",
                &[],
            )
            .await?;

        client
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_feeds_guild_id ON feeds(guild_id)",
                &[],
            )
            .await?;

        client
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_feeds_url ON feeds(url)",
                &[],
            )
            .await?;

        client
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_feeds_guild_channel ON feeds(guild_id, channel_id)",
                &[],
            )
            .await?;

        info!("Database initialized successfully");
        Ok(Self { pool })
    }

    pub async fn add(
        &self,
        guild_id: u64,
        channel_id: u64,
        url: &str,
        title: Option<&str>,
        webhook_url: Option<&str>,
    ) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "INSERT INTO feeds (guild_id, channel_id, url, title, webhook_url) VALUES ($1, \
                 $2, $3, $4, $5)",
                &[
                    &(guild_id as i64),
                    &(channel_id as i64),
                    &url,
                    &title,
                    &webhook_url,
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn remove(&self, guild_id: u64, url: &str) -> Result<bool> {
        let client = self.pool.get().await?;
        let result = client
            .execute(
                "DELETE FROM feeds WHERE guild_id = $1 AND url = $2",
                &[&(guild_id as i64), &url],
            )
            .await?;
        Ok(result > 0)
    }

    pub async fn guild(&self, guild_id: u64) -> Result<Vec<Feed>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, guild_id, channel_id, url, title, webhook_url, last_updated, \
                 last_item_date 
             FROM feeds WHERE guild_id = $1 ORDER BY id",
                &[&(guild_id as i64)],
            )
            .await?;

        let feeds = rows
            .into_iter()
            .map(|row| {
                let last_updated: chrono::DateTime<chrono::Utc> = row.get(6);
                let last_item_date: Option<chrono::DateTime<chrono::Utc>> = row.get(7);

                Feed {
                    id: row.get(0),
                    guild_id: row.get(1),
                    channel_id: row.get(2),
                    url: row.get(3),
                    title: row.get(4),
                    webhook_url: row.get(5),
                    last_updated: last_updated.to_rfc3339(),
                    last_item_date: last_item_date.map(|dt| dt.to_rfc3339()),
                }
            })
            .collect();

        Ok(feeds)
    }

    pub async fn feeds(&self) -> Result<Vec<Feed>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, guild_id, channel_id, url, title, webhook_url, last_updated, \
                 last_item_date 
             FROM feeds ORDER BY id",
                &[],
            )
            .await?;

        let feeds = rows
            .into_iter()
            .map(|row| {
                let last_updated: chrono::DateTime<chrono::Utc> = row.get(6);
                let last_item_date: Option<chrono::DateTime<chrono::Utc>> = row.get(7);

                Feed {
                    id: row.get(0),
                    guild_id: row.get(1),
                    channel_id: row.get(2),
                    url: row.get(3),
                    title: row.get(4),
                    webhook_url: row.get(5),
                    last_updated: last_updated.to_rfc3339(),
                    last_item_date: last_item_date.map(|dt| dt.to_rfc3339()),
                }
            })
            .collect();

        Ok(feeds)
    }

    pub async fn find(&self, url: &str) -> Result<Option<Feed>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, guild_id, channel_id, url, title, webhook_url, last_updated, \
                 last_item_date 
             FROM feeds WHERE url = $1 LIMIT 1",
                &[&url],
            )
            .await?;

        if let Some(row) = rows.first() {
            let last_updated: chrono::DateTime<chrono::Utc> = row.get(6);
            let last_item_date: Option<chrono::DateTime<chrono::Utc>> = row.get(7);

            Ok(Some(Feed {
                id: row.get(0),
                guild_id: row.get(1),
                channel_id: row.get(2),
                url: row.get(3),
                title: row.get(4),
                webhook_url: row.get(5),
                last_updated: last_updated.to_rfc3339(),
                last_item_date: last_item_date.map(|dt| dt.to_rfc3339()),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn update(&self, id: i64, last_item_date: Option<&str>) -> Result<()> {
        let client = self.pool.get().await?;

        let last_item_dt = if let Some(date_str) = last_item_date {
            match chrono::DateTime::parse_from_rfc3339(date_str) {
                Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
                Err(e) => {
                    error!("Failed to parse date {}: {}", date_str, e);
                    None
                }
            }
        } else {
            None
        };

        client
            .execute(
                "UPDATE feeds SET last_updated = NOW(), last_item_date = $1 WHERE id = $2",
                &[&last_item_dt, &id],
            )
            .await?;
        Ok(())
    }

    pub async fn exists(&self, guild_id: u64, url: &str) -> Result<bool> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT COUNT(*) FROM feeds WHERE guild_id = $1 AND url = $2",
                &[&(guild_id as i64), &url],
            )
            .await?;

        let count: i64 = rows[0].get(0);
        Ok(count > 0)
    }

    pub async fn duplicate(&self, guild_id: u64, channel_id: u64, url: &str) -> Result<bool> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT COUNT(*) FROM feeds WHERE guild_id = $1 AND channel_id = $2 AND url = $3",
                &[&(guild_id as i64), &(channel_id as i64), &url],
            )
            .await?;

        let count: i64 = rows[0].get(0);
        Ok(count > 0)
    }
}
