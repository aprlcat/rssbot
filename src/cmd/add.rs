use std::sync::Arc;

use anyhow::Result;
use serenity::{
    all::{
        CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
        EditInteractionResponse, Permissions,
    },
    prelude::*,
};
use tokio::time::{Duration, timeout};
use url::Url;

use crate::{
    data::Database,
    util::{parser::parse, webhook::create},
};

pub async fn execute(
    ctx: &Context,
    command: &CommandInteraction,
    database: &Arc<Database>,
) -> Result<()> {
    if !check_permissions(ctx, command).await? {
        return Ok(());
    }

    let url = extract_url(command)?;
    let channel = extract_channel(command);

    if !validate_url(&url) {
        return respond_error(command, &ctx.http, "Invalid URL format.").await;
    }

    let guild_id = command.guild_id.unwrap().get();
    let channel_id = channel.get();

    if database.duplicate(guild_id, channel_id, &url).await? {
        return respond_error(
            command,
            &ctx.http,
            &format!("This feed is already added to <#{}>.", channel_id),
        )
        .await;
    }

    defer_response(command, &ctx.http).await?;
    process_feed(ctx, command, database, &url, guild_id, channel_id).await
}

async fn check_permissions(ctx: &Context, command: &CommandInteraction) -> Result<bool> {
    if let Some(guild_id) = command.guild_id {
        if let Ok(member) = guild_id.member(&ctx.http, command.user.id).await {
            #[allow(deprecated)]
            let permissions = member.permissions(&ctx.cache)?;
            if !permissions.contains(Permissions::MANAGE_GUILD) {
                let response = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("You need the **Manage Server** permission to add RSS feeds.")
                        .ephemeral(true),
                );
                command.create_response(&ctx.http, response).await?;
                return Ok(false);
            }
        } else {
            let response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("Unable to verify your permissions.")
                    .ephemeral(true),
            );
            command.create_response(&ctx.http, response).await?;
            return Ok(false);
        }
    }
    Ok(true)
}

fn extract_url(command: &CommandInteraction) -> Result<String> {
    command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "url")
        .and_then(|opt| opt.value.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("URL is required"))
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

fn validate_url(url: &str) -> bool {
    Url::parse(url).is_ok()
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

async fn process_feed(
    ctx: &Context,
    command: &CommandInteraction,
    database: &Arc<Database>,
    url: &str,
    guild_id: u64,
    channel_id: u64,
) -> Result<()> {
    let validation_result = timeout(Duration::from_secs(15), validate_feed(url)).await;

    match validation_result {
        Ok(Ok((feed, content_size))) => {
            handle_valid_feed(
                ctx,
                command,
                database,
                url,
                guild_id,
                channel_id,
                feed,
                content_size,
            )
            .await
        }
        Ok(Err(e)) => {
            let edit_response = EditInteractionResponse::new()
                .content(format!("Failed to validate RSS feed: {}", e));
            command.edit_response(&ctx.http, edit_response).await?;
            Ok(())
        }
        Err(_) => {
            let edit_response = EditInteractionResponse::new().content(
                "Feed validation timed out (15s limit). The feed might be too large or slow to \
                 respond.",
            );
            command.edit_response(&ctx.http, edit_response).await?;
            Ok(())
        }
    }
}

async fn validate_feed(url: &str) -> Result<(feed_rs::model::Feed, usize)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 RSS Bot")
        .build()?;

    let head_response = client.head(url).send().await;
    if head_response.is_err() {
        return Err(anyhow::anyhow!("Unable to reach the URL"));
    }

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", response.status()));
    }

    if let Some(content_length) = response.content_length() {
        if content_length > 5_000_000 {
            return Err(anyhow::anyhow!(
                "Feed is too large ({} bytes). Please use a smaller feed.",
                content_length
            ));
        }
    }

    let content = response.text().await?;
    if content.len() > 5_000_000 {
        return Err(anyhow::anyhow!(
            "Feed content is too large. Please use a smaller feed."
        ));
    }

    let parsed_feed = parse(&content)?;

    if parsed_feed.entries.len() > 500 {
        return Err(anyhow::anyhow!(
            "Feed has {} items, which is too many. Please use a feed with fewer items.",
            parsed_feed.entries.len()
        ));
    }

    Ok((parsed_feed, content.len()))
}

async fn handle_valid_feed(
    ctx: &Context,
    command: &CommandInteraction,
    database: &Arc<Database>,
    url: &str,
    guild_id: u64,
    channel_id: u64,
    feed: feed_rs::model::Feed,
    content_size: usize,
) -> Result<()> {
    let webhook_future = create(
        &ctx.http,
        channel_id,
        &feed
            .title
            .as_ref()
            .map(|t| t.content.as_str())
            .unwrap_or("RSS Feed"),
        url,
    );

    match webhook_future.await {
        Ok(webhook_url) => {
            database
                .add(
                    guild_id,
                    channel_id,
                    url,
                    feed.title.as_ref().map(|t| t.content.as_str()),
                    Some(&webhook_url),
                )
                .await?;

            let feed_title = feed
                .title
                .as_ref()
                .map(|t| t.content.as_str())
                .unwrap_or("RSS Feed");
            let item_count = feed.entries.len();

            let edit_response = EditInteractionResponse::new().content(format!(
                "Successfully added **{}** to <#{}>\n{} items • {:.1}KB",
                feed_title,
                channel_id,
                item_count,
                content_size as f64 / 1024.0
            ));
            command.edit_response(&ctx.http, edit_response).await?;
        }
        Err(webhook_error) => {
            let edit_response = EditInteractionResponse::new().content(format!(
                "Feed is valid but webhook creation failed: {}\nTry again or check channel \
                 permissions.",
                webhook_error
            ));
            command.edit_response(&ctx.http, edit_response).await?;
        }
    }
    Ok(())
}
