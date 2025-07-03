use std::{collections::HashSet, sync::Arc};

use anyhow::Result;
use serenity::{
    all::{CreateEmbed, CreateMessage, Http},
    model::id::ChannelId,
};
use tokio::{
    sync::{Mutex, Semaphore},
    time::{Duration, timeout},
};
use tracing::{error, info, warn};

use crate::{
    data::{Database, models::Feed as DbFeed},
    util::{fetcher, parser},
};

static FEED_CHECK_LOCK: Mutex<()> = Mutex::const_new(());
static POSTED_ARTICLES: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

pub async fn check(database: Arc<Database>, http: Arc<Http>) -> Result<()> {
    let _lock = FEED_CHECK_LOCK.try_lock();
    if _lock.is_err() {
        warn!("Feed check already in progress, skipping this cycle");
        return Ok(());
    }

    let feeds = database.feeds().await?;
    info!("Checking {} feeds", feeds.len());

    if feeds.is_empty() {
        info!("No feeds to check");
        return Ok(());
    }

    let semaphore = Arc::new(Semaphore::new(8));

    let tasks: Vec<_> = feeds
        .into_iter()
        .map(|feed| {
            let db = database.clone();
            let http = http.clone();
            let sem = semaphore.clone();

            tokio::spawn(async move {
                let _permit = sem.acquire().await.ok()?;
                let result = timeout(Duration::from_secs(45), process(&feed, &db, &http)).await;

                match result {
                    Ok(Ok(count)) => Some((feed.url.clone(), Ok(count))),
                    Ok(Err(e)) => Some((feed.url.clone(), Err(e))),
                    Err(_) => {
                        warn!("Feed check timed out: {}", feed.url);
                        Some((feed.url.clone(), Err(anyhow::anyhow!("Timeout"))))
                    }
                }
            })
        })
        .collect();

    let results: Vec<_> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|r| r.ok().flatten())
        .collect();

    let success = results.iter().filter(|(_, r)| r.is_ok()).count();
    let failed = results.iter().filter(|(_, r)| r.is_err()).count();

    info!(
        "Feed check complete: {} successful, {} failed",
        success, failed
    );

    for (url, result) in results.iter().filter(|(_, r)| r.is_err()) {
        if let Err(e) = result {
            if !e.to_string().contains("Timeout") {
                error!("Failed to check {}: {}", url, e);
            }
        }
    }

    Ok(())
}

pub async fn single(database: Arc<Database>, http: Arc<Http>, url: &str) -> Result<u32> {
    match database.find(url).await? {
        Some(feed) => process(&feed, &database, &http).await,
        None => Err(anyhow::anyhow!("Feed not found: {}", url)),
    }
}

async fn process(feed: &DbFeed, database: &Database, http: &Http) -> Result<u32> {
    info!("Checking feed: {}", feed.url);

    let content = match timeout(Duration::from_secs(15), fetcher::single(&feed.url)).await {
        Ok(Ok(content)) => content,
        Ok(Err(e)) => {
            warn!("Failed to fetch {}: {}", feed.url, e);
            return Err(e);
        }
        Err(_) => {
            warn!("Timeout fetching feed: {}", feed.url);
            return Err(anyhow::anyhow!("Timeout fetching feed"));
        }
    };

    let parsed_feed = parser::parse(&content)?;
    let total_items = parsed_feed.entries.len();

    if total_items == 0 {
        info!("Feed {} is empty", feed.url);
        return Ok(0);
    }

    info!("Feed {} has {} total items", feed.url, total_items);

    let mut new_items = 0u32;
    let mut newest_posted_date: Option<String> = None;

    let items_to_check = if feed.last_item_date.is_some() {
        std::cmp::min(3, total_items)
    } else {
        1
    };

    let mut sorted_entries = parsed_feed.entries.clone();
    sorted_entries.sort_by(|a, b| {
        let date_a = a.published.or(a.updated);
        let date_b = b.published.or(b.updated);
        date_b.cmp(&date_a)
    });

    for entry in sorted_entries.iter().take(items_to_check) {
        let entry_id = identifier(entry);

        {
            let posted_articles = POSTED_ARTICLES.lock().await;
            if posted_articles.contains(&entry_id) {
                info!("Skipping already posted article: {}", entry_id);
                continue;
            }
        }

        let should_post = if let Some(last_date) = &feed.last_item_date {
            if let Some(pub_date) = entry.published.or(entry.updated) {
                let entry_date = pub_date.to_rfc3339();
                entry_date > *last_date
            } else {
                false
            }
        } else {
            new_items == 0
        };

        if should_post {
            if let Some(title) = &entry.title {
                info!("Posting new item: {}", title.content);
            }

            match post(feed, entry, http).await {
                Ok(_) => {
                    new_items += 1;

                    {
                        let mut posted_articles = POSTED_ARTICLES.lock().await;
                        posted_articles.insert(entry_id);
                    }

                    if let Some(pub_date) = entry.published.or(entry.updated) {
                        let date_string = pub_date.to_rfc3339();
                        if newest_posted_date
                            .as_ref()
                            .map_or(true, |existing| date_string > *existing)
                        {
                            newest_posted_date = Some(date_string);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to post to channel: {}", e);
                    break;
                }
            }
        }
    }

    if new_items > 0 {
        info!("Updating last_item_date to: {:?}", newest_posted_date);
        if let Err(e) = database
            .update(feed.id, newest_posted_date.as_deref())
            .await
        {
            error!("Failed to update database for feed {}: {}", feed.url, e);
        }
        info!("Posted {} new items for feed: {}", new_items, feed.url);
    } else {
        info!("No new items for feed: {}", feed.url);
    }

    Ok(new_items)
}

fn identifier(entry: &feed_rs::model::Entry) -> String {
    let mut parts = Vec::new();

    if let Some(title) = &entry.title {
        let normalized_title = title
            .content
            .trim()
            .to_lowercase()
            .replace(
                &[
                    '\n', '\r', '\t', ':', '!', '?', '.', ',', ';', '-', '–', '—',
                ],
                " ",
            )
            .split_whitespace()
            .filter(|word| word.len() > 2)
            .collect::<Vec<_>>()
            .join(" ");

        if !normalized_title.is_empty() {
            parts.push(normalized_title);
        }
    }

    if let Some(link) = entry.links.first() {
        if let Ok(url) = url::Url::parse(&link.href) {
            if let Some(path) = url.path_segments() {
                let path_parts: Vec<&str> = path.collect();
                if !path_parts.is_empty() {
                    parts.push(path_parts.join("/"));
                }
            }
        } else {
            parts.push(link.href.clone());
        }
    }

    if !entry.id.is_empty() {
        parts.push(entry.id.clone());
    }

    if let Some(pub_date) = entry.published.or(entry.updated) {
        let date_str = pub_date.format("%Y-%m-%d").to_string();
        parts.push(date_str);
    }

    if parts.is_empty() {
        return format!(
            "entry_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
    }

    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };
    let mut hasher = DefaultHasher::new();
    parts.join("|").hash(&mut hasher);

    let hash = hasher.finish().to_string();

    tracing::debug!("Article identifier: {} -> {}", parts.join(" | "), hash);

    hash
}

async fn post(feed: &DbFeed, entry: &feed_rs::model::Entry, http: &Http) -> Result<()> {
    let channel_id = ChannelId::new(feed.channel_id as u64);

    let title = parser::truncate(&parser::title(entry), 256);
    let description = parser::description(entry);
    let url = entry.links.first().map(|l| l.href.clone());

    let embed_color = 0x5865f2;

    let mut embed = CreateEmbed::new()
        .title(&title)
        .description(&description)
        .color(embed_color);

    if let Some(link) = &url {
        embed = embed.url(link);
    }

    if let Some(pub_date) = entry.published.or(entry.updated) {
        embed = embed.timestamp(pub_date);
    }

    if let Some(image_url) = extract_image(entry) {
        embed = embed.image(image_url);
    }

    let footer_text = if let Some(feed_title) = &feed.title {
        parser::clean(feed_title)
    } else if let Ok(parsed_url) = url::Url::parse(&feed.url) {
        parsed_url.host_str().unwrap_or("RSS Feed").to_string()
    } else {
        "RSS Feed".to_string()
    };

    embed = embed.footer(serenity::all::CreateEmbedFooter::new(footer_text));

    let message = CreateMessage::new().embed(embed);

    for attempt in 0..2 {
        match channel_id.send_message(http, message.clone()).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                if attempt == 1 {
                    return Err(anyhow::anyhow!(
                        "Failed to send message after 2 attempts: {}",
                        e
                    ));
                }
                warn!(
                    "Failed to send message (attempt {}), retrying immediately",
                    attempt + 1
                );
            }
        }
    }

    Ok(())
}

fn extract_image(entry: &feed_rs::model::Entry) -> Option<String> {
    if let Some(content) = &entry.content {
        if let Some(body) = &content.body {
            if let Some(img_url) = extract_image_from_html(body) {
                return Some(img_url);
            }
        }
    }

    if let Some(summary) = &entry.summary {
        if let Some(img_url) = extract_image_from_html(&summary.content) {
            return Some(img_url);
        }
    }

    None
}

fn extract_image_from_html(html: &str) -> Option<String> {
    use std::sync::LazyLock;

    use regex::Regex;

    static IMG_REGEX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"<img[^>]+src=["']([^"']+)["'][^>]*>"#).unwrap());

    if let Some(captures) = IMG_REGEX.captures(html) {
        if let Some(url) = captures.get(1) {
            let image_url = url.as_str();

            if image_url.starts_with("http") && validate_image_url(image_url) {
                return Some(image_url.to_string());
            }
        }
    }

    None
}

fn validate_image_url(url: &str) -> bool {
    let image_extensions = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".svg"];
    let lower_url = url.to_lowercase();

    image_extensions.iter().any(|ext| lower_url.contains(ext))
        || lower_url.contains("image")
        || lower_url.contains("img")
}
