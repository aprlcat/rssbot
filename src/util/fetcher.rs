use std::{sync::Arc, time::Duration};

use anyhow::Result;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use tokio::time::timeout;

pub async fn single(url: &str) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0 RSS Bot")
        .build()?;

    fetch(&client, url).await
}

pub async fn batch(urls: Vec<String>) -> Vec<Result<(String, String)>> {
    let client = Arc::new(
        Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("Mozilla/5.0 RSS Bot")
            .build()
            .unwrap_or_default(),
    );

    stream::iter(urls)
        .map(|url| {
            let client = client.clone();
            let url_clone = url.clone();
            async move {
                match timeout(Duration::from_secs(20), fetch(&client, &url)).await {
                    Ok(Ok(content)) => Ok((url_clone, content)),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(anyhow::anyhow!("Timeout")),
                }
            }
        })
        .buffer_unordered(50)
        .collect()
        .await
}

async fn fetch(client: &Client, url: &str) -> Result<String> {
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", response.status()));
    }

    let bytes = response.bytes().await?;
    if bytes.len() > 5_000_000 {
        return Err(anyhow::anyhow!("Feed too large: {} bytes", bytes.len()));
    }

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub async fn color(url: &str) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .user_agent("Mozilla/5.0 RSS Bot")
        .build()?;

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", response.status()));
    }

    let html = response.text().await?;
    tokio::task::spawn_blocking(move || extract(&html)).await?
}

fn extract(html: &str) -> Result<String> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);

    for selector in &[
        r#"meta[name="theme-color"]"#,
        r#"meta[property="theme-color"]"#,
    ] {
        if let Ok(sel) = Selector::parse(selector) {
            if let Some(element) = document.select(&sel).next() {
                if let Some(content) = element.value().attr("content") {
                    if let Ok(color) = validate(content) {
                        return Ok(color);
                    }
                }
            }
        }
    }

    Err(anyhow::anyhow!("No theme color found"))
}

fn validate(color: &str) -> Result<String> {
    let cleaned = color.trim().trim_start_matches('#');

    match cleaned.len() {
        6 if cleaned.chars().all(|c| c.is_ascii_hexdigit()) => Ok(cleaned.to_uppercase()),
        3 if cleaned.chars().all(|c| c.is_ascii_hexdigit()) => Ok(cleaned
            .chars()
            .flat_map(|c| [c, c])
            .collect::<String>()
            .to_uppercase()),
        _ => Err(anyhow::anyhow!("Invalid color format")),
    }
}
