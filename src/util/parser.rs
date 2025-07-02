use anyhow::Result;

pub fn parse(content: &str) -> Result<feed_rs::model::Feed> {
    let feed = feed_rs::parser::parse(content.as_bytes())?;
    Ok(feed)
}
