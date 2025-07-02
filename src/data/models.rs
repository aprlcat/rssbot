use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feed {
    pub id: i64,
    pub guild_id: i64,
    pub channel_id: i64,
    pub url: String,
    pub title: Option<String>,
    pub webhook_url: Option<String>,
    pub last_updated: String,
    pub last_item_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuildSettings {
    pub guild_id: i64,
    pub rss_channel_id: i64,
}
