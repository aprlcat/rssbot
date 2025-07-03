use std::sync::Arc;

use anyhow::Result;
use serenity::{
    all::{
        ButtonStyle, CommandInteraction, ComponentInteraction, ComponentInteractionDataKind,
        CreateActionRow, CreateButton, CreateEmbed, CreateInputText, CreateInteractionResponse,
        CreateInteractionResponseMessage, CreateModal, CreateSelectMenu, CreateSelectMenuKind,
        CreateSelectMenuOption, EditInteractionResponse, InputTextStyle, ModalInteraction,
    },
    prelude::*,
};
use tracing::{info, warn};

use crate::data::Database;

const FEEDS_PER_PAGE: usize = 10;

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

    let page = 0;
    let total_pages = (feeds.len() + FEEDS_PER_PAGE - 1) / FEEDS_PER_PAGE;

    let (embed, components) = build_page_fast(&feeds, page, total_pages);

    let mut response = EditInteractionResponse::new().embed(embed);
    if total_pages > 1 {
        response = response.components(components);
    }

    command.edit_response(&ctx.http, response).await?;
    Ok(())
}

pub async fn handle_component(
    ctx: &Context,
    interaction: &ComponentInteraction,
    database: &Arc<Database>,
) -> Result<()> {
    let guild_id = interaction.guild_id.unwrap().get();
    let feeds = database.guild(guild_id).await?;

    if feeds.is_empty() {
        warn!("No feeds found for guild {}", guild_id);
        return Ok(());
    }

    let total_pages = (feeds.len() + FEEDS_PER_PAGE - 1) / FEEDS_PER_PAGE;
    info!(
        "Handling component interaction: {} (total pages: {})",
        interaction.data.custom_id, total_pages
    );

    match &interaction.data.kind {
        ComponentInteractionDataKind::Button => {
            let custom_id = &interaction.data.custom_id;
            let current_page = extract_page_from_custom_id(custom_id);

            info!(
                "Button interaction: {} (current page: {})",
                custom_id, current_page
            );

            let new_page = match custom_id.split('_').next() {
                Some("prev") => {
                    let new_page = current_page.saturating_sub(1);
                    info!("Going to previous page: {} -> {}", current_page, new_page);
                    new_page
                }
                Some("next") => {
                    let new_page = std::cmp::min(current_page + 1, total_pages - 1);
                    info!("Going to next page: {} -> {}", current_page, new_page);
                    new_page
                }
                Some("jump") => {
                    let modal =
                        CreateModal::new("page_jump_modal", "Jump to Page").components(vec![
                            CreateActionRow::InputText(
                                CreateInputText::new(InputTextStyle::Short, "page", "Page Number")
                                    .placeholder(&format!("1-{}", total_pages))
                                    .min_length(1)
                                    .max_length(3)
                                    .required(true),
                            ),
                        ]);

                    interaction
                        .create_response(&ctx.http, CreateInteractionResponse::Modal(modal))
                        .await?;
                    return Ok(());
                }
                _ => {
                    warn!("Unknown button interaction: {}", custom_id);
                    current_page
                }
            };

            let (embed, components) = build_page_fast(&feeds, new_page, total_pages);

            let response_message = CreateInteractionResponseMessage::new()
                .embed(embed)
                .components(components);

            let response = CreateInteractionResponse::UpdateMessage(response_message);

            interaction.create_response(&ctx.http, response).await?;
            info!("Page navigation successful: page {}", new_page + 1);
        }
        ComponentInteractionDataKind::StringSelect { values } => {
            info!("Select menu interaction with values: {:?}", values);

            if let Some(selected_page) = values.first() {
                if let Ok(page) = selected_page.parse::<usize>() {
                    let page = page.saturating_sub(1);
                    info!("Selected page from dropdown: {}", page + 1);

                    let (embed, components) = build_page_fast(&feeds, page, total_pages);

                    let response_message = CreateInteractionResponseMessage::new()
                        .embed(embed)
                        .components(components);

                    let response = CreateInteractionResponse::UpdateMessage(response_message);

                    interaction.create_response(&ctx.http, response).await?;
                    info!("Dropdown navigation successful: page {}", page + 1);
                }
            }
        }
        _ => {
            warn!("Unknown component interaction type");
        }
    }

    Ok(())
}

pub async fn handle_modal(
    ctx: &Context,
    interaction: &ModalInteraction,
    database: &Arc<Database>,
) -> Result<()> {
    if interaction.data.custom_id != "page_jump_modal" {
        return Ok(());
    }

    let guild_id = interaction.guild_id.unwrap().get();
    let feeds = database.guild(guild_id).await?;

    if feeds.is_empty() {
        return Ok(());
    }

    let total_pages = (feeds.len() + FEEDS_PER_PAGE - 1) / FEEDS_PER_PAGE;

    let page_input = interaction
        .data
        .components
        .first()
        .and_then(|row| row.components.first())
        .and_then(|component| match component {
            serenity::all::ActionRowComponent::InputText(input) => input.value.as_ref(),
            _ => None,
        })
        .map_or("1", |s| s);

    let page = match page_input.parse::<usize>() {
        Ok(p) if p > 0 && p <= total_pages => p - 1,
        _ => {
            interaction
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content(&format!(
                                "Invalid page number. Please enter a number between 1 and {}.",
                                total_pages
                            ))
                            .ephemeral(true),
                    ),
                )
                .await?;
            return Ok(());
        }
    };

    let (embed, components) = build_page_fast(&feeds, page, total_pages);

    let response_message = CreateInteractionResponseMessage::new()
        .embed(embed)
        .components(components);

    let response = CreateInteractionResponse::UpdateMessage(response_message);

    interaction.create_response(&ctx.http, response).await?;
    info!("Modal jump successful: page {}", page + 1);
    Ok(())
}

fn build_page_fast(
    feeds: &[crate::data::models::Feed],
    page: usize,
    total_pages: usize,
) -> (CreateEmbed, Vec<CreateActionRow>) {
    let start_idx = page * FEEDS_PER_PAGE;
    let end_idx = std::cmp::min(start_idx + FEEDS_PER_PAGE, feeds.len());
    let page_feeds = &feeds[start_idx..end_idx];

    let description = build_description_fast(page_feeds, start_idx);

    let embed = CreateEmbed::new()
        .title("RSS Feeds")
        .description(description)
        .color(0x7289da)
        .footer(serenity::all::CreateEmbedFooter::new(format!(
            "Page {} of {} • {} total feeds",
            page + 1,
            total_pages,
            feeds.len()
        )));

    let mut components = Vec::new();

    if total_pages > 1 {
        let mut buttons = Vec::new();

        buttons.push(
            CreateButton::new(format!("prev_{}", page))
                .emoji('◀')
                .style(ButtonStyle::Secondary)
                .disabled(page == 0),
        );

        buttons.push(
            CreateButton::new(format!("jump_{}", page))
                .emoji('🎚')
                .style(ButtonStyle::Primary)
                .label(&format!("{}/{}", page + 1, total_pages)),
        );

        buttons.push(
            CreateButton::new(format!("next_{}", page))
                .emoji('▶')
                .style(ButtonStyle::Secondary)
                .disabled(page >= total_pages - 1),
        );

        components.push(CreateActionRow::Buttons(buttons));

        if total_pages > 5 {
            let mut options = Vec::new();
            let start_page = if page < 5 { 0 } else { page - 4 };
            let end_page = std::cmp::min(start_page + 10, total_pages);

            for i in start_page..end_page {
                options.push(
                    CreateSelectMenuOption::new(format!("Page {}", i + 1), (i + 1).to_string())
                        .default_selection(i == page),
                );
            }

            let select_menu =
                CreateSelectMenu::new("page_select", CreateSelectMenuKind::String { options })
                    .placeholder("Jump to page...");

            components.push(CreateActionRow::SelectMenu(select_menu));
        }
    }

    (embed, components)
}

fn build_description_fast(feeds: &[crate::data::models::Feed], start_idx: usize) -> String {
    let mut description = String::new();

    for (i, feed) in feeds.iter().enumerate() {
        let channel_mention = format!("<#{}>", feed.channel_id);
        let domain = extract_domain(&feed.url);

        let last_updated = if let Some(ref last_date) = feed.last_item_date {
            if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(last_date) {
                parsed.format("%b %d, %Y").to_string()
            } else {
                "Recently".to_string()
            }
        } else {
            "Never".to_string()
        };

        description.push_str(&format!(
            "{}. `{}` → {} | Last updated: {}\n",
            start_idx + i + 1,
            domain,
            channel_mention,
            last_updated
        ));
    }

    description
}

fn extract_page_from_custom_id(custom_id: &str) -> usize {
    custom_id
        .split('_')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
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

fn extract_domain(url: &str) -> String {
    if let Ok(parsed_url) = url::Url::parse(url) {
        parsed_url.host_str().unwrap_or("Unknown").to_string()
    } else {
        "Unknown".to_string()
    }
}
