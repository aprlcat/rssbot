use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use serenity::{
    all::{
        ButtonStyle, ChannelType, CommandInteraction, ComponentInteraction,
        ComponentInteractionDataKind, CreateActionRow, CreateButton, CreateChannel, CreateEmbed,
        CreateEmbedFooter, CreateInteractionResponse, CreateInteractionResponseMessage,
        CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption, EditInteractionResponse,
    },
    prelude::*,
};
use tokio::sync::Mutex;
use tracing::error;

use crate::data::Database;

static STATES: std::sync::LazyLock<Mutex<HashMap<String, State>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
struct State {
    category_id: u64,
    topics: Vec<String>,
    guild_id: u64,
    user_id: u64,
}

pub async fn execute(
    ctx: &Context,
    command: &CommandInteraction,
    _database: &Arc<Database>,
) -> Result<()> {
    let guild_id = command.guild_id.unwrap();
    let user_id = command.user.id;

    let guild = match guild_id.to_partial_guild(&ctx.http).await {
        Ok(guild) => guild,
        Err(e) => {
            error!("Failed to get guild: {}", e);
            return respond_error(command, &ctx.http, "Failed to access guild information").await;
        }
    };

    let bot_user_id = ctx.cache.current_user().id;
    let bot_member = guild.member(&ctx.http, bot_user_id).await?;

    #[allow(deprecated)]
    let bot_permissions = guild.member_permissions(&bot_member);

    if !bot_permissions.manage_channels() {
        return respond_error(
            command,
            &ctx.http,
            "Missing required permission: Manage Channels",
        )
        .await;
    }

    defer(command, &ctx.http).await?;

    let channels = guild.channels(&ctx.http).await?;
    let category_channels = channels
        .into_iter()
        .filter(|(_, channel)| channel.kind == ChannelType::Category)
        .collect::<Vec<_>>();

    let state = State {
        category_id: 0,
        topics: Vec::new(),
        guild_id: guild_id.get(),
        user_id: user_id.get(),
    };

    {
        let mut states = STATES.lock().await;
        states.insert(key(guild_id.get(), user_id.get()), state);
    }

    categories(ctx, command, &category_channels).await
}

pub async fn handle_component(
    ctx: &Context,
    interaction: &ComponentInteraction,
    database: &Arc<Database>,
) -> Result<()> {
    let custom_id = &interaction.data.custom_id;
    let guild_id = interaction.guild_id.unwrap().get();
    let user_id = interaction.user.id.get();
    let state_key = key(guild_id, user_id);

    match &interaction.data.kind {
        ComponentInteractionDataKind::StringSelect { values } => {
            if custom_id == "setup_category_select" {
                if let Some(category_id_str) = values.first() {
                    let category_id = if category_id_str == "new_category" {
                        0
                    } else {
                        category_id_str
                            .parse::<u64>()
                            .map_err(|_| anyhow::anyhow!("Invalid category ID"))?
                    };

                    {
                        let mut states = STATES.lock().await;
                        if let Some(state) = states.get_mut(&state_key) {
                            state.category_id = category_id;
                        }
                    }

                    topics(ctx, interaction, database, category_id).await?;
                }
            } else if custom_id == "setup_topic_select" {
                let category_id = {
                    let states = STATES.lock().await;
                    states.get(&state_key).map(|s| s.category_id).unwrap_or(0)
                };

                {
                    let mut states = STATES.lock().await;
                    if let Some(state) = states.get_mut(&state_key) {
                        state.topics = values.clone();
                    }
                }

                confirmation(ctx, interaction, database, category_id, values).await?;
            }
        }
        ComponentInteractionDataKind::Button => {
            if custom_id == "setup_confirm" {
                let (category_id, topics) = {
                    let states = STATES.lock().await;
                    if let Some(state) = states.get(&state_key) {
                        (state.category_id, state.topics.clone())
                    } else {
                        return Ok(());
                    }
                };

                process(ctx, interaction, database, category_id, &topics).await?;

                {
                    let mut states = STATES.lock().await;
                    states.remove(&state_key);
                }
            } else if custom_id == "setup_cancel" {
                cancel(ctx, interaction).await?;

                {
                    let mut states = STATES.lock().await;
                    states.remove(&state_key);
                }
            }
        }
        _ => {}
    }

    Ok(())
}

async fn categories(
    ctx: &Context,
    command: &CommandInteraction,
    categories: &[(
        serenity::model::id::ChannelId,
        serenity::model::channel::GuildChannel,
    )],
) -> Result<()> {
    let mut options = vec![
        CreateSelectMenuOption::new("Create New Category", "new_category")
            .description("Create a new category for RSS feeds"),
    ];

    for (id, channel) in categories {
        options.push(
            CreateSelectMenuOption::new(&channel.name, id.to_string())
                .description(&format!("Use existing category: {}", channel.name)),
        );
    }

    let select_menu = CreateSelectMenu::new(
        "setup_category_select",
        CreateSelectMenuKind::String { options },
    )
    .placeholder("Choose a category for your RSS feeds");

    let embed = CreateEmbed::new()
        .title("RSS Feed Setup")
        .description("Select where to organize your RSS feeds")
        .color(0x89b4fa);

    let components = vec![CreateActionRow::SelectMenu(select_menu)];
    let response = EditInteractionResponse::new()
        .embed(embed)
        .components(components);

    command.edit_response(&ctx.http, response).await?;
    Ok(())
}

async fn topics(
    ctx: &Context,
    interaction: &ComponentInteraction,
    _database: &Arc<Database>,
    category_id: u64,
) -> Result<()> {
    let topics = match crate::cmd::opinionated::topics().await {
        Ok(topics) => topics,
        Err(e) => {
            error!("Failed to load topics: {}", e);
            return respond_component_error(
                interaction,
                &ctx.http,
                "Failed to load available topics",
            )
            .await;
        }
    };

    if topics.is_empty() {
        return respond_component_error(
            interaction,
            &ctx.http,
            "No curated feed collections available",
        )
        .await;
    }

    let options: Vec<_> = topics
        .iter()
        .map(|topic| {
            CreateSelectMenuOption::new(topic, topic)
                .description(&format!("Add {} RSS feeds", topic))
        })
        .collect();

    let select_menu = CreateSelectMenu::new(
        "setup_topic_select",
        CreateSelectMenuKind::String { options },
    )
    .placeholder("Select RSS feed topics (multiple allowed)")
    .min_values(1)
    .max_values(std::cmp::min(topics.len() as u8, 25));

    let category_name = if category_id == 0 {
        "New Category".to_string()
    } else {
        match serenity::model::id::ChannelId::new(category_id)
            .name(ctx)
            .await
        {
            Ok(name) => name,
            Err(_) => "Selected Category".to_string(),
        }
    };

    let embed = CreateEmbed::new()
        .title("Select Topics")
        .description("Choose the RSS feed topics you want to add")
        .field("Category", category_name, true)
        .field("Available Topics", topics.len().to_string(), true)
        .color(0xb4befe);

    let components = vec![CreateActionRow::SelectMenu(select_menu)];
    let response = CreateInteractionResponseMessage::new()
        .embed(embed)
        .components(components);

    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::UpdateMessage(response),
        )
        .await?;

    Ok(())
}

async fn confirmation(
    ctx: &Context,
    interaction: &ComponentInteraction,
    _database: &Arc<Database>,
    category_id: u64,
    topics: &[String],
) -> Result<()> {
    if topics.is_empty() {
        return respond_component_error(interaction, &ctx.http, "Please select at least one topic")
            .await;
    }

    let mut total_feeds = 0;
    let mut topic_fields = Vec::new();

    for topic in topics {
        match crate::cmd::opinionated::load_collection(topic).await {
            Ok(collection) => {
                let feed_count = collection.feeds.len();
                total_feeds += feed_count;
                topic_fields.push((topic.clone(), feed_count.to_string(), true));
            }
            Err(e) => {
                error!("Failed to load collection for topic {}: {}", topic, e);
                return respond_component_error(
                    interaction,
                    &ctx.http,
                    &format!("Failed to load topic: {}", topic),
                )
                .await;
            }
        }
    }

    let category_name = if category_id == 0 {
        "New Category".to_string()
    } else {
        match serenity::model::id::ChannelId::new(category_id)
            .name(ctx)
            .await
        {
            Ok(name) => name,
            Err(_) => "Selected Category".to_string(),
        }
    };

    let channels_list = topics
        .iter()
        .map(|topic| topic.to_lowercase().replace(' ', "-"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut embed = CreateEmbed::new()
        .title("Setup Confirmation")
        .description("Review your RSS feed setup configuration")
        .field("Category", category_name, true)
        .field("Selected Topics", topics.len().to_string(), true)
        .field("Total Feeds", total_feeds.to_string(), true)
        .field("Channels to Create", channels_list, false)
        .color(0xf9e2af)
        .footer(CreateEmbedFooter::new(
            "Click Confirm to proceed or Cancel to abort",
        ));

    for (name, value, inline) in topic_fields {
        embed = embed.field(name, format!("{} feeds", value), inline);
    }

    let buttons = vec![
        CreateButton::new("setup_confirm")
            .label("Confirm Setup")
            .style(ButtonStyle::Success),
        CreateButton::new("setup_cancel")
            .label("Cancel")
            .style(ButtonStyle::Secondary),
    ];

    let components = vec![CreateActionRow::Buttons(buttons)];
    let response = CreateInteractionResponseMessage::new()
        .embed(embed)
        .components(components);

    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::UpdateMessage(response),
        )
        .await?;

    Ok(())
}

async fn process(
    ctx: &Context,
    interaction: &ComponentInteraction,
    database: &Arc<Database>,
    category_id: u64,
    topics: &[String],
) -> Result<()> {
    let guild_id = interaction.guild_id.unwrap();

    let embed = CreateEmbed::new()
        .title("Setting Up RSS Feeds")
        .description("Creating channels and adding feeds...")
        .color(0x94e2d5);

    let response = CreateInteractionResponseMessage::new()
        .embed(embed)
        .components(vec![]);

    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::UpdateMessage(response),
        )
        .await?;

    let actual_category_id = if category_id == 0 {
        match create_category(&ctx, guild_id, "RSS Feeds").await {
            Ok(id) => id,
            Err(e) => {
                error!("Failed to create category: {}", e);
                return respond_update_error(interaction, &ctx.http, "Failed to create category")
                    .await;
            }
        }
    } else {
        category_id
    };

    let mut total_added = 0;
    let mut total_skipped = 0;
    let mut total_failed = 0;
    let mut channel_fields = Vec::new();

    for topic in topics {
        let collection = match crate::cmd::opinionated::load_collection(topic).await {
            Ok(collection) => collection,
            Err(e) => {
                error!("Failed to load collection for {}: {}", topic, e);
                channel_fields.push((topic.clone(), "Failed to load".to_string(), false));
                continue;
            }
        };

        let channel_name = topic.to_lowercase().replace(' ', "-");
        let channel_id = match create_channel(&ctx, guild_id, &channel_name, actual_category_id)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                error!("Failed to create channel for {}: {}", topic, e);
                channel_fields.push((topic.clone(), "Failed to create channel".to_string(), false));
                continue;
            }
        };

        let mut added_count = 0;
        let mut skipped_count = 0;
        let mut failed_feeds = 0;

        for feed in &collection.feeds {
            if database.exists(guild_id.get(), &feed.url).await? {
                skipped_count += 1;
                continue;
            }

            match database
                .add(
                    guild_id.get(),
                    channel_id,
                    &feed.url,
                    Some(&feed.name),
                    None,
                )
                .await
            {
                Ok(()) => added_count += 1,
                Err(e) => {
                    error!("Failed to add feed {} in {}: {}", feed.name, topic, e);
                    failed_feeds += 1;
                }
            }
        }

        total_added += added_count;
        total_skipped += skipped_count;
        total_failed += failed_feeds;

        channel_fields.push((
            format!("{} Channel", topic),
            format!(
                "<#{}>\n{} added, {} skipped, {} failed",
                channel_id, added_count, skipped_count, failed_feeds
            ),
            false,
        ));
    }

    let mut embed = CreateEmbed::new()
        .title("Setup Complete")
        .description("Your RSS feeds have been successfully configured")
        .field("Channels Created", topics.len().to_string(), true)
        .field("Total Feeds Added", total_added.to_string(), true)
        .field("Total Feeds Skipped", total_skipped.to_string(), true)
        .color(0xa6e3a1)
        .footer(CreateEmbedFooter::new("RSS feeds are now active"));

    for (name, value, inline) in channel_fields {
        embed = embed.field(name, value, inline);
    }

    let response = EditInteractionResponse::new()
        .embed(embed)
        .components(vec![]);

    interaction.edit_response(&ctx.http, response).await?;
    Ok(())
}

async fn create_category(
    ctx: &Context,
    guild_id: serenity::model::id::GuildId,
    name: &str,
) -> Result<u64> {
    let channel = guild_id
        .create_channel(
            &ctx.http,
            CreateChannel::new(name)
                .kind(ChannelType::Category)
                .permissions(vec![]),
        )
        .await?;

    Ok(channel.id.get())
}

async fn create_channel(
    ctx: &Context,
    guild_id: serenity::model::id::GuildId,
    name: &str,
    category_id: u64,
) -> Result<u64> {
    let channel = guild_id
        .create_channel(
            &ctx.http,
            CreateChannel::new(name)
                .kind(ChannelType::Text)
                .category(serenity::model::id::ChannelId::new(category_id))
                .permissions(vec![]),
        )
        .await?;

    Ok(channel.id.get())
}

async fn cancel(ctx: &Context, interaction: &ComponentInteraction) -> Result<()> {
    let embed = CreateEmbed::new()
        .title("Setup Cancelled")
        .description("No changes were made to your server")
        .color(0xf38ba8);

    let response = CreateInteractionResponseMessage::new().embed(embed);

    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::UpdateMessage(response),
        )
        .await?;

    Ok(())
}

async fn respond_error(
    command: &CommandInteraction,
    http: &serenity::http::Http,
    message: &str,
) -> Result<()> {
    let embed = CreateEmbed::new()
        .title("Error")
        .description(message)
        .color(0xf38ba8);

    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .embed(embed)
            .ephemeral(true),
    );
    command.create_response(http, response).await?;
    Ok(())
}

async fn respond_component_error(
    interaction: &ComponentInteraction,
    http: &serenity::http::Http,
    message: &str,
) -> Result<()> {
    let embed = CreateEmbed::new()
        .title("Error")
        .description(message)
        .color(0xf38ba8);

    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .embed(embed)
            .ephemeral(true),
    );
    interaction.create_response(http, response).await?;
    Ok(())
}

async fn respond_update_error(
    interaction: &ComponentInteraction,
    http: &serenity::http::Http,
    message: &str,
) -> Result<()> {
    let embed = CreateEmbed::new()
        .title("Error")
        .description(message)
        .color(0xf38ba8);

    let response = EditInteractionResponse::new()
        .embed(embed)
        .components(vec![]);

    interaction.edit_response(http, response).await?;
    Ok(())
}

async fn defer(command: &CommandInteraction, http: &serenity::http::Http) -> Result<()> {
    let response =
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true));
    command.create_response(http, response).await?;
    Ok(())
}

fn key(guild_id: u64, user_id: u64) -> String {
    format!("{}:{}", guild_id, user_id)
}
