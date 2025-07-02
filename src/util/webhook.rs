use std::sync::Arc;

use anyhow::Result;
use serenity::{
    all::{CreateAttachment, CreateWebhook, Http},
    model::id::ChannelId,
};
use tracing::debug;

pub async fn create(
    http: &Arc<Http>,
    channel_id: u64,
    name: &str,
    _feed_url: &str,
) -> Result<String> {
    let channel = ChannelId::new(channel_id);

    let avatar_bytes = match std::fs::read("assets/pfp.png") {
        Ok(bytes) => {
            debug!(
                "Successfully loaded static avatar, size: {} bytes",
                bytes.len()
            );
            Some(bytes)
        }
        Err(e) => {
            debug!("Failed to load static avatar: {}", e);
            None
        }
    };

    let mut webhook_builder = CreateWebhook::new(name);

    if let Some(avatar_data) = avatar_bytes {
        let attachment = CreateAttachment::bytes(avatar_data, "avatar.png");
        webhook_builder = webhook_builder.avatar(&attachment);
        debug!("Set webhook avatar to static pfp.png");
    }

    let webhook = channel.create_webhook(http, webhook_builder).await?;
    debug!("Created webhook successfully");
    Ok(webhook.url()?)
}
