use std::time::Duration;

use anyhow::Result;
use reqwest::Client;

pub async fn single(url: &str) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0 RSS Bot")
        .build()?;

    fetch(&client, url).await
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
