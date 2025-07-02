use std::sync::Arc;

use anyhow::Result;
use serenity::{
    all::{
        CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
        EditInteractionResponse,
    },
    prelude::*,
};

use crate::{
    data::Database,
    scheduler::tasks::{check, single},
};

pub async fn execute(
    ctx: &Context,
    command: &CommandInteraction,
    database: &Arc<Database>,
) -> Result<()> {
    let url = extract_url(command);
    defer_response(command, &ctx.http).await?;

    let result = if let Some(feed_url) = url {
        sync_single(database, ctx, &feed_url).await
    } else {
        sync_all(database, ctx).await
    };

    let edit_response = EditInteractionResponse::new().content(result);
    command.edit_response(&ctx.http, edit_response).await?;
    Ok(())
}

fn extract_url(command: &CommandInteraction) -> Option<String> {
    command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "url")
        .and_then(|opt| opt.value.as_str())
        .map(|s| s.to_string())
}

async fn defer_response(command: &CommandInteraction, http: &serenity::http::Http) -> Result<()> {
    let response =
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true));
    command.create_response(http, response).await?;
    Ok(())
}

async fn sync_single(database: &Arc<Database>, ctx: &Context, feed_url: &str) -> String {
    match single(database.clone(), ctx.http.clone(), feed_url).await {
        Ok(new_items) => {
            if new_items > 0 {
                format!("Synced feed and found {} new items", new_items)
            } else {
                "Synced feed, no new items found".to_string()
            }
        }
        Err(e) => format!("Failed to sync feed: {}", e),
    }
}

async fn sync_all(database: &Arc<Database>, ctx: &Context) -> String {
    match check(database.clone(), ctx.http.clone()).await {
        Ok(_) => "Successfully synced all feeds".to_string(),
        Err(e) => format!("Failed to sync feeds: {}", e),
    }
}
