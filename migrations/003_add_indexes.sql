CREATE INDEX IF NOT EXISTS idx_feeds_guild_id ON feeds(guild_id);
CREATE INDEX IF NOT EXISTS idx_feeds_url ON feeds(url);
CREATE INDEX IF NOT EXISTS idx_feeds_last_updated ON feeds(last_updated);
CREATE INDEX IF NOT EXISTS idx_feeds_guild_channel ON feeds(guild_id, channel_id);