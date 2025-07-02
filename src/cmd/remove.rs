use std::sync::Arc;

use anyhow::Result;
use serenity::{
    all::{CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage},
    prelude::*,
};

use crate::data::Database;

pub async fn execute(
    ctx: &Context,
    command: &CommandInteraction,
    database: &Arc<Database>,
) -> Result<()> {
    let url = extract_url(command)?;
    let guild_id = command.guild_id.unwrap().get();
    let removed = database.remove(guild_id, &url).await?;

    let content = if removed {
        format!("Successfully removed RSS feed: {}", url)
    } else {
        "RSS feed not found.".to_string()
    };

    respond(command, &ctx.http, &content).await
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

async fn respond(
    command: &CommandInteraction,
    http: &serenity::http::Http,
    content: &str,
) -> Result<()> {
    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(content)
            .ephemeral(true),
    );
    command.create_response(http, response).await?;
    Ok(())
}
