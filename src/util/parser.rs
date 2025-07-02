use anyhow::Result;
use feed_rs::parser;

pub fn parse(content: &str) -> Result<feed_rs::model::Feed> {
    let feed = parser::parse(content.as_bytes())?;
    Ok(feed)
}
