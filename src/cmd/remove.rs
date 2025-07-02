use std::sync::Arc;

use anyhow::Result;
use serenity::{
    all::{
        CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
        Permissions,
    },
    prelude::*,
};

use crate::data::Database;

pub async fn execute(
    ctx: &Context,
    command: &CommandInteraction,
    database: &Arc<Database>,
) -> Result<()> {
    if !check_permissions(ctx, command).await? {
        return Ok(());
    }

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

async fn check_permissions(ctx: &Context, command: &CommandInteraction) -> Result<bool> {
    if let Some(guild_id) = command.guild_id {
        if let Ok(member) = guild_id.member(&ctx.http, command.user.id).await {
            #[allow(deprecated)]
            let permissions = member.permissions(&ctx.cache)?;
            if !permissions.contains(Permissions::MANAGE_GUILD) {
                let response = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("You need the **Manage Server** permission to remove RSS feeds.")
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
