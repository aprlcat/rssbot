CREATE TABLE IF NOT EXISTS feeds (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    guild_id INTEGER NOT NULL,
    channel_id INTEGER NOT NULL,
    url TEXT NOT NULL,
    title TEXT,
    webhook_url TEXT,
    last_updated TEXT NOT NULL,
    last_item_date TEXT,
    UNIQUE(guild_id, url)
);

CREATE TABLE IF NOT EXISTS guild_settings (
    guild_id INTEGER PRIMARY KEY NOT NULL,
    rss_channel_id INTEGER NOT NULL
);