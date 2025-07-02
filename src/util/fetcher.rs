use std::time::Duration;

use anyhow::Result;
use reqwest::Client;
use tracing::debug;

pub async fn fetch(url: &str) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent("Mozilla/5.0 RSS Bot")
        .build()?;

    debug!("Fetching feed from: {}", url);
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", response.status()));
    }

    if let Some(content_length) = response.content_length() {
        debug!("Feed content length: {} bytes", content_length);
        if content_length > 50_000_000 {
            return Err(anyhow::anyhow!("Feed too large: {} bytes", content_length));
        }
    }

    let content = response.text().await?;

    if content.len() > 50_000_000 {
        return Err(anyhow::anyhow!(
            "Feed content too large: {} bytes",
            content.len()
        ));
    }

    debug!("Successfully fetched feed, size: {} bytes", content.len());
    Ok(content)
}

pub async fn color(url: &str) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 RSS Bot")
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", response.status()));
    }

    let html = response.text().await?;

    let extracted_color = tokio::task::spawn_blocking(move || extract(&html)).await??;

    Ok(extracted_color)
}

fn extract(html: &str) -> Result<String> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);

    let theme_color_selector = Selector::parse(r#"meta[name="theme-color"]"#).unwrap();
    if let Some(element) = document.select(&theme_color_selector).next() {
        if let Some(content) = element.value().attr("content") {
            if let Ok(parsed_color) = validate(content) {
                return Ok(parsed_color);
            }
        }
    }

    let theme_color_selector2 = Selector::parse(r#"meta[property="theme-color"]"#).unwrap();
    if let Some(element) = document.select(&theme_color_selector2).next() {
        if let Some(content) = element.value().attr("content") {
            if let Ok(parsed_color) = validate(content) {
                return Ok(parsed_color);
            }
        }
    }

    Err(anyhow::anyhow!("No theme color found"))
}

fn validate(color_str: &str) -> Result<String> {
    let cleaned = color_str.trim().trim_start_matches('#');

    if cleaned.len() == 6 && cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(cleaned.to_uppercase())
    } else if cleaned.len() == 3 && cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
        let expanded: String = cleaned.chars().map(|c| format!("{}{}", c, c)).collect();
        Ok(expanded.to_uppercase())
    } else {
        Err(anyhow::anyhow!("Invalid color format"))
    }
}
