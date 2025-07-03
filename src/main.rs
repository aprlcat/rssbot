use std::sync::Arc;

use anyhow::Result;
use serenity::{
    all::{
        ActivityData, AutocompleteChoice, Command, CommandOptionType, CreateAutocompleteResponse,
        CreateCommand, CreateInteractionResponse, CreateInteractionResponseFollowup,
        CreateInteractionResponseMessage, Interaction, OnlineStatus, Permissions, Ready,
    },
    async_trait,
    prelude::*,
};
use sqlx::SqlitePool;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};

use crate::{data::Database, scheduler::tasks::check};

mod cmd;
mod data;
mod scheduler;
mod util;

#[derive(Debug)]
struct Config {
    token: String,
    check_interval_minutes: u64,
    database_url: String,
}

impl Config {
    fn load() -> Result<Self> {
        let config_str = std::fs::read_to_string("config.toml")?;
        let config: toml::Value = toml::from_str(&config_str)?;

        Ok(Self {
            token: config["bot"]["token"].as_str().unwrap().to_string(),
            check_interval_minutes: config["bot"]["check_interval_minutes"]
                .as_integer()
                .unwrap_or(15) as u64,
            database_url: config["database"]["url"].as_str().unwrap().to_string(),
        })
    }
}

struct Handler {
    database: Arc<Database>,
}

impl Handler {
    async fn update(&self, ctx: &Context) {
        match self.database.feeds().await {
            Ok(feeds) => {
                let count = feeds.len();
                let activity = ActivityData::watching(format!("{} feeds", count));
                ctx.set_presence(Some(activity), OnlineStatus::Online);
                info!("Updated status: Watching {} feeds", count);
            }
            Err(e) => error!("Failed to get feed count for status: {}", e),
        }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(command) => {
                let result = match command.data.name.as_str() {
                    "add" => {
                        let result = cmd::add::execute(&ctx, &command, &self.database).await;
                        self.update(&ctx).await;
                        result
                    }
                    "remove" => {
                        let result = cmd::remove::execute(&ctx, &command, &self.database).await;
                        self.update(&ctx).await;
                        result
                    }
                    "list" => cmd::list::execute(&ctx, &command, &self.database).await,
                    "sync" => cmd::sync::execute(&ctx, &command, &self.database).await,
                    "opinionated" => {
                        let result =
                            cmd::opinionated::execute(&ctx, &command, &self.database).await;
                        self.update(&ctx).await;
                        result
                    }
                    "setup" => {
                        let result = cmd::setup::execute(&ctx, &command, &self.database).await;
                        self.update(&ctx).await;
                        result
                    }
                    _ => Ok(()),
                };

                if let Err(e) = result {
                    error!("Command error: {}", e);
                    let response = CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("An error occurred while processing the command.")
                            .ephemeral(true),
                    );
                    let _ = command.create_response(&ctx.http, response).await;
                }
            }
            Interaction::Component(component) => {
                if component.data.custom_id.starts_with("prev_")
                    || component.data.custom_id.starts_with("next_")
                    || component.data.custom_id.starts_with("jump_")
                    || component.data.custom_id == "page_select"
                {
                    if let Err(e) =
                        cmd::list::handle_component(&ctx, &component, &self.database).await
                    {
                        error!("Component interaction error: {}", e);
                        let _ = component
                            .create_followup(
                                &ctx.http,
                                CreateInteractionResponseFollowup::new()
                                    .content(
                                        "An error occurred while processing your request. Please \
                                         try again.",
                                    )
                                    .ephemeral(true),
                            )
                            .await;
                    }
                } else if component.data.custom_id.starts_with("setup_") {
                    if let Err(e) =
                        cmd::setup::handle_component(&ctx, &component, &self.database).await
                    {
                        error!("Setup component interaction error: {}", e);
                        let _ = component
                            .create_followup(
                                &ctx.http,
                                CreateInteractionResponseFollowup::new()
                                    .content(
                                        "An error occurred while processing your request. Please \
                                         try again.",
                                    )
                                    .ephemeral(true),
                            )
                            .await;
                    }
                }
            }
            Interaction::Modal(modal) => {
                if modal.data.custom_id == "page_jump_modal" {
                    if let Err(e) = cmd::list::handle_modal(&ctx, &modal, &self.database).await {
                        error!("Modal interaction error: {}", e);
                    }
                }
            }
            Interaction::Autocomplete(autocomplete) => {
                if autocomplete.data.name == "opinionated" {
                    let current_value = autocomplete
                        .data
                        .options
                        .iter()
                        .find(|opt| opt.name == "topic")
                        .and_then(|opt| opt.value.as_str())
                        .unwrap_or("");

                    match cmd::opinionated::topics().await {
                        Ok(topics) => {
                            let filtered_topics: Vec<_> = topics
                                .iter()
                                .filter(|topic| {
                                    topic.to_lowercase().contains(&current_value.to_lowercase())
                                })
                                .take(25)
                                .map(|topic| AutocompleteChoice::new(topic.clone(), topic.clone()))
                                .collect();

                            let response = CreateInteractionResponse::Autocomplete(
                                CreateAutocompleteResponse::new().set_choices(filtered_topics),
                            );
                            let _ = autocomplete.create_response(&ctx.http, response).await;
                        }
                        Err(e) => {
                            error!("Failed to load topics for autocomplete: {}", e);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
        self.update(&ctx).await;

        let commands = vec![
            CreateCommand::new("add")
                .description("Add an RSS feed to a channel")
                .default_member_permissions(Permissions::MANAGE_GUILD)
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        CommandOptionType::String,
                        "url",
                        "RSS feed URL",
                    )
                    .required(true),
                )
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        CommandOptionType::Channel,
                        "channel",
                        "Channel to send RSS feeds to (defaults to current channel)",
                    )
                    .required(false),
                ),
            CreateCommand::new("remove")
                .description("Remove an RSS feed")
                .default_member_permissions(Permissions::MANAGE_GUILD)
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        CommandOptionType::String,
                        "url",
                        "RSS feed URL",
                    )
                    .required(true),
                ),
            CreateCommand::new("list").description("List all RSS feeds"),
            CreateCommand::new("sync")
                .description("Manually sync RSS feeds")
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        CommandOptionType::String,
                        "url",
                        "Specific RSS feed URL to sync (optional)",
                    )
                    .required(false),
                ),
            CreateCommand::new("opinionated")
                .description("Add curated RSS feeds from community collections")
                .default_member_permissions(Permissions::MANAGE_GUILD)
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        CommandOptionType::String,
                        "topic",
                        "Topic collection to add feeds from",
                    )
                    .required(true)
                    .set_autocomplete(true),
                )
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        CommandOptionType::Channel,
                        "channel",
                        "Channel to send RSS feeds to (defaults to current channel)",
                    )
                    .required(false),
                ),
            CreateCommand::new("setup")
                .description("Interactive setup for RSS feeds with categories and channels")
                .default_member_permissions(Permissions::MANAGE_GUILD),
        ];

        if let Err(e) = Command::set_global_commands(&ctx.http, commands).await {
            error!("Failed to set commands: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rssbot=info".parse().unwrap())
                .add_directive("serenity=warn".parse().unwrap()),
        )
        .init();

    info!("Starting RSS Bot...");

    let config = Config::load()?;
    let pool = SqlitePool::connect(&config.database_url).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    let database = Arc::new(Database::new(pool));

    let mut client = Client::builder(
        &config.token,
        GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MEMBERS,
    )
    .event_handler(Handler {
        database: database.clone(),
    })
    .await?;

    let mut scheduler = JobScheduler::new().await?;

    let interval_minutes = config.check_interval_minutes;
    let db_for_job = database.clone();
    let http_for_job = client.http.clone();

    scheduler
        .add(Job::new_async(
            &format!("0 */{} * * * *", interval_minutes),
            move |_uuid, _l| {
                let db = db_for_job.clone();
                let http = http_for_job.clone();
                Box::pin(async move {
                    if let Err(e) = check(db, http).await {
                        error!("Feed check error: {}", e);
                    }
                })
            },
        )?)
        .await?;

    scheduler.start().await?;
    info!(
        "Scheduler started with {} minute intervals",
        interval_minutes
    );

    tokio::select! {
        result = client.start() => {
            if let Err(e) = result {
                error!("Client error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down gracefully...");
            scheduler.shutdown().await?;
            info!("Scheduler shut down");
        }
    }

    Ok(())
}
