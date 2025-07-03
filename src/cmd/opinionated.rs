use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serenity::{
    all::{
        CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
        EditInteractionResponse,
    },
    prelude::*,
};
use tracing::{error, info};

use crate::data::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpinionatedFeed {
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpinionatedCollection {
    pub topic: String,
    pub feeds: Vec<OpinionatedFeed>,
}

pub async fn execute(
    ctx: &Context,
    command: &CommandInteraction,
    database: &Arc<Database>,
) -> Result<()> {
    let topic = extract_topic(command)?;
    let channel = extract_channel(command);
    let guild_id = command.guild_id.unwrap().get();
    let channel_id = channel.get();

    info!(
        "Processing opinionated command: topic={}, channel={}",
        topic, channel_id
    );

    let collection = match load_collection(&topic).await {
        Ok(collection) => collection,
        Err(_) => {
            error!("Topic '{}' not found in curated collections", topic);
            return respond_error(
                command,
                &ctx.http,
                &format!("Topic '{}' not found in curated collections.", topic),
            )
            .await;
        }
    };

    defer_response(command, &ctx.http).await?;

    let mut added_count = 0;
    let mut skipped_count = 0;
    let mut failed_feeds = Vec::new();

    info!(
        "Processing {} feeds from '{}' collection",
        collection.feeds.len(),
        collection.topic
    );

    for feed in &collection.feeds {
        info!("Processing feed: {}", feed.name);

        if database.exists(guild_id, &feed.url).await? {
            info!(
                "Skipping feed '{}' - already exists in this server",
                feed.name
            );
            skipped_count += 1;
            continue;
        }

        match add_feed(database, feed, guild_id, channel_id).await {
            Ok(()) => {
                info!("Successfully added feed: {}", feed.name);
                added_count += 1;
            }
            Err(e) => {
                error!("Failed to add feed '{}': {}", feed.name, e);
                let error_msg = if e.to_string().contains("UNIQUE constraint") {
                    "already exists".to_string()
                } else {
                    e.to_string()
                };
                failed_feeds.push(format!("• {} ({})", feed.name, error_msg));
            }
        }
    }

    let mut summary = format!(
        "Added {} feeds from '{}' collection to <#{}>\n• {} added\n• {} skipped (already in \
         server)",
        added_count, collection.topic, channel_id, added_count, skipped_count
    );

    if !failed_feeds.is_empty() {
        summary.push_str(&format!("\n• {} failed:", failed_feeds.len()));
        for failed in failed_feeds.iter().take(5) {
            summary.push_str(&format!("\n  {}", failed));
        }
        if failed_feeds.len() > 5 {
            summary.push_str(&format!("\n  ... and {} more", failed_feeds.len() - 5));
        }
    }

    info!(
        "Opinionated command completed: {} added, {} skipped, {} failed",
        added_count,
        skipped_count,
        failed_feeds.len()
    );

    let edit_response = EditInteractionResponse::new().content(summary);
    command.edit_response(&ctx.http, edit_response).await?;
    Ok(())
}

pub async fn topics() -> Result<Vec<String>> {
    let mut topics = Vec::new();
    let opinionated_dir = std::path::Path::new("opinionated");

    if !opinionated_dir.exists() {
        return Ok(topics);
    }

    let mut entries = tokio::fs::read_dir(opinionated_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            match load_collection_from_path(&path).await {
                Ok(collection) => {
                    topics.push(collection.topic);
                }
                Err(e) => {
                    error!("Failed to parse {}: {}, using filename", path.display(), e);
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        topics.push(stem.to_string());
                    }
                }
            }
        }
    }

    topics.sort();
    Ok(topics)
}

fn extract_topic(command: &CommandInteraction) -> Result<String> {
    command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "topic")
        .and_then(|opt| opt.value.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("Topic is required"))
}

fn extract_channel(command: &CommandInteraction) -> serenity::model::id::ChannelId {
    command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "channel")
        .and_then(|opt| opt.value.as_channel_id())
        .unwrap_or(command.channel_id)
}

pub async fn load_collection(topic: &str) -> Result<OpinionatedCollection> {
    let opinionated_dir = std::path::Path::new("opinionated");

    if !opinionated_dir.exists() {
        return Err(anyhow::anyhow!("Opinionated directory not found"));
    }

    let mut entries = tokio::fs::read_dir(opinionated_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(collection) = load_collection_from_path(&path).await {
                if collection.topic.to_lowercase() == topic.to_lowercase() {
                    return Ok(collection);
                }
            }
        }
    }

    let path = format!("opinionated/{}.json", topic.to_lowercase());
    load_collection_from_path(std::path::Path::new(&path)).await
}

async fn load_collection_from_path(path: &std::path::Path) -> Result<OpinionatedCollection> {
    let content = tokio::fs::read_to_string(path).await?;
    let collection: OpinionatedCollection = serde_json::from_str(&content)?;
    Ok(collection)
}

async fn add_feed(
    database: &Arc<Database>,
    feed: &OpinionatedFeed,
    guild_id: u64,
    channel_id: u64,
) -> Result<()> {
    database
        .add(guild_id, channel_id, &feed.url, Some(&feed.name), None)
        .await?;

    Ok(())
}

async fn respond_error(
    command: &CommandInteraction,
    http: &serenity::http::Http,
    message: &str,
) -> Result<()> {
    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(message)
            .ephemeral(true),
    );
    command.create_response(http, response).await?;
    Ok(())
}

async fn defer_response(command: &CommandInteraction, http: &serenity::http::Http) -> Result<()> {
    let response =
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true));
    command.create_response(http, response).await?;
    Ok(())
}
