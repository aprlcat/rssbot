use std::{collections::HashSet, sync::Arc};

use anyhow::Result;
use futures::stream::{self, StreamExt};
use serenity::{
    all::{CreateEmbed, ExecuteWebhook, Http},
    model::webhook::Webhook,
};
use tokio::{
    sync::Semaphore,
    time::{Duration, timeout},
};
use tracing::{error, info};
use url::Url;

use crate::{
    data::{Database, models::Feed as DbFeed},
    util::{fetcher, parser},
};

pub async fn check(database: Arc<Database>, http: Arc<Http>) -> Result<()> {
    let feeds = database.feeds().await?;
    info!("Checking {} feeds", feeds.len());

    let semaphore = Arc::new(Semaphore::new(50));

    let tasks: Vec<_> = feeds
        .into_iter()
        .map(|feed| {
            let db = database.clone();
            let http = http.clone();
            let sem = semaphore.clone();

            tokio::spawn(async move {
                let _permit = sem.acquire().await.ok()?;
                let result = timeout(Duration::from_secs(120), process(&feed, &db, &http)).await;

                match result {
                    Ok(Ok(count)) => Some((feed.url.clone(), Ok(count))),
                    Ok(Err(e)) => Some((feed.url.clone(), Err(e))),
                    Err(_) => Some((feed.url.clone(), Err(anyhow::anyhow!("Timeout")))),
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
            error!("Failed to check {}: {}", url, e);
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

    let content = match timeout(Duration::from_secs(30), fetcher::single(&feed.url)).await {
        Ok(Ok(content)) => content,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(anyhow::anyhow!("Timeout fetching feed")),
    };

    let parsed_feed = parser::parse(&content)?;

    let total_items = parsed_feed.entries.len();
    info!("Feed {} has {} total items", feed.url, total_items);

    let mut new_items = 0u32;
    let mut newest_posted_date: Option<String> = None;
    let mut posted_items = HashSet::new();

    let items_to_check = if feed.last_item_date.is_some() {
        std::cmp::min(20, total_items)
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

        if posted_items.contains(&entry_id) {
            continue;
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
                    posted_items.insert(entry_id);

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
                    error!("Failed to post to webhook: {}", e);
                }
            }

            if new_items > 0 {
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
        }
    }

    if new_items > 0 {
        info!("Updating last_item_date to: {:?}", newest_posted_date);
        database
            .update(feed.id, newest_posted_date.as_deref())
            .await?;
        info!("Posted {} new items for feed: {}", new_items, feed.url);
    } else {
        info!("No new items for feed: {}", feed.url);
    }

    Ok(new_items)
}

fn identifier(entry: &feed_rs::model::Entry) -> String {
    if !entry.id.is_empty() {
        return entry.id.clone();
    }

    if let Some(link) = entry.links.first() {
        return link.href.clone();
    }

    if let Some(title) = &entry.title {
        if let Some(pub_date) = entry.published.or(entry.updated) {
            return format!("{}|{}", title.content, pub_date.to_rfc3339());
        }
        return title.content.clone();
    }

    if let Some(pub_date) = entry.published.or(entry.updated) {
        return pub_date.to_rfc3339();
    }

    let mut content_parts = Vec::new();

    if let Some(title) = &entry.title {
        content_parts.push(title.content.clone());
    }

    if let Some(summary) = &entry.summary {
        content_parts.push(summary.content.clone());
    }

    if let Some(link) = entry.links.first() {
        content_parts.push(link.href.clone());
    }

    if content_parts.is_empty() {
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
    content_parts.join("|").hash(&mut hasher);
    hasher.finish().to_string()
}

async fn post(feed: &DbFeed, entry: &feed_rs::model::Entry, http: &Http) -> Result<()> {
    let webhook_url = feed
        .webhook_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No webhook URL"))?;

    let webhook = Webhook::from_url(http, webhook_url).await?;

    let title = entry
        .title
        .as_ref()
        .map(|t| {
            let content = &t.content;
            if content.len() > 256 {
                format!("{}...", &content[..253])
            } else {
                content.clone()
            }
        })
        .unwrap_or_else(|| "Untitled".to_string());

    let description = entry
        .summary
        .as_ref()
        .map(|s| {
            let content = clean(&s.content);
            if content.len() > 2000 {
                format!("{}...", &content[..1997])
            } else {
                content
            }
        })
        .unwrap_or_else(|| "No description available.".to_string());

    let url = entry.links.first().map(|l| l.href.clone());

    let embed_color = if let Some(link) = &url {
        match timeout(Duration::from_secs(5), fetcher::color(link)).await {
            Ok(Ok(color_str)) => u32::from_str_radix(&color_str, 16).unwrap_or(0xb4befe),
            _ => 0xb4befe,
        }
    } else {
        0xb4befe
    };

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

    let username = if let Ok(parsed_url) = Url::parse(&feed.url) {
        parsed_url.host_str().unwrap_or("RSS Feed").to_string()
    } else {
        "RSS Feed".to_string()
    };

    let execute_webhook = ExecuteWebhook::new().username(&username).embed(embed);

    webhook.execute(http, false, execute_webhook).await?;
    Ok(())
}

fn clean(input: &str) -> String {
    use regex::Regex;

    let cdata_regex = Regex::new(r"<!\[CDATA\[(.*?)\]\]>").unwrap();
    let without_cdata = cdata_regex.replace_all(input, "$1");

    let html_regex = Regex::new(r"<[^>]*>").unwrap();
    let result = html_regex.replace_all(&without_cdata, "");

    let whitespace_regex = Regex::new(r"\s+").unwrap();
    let cleaned = whitespace_regex.replace_all(&result, " ");

    cleaned
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}
