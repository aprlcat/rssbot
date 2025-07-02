use std::sync::Arc;

use anyhow::Result;
use serenity::{
    all::{
        CommandInteraction, CreateEmbed, CreateInteractionResponse,
        CreateInteractionResponseMessage,
    },
    prelude::*,
};

use crate::{
    data::Database,
    util::{fetcher::fetch, parser::parse},
};

pub async fn execute(
    ctx: &Context,
    command: &CommandInteraction,
    database: &Arc<Database>,
) -> Result<()> {
    let guild_id = command.guild_id.unwrap().get();
    let feeds = database.guild(guild_id).await?;

    if feeds.is_empty() {
        return respond_empty(command, &ctx.http).await;
    }

    defer_response(command, &ctx.http).await?;

    let description = build_description(&feeds).await;
    let embed = CreateEmbed::new()
        .title("RSS Feeds")
        .description(description)
        .color(0x7289da);

    let edit_response = serenity::all::EditInteractionResponse::new().embed(embed);
    command.edit_response(&ctx.http, edit_response).await?;
    Ok(())
}

async fn respond_empty(command: &CommandInteraction, http: &serenity::http::Http) -> Result<()> {
    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content("No RSS feeds configured for this server.")
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

async fn build_description(feeds: &[crate::data::models::Feed]) -> String {
    let mut description = String::new();
    let total_feeds = feeds.len();

    for (i, feed) in feeds.iter().enumerate() {
        let channel_mention = format!("<#{}>", feed.channel_id);
        let domain = extract_domain(&feed.url);
        let last_updated = get_last_updated(&feed.url).await;

        description.push_str(&format!(
            "{}. `{}` → {} | Last updated: {}\n",
            i + 1,
            domain,
            channel_mention,
            last_updated
        ));
    }

    description.push_str(&format!("\nTotal: {} feeds configured", total_feeds));
    description
}

fn extract_domain(url: &str) -> String {
    if let Ok(parsed_url) = url::Url::parse(url) {
        parsed_url.host_str().unwrap_or("Unknown").to_string()
    } else {
        "Unknown".to_string()
    }
}

async fn get_last_updated(url: &str) -> String {
    match tokio::time::timeout(std::time::Duration::from_secs(5), fetch(url)).await {
        Ok(Ok(content)) => match parse(&content) {
            Ok(parsed_feed) => parsed_feed
                .entries
                .first()
                .and_then(|entry| entry.published.or(entry.updated))
                .map(|date| date.format("%b %d, %Y").to_string())
                .unwrap_or_else(|| "No recent items".to_string()),
            Err(_) => "Unable to parse".to_string(),
        },
        _ => "Feed unavailable".to_string(),
    }
}
