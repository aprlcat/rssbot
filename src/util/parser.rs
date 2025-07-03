use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;

static HTML_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static CDATA_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!\[CDATA\[(.*?)\]\]>").unwrap());
static SCRIPT_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap());
static STYLE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap());
static WHITESPACE_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static WAGTAIL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<wagtail[^>]*>.*?</wagtail>|<wagtail\.rich_text\.RichText[^>]*>").unwrap()
});
static STRUCT_VALUE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<ListValue:\s*\[StructValue\([^)]*\)\]>|StructValue\([^)]*\)").unwrap()
});
static ASIDE_BLOCK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"aside_block\s+<[^>]*>").unwrap());
static OBJECT_REFERENCE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]*object at 0x[a-fA-F0-9]+>").unwrap());
static ENCODED_ENTITIES_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#\d+;").unwrap());

pub fn parse(content: &str) -> Result<feed_rs::model::Feed> {
    let feed = feed_rs::parser::parse(content.as_bytes())?;
    Ok(feed)
}

pub fn clean(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }

    let text = strip(input);
    let text = decode(&text);
    let text = artifacts(&text);
    let text = normalize(&text);
    let text = format(&text);

    text.trim().to_string()
}

fn strip(input: &str) -> String {
    let without_cdata = CDATA_REGEX.replace_all(input, "$1");
    let without_scripts = SCRIPT_REGEX.replace_all(&without_cdata, "");
    let without_styles = STYLE_REGEX.replace_all(&without_scripts, "");
    HTML_REGEX.replace_all(&without_styles, "").to_string()
}

fn artifacts(input: &str) -> String {
    let without_wagtail = WAGTAIL_REGEX.replace_all(input, "");
    let without_struct = STRUCT_VALUE_REGEX.replace_all(&without_wagtail, "");
    let without_aside = ASIDE_BLOCK_REGEX.replace_all(&without_struct, "");
    let without_objects = OBJECT_REFERENCE_REGEX.replace_all(&without_aside, "");
    let clean_entities = ENCODED_ENTITIES_REGEX.replace_all(&without_objects, "");

    clean_entities.to_string()
}

fn decode(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
        .replace("&#8220;", "\u{201C}")
        .replace("&#8221;", "\u{201D}")
        .replace("&#8217;", "\u{2019}")
        .replace("&#8211;", "\u{2013}")
        .replace("&#8212;", "\u{2014}")
        .replace("&#8230;", "\u{2026}")
        .replace("&mdash;", "\u{2014}")
        .replace("&ndash;", "\u{2013}")
        .replace("&ldquo;", "\u{201C}")
        .replace("&rdquo;", "\u{201D}")
        .replace("&lsquo;", "\u{2018}")
        .replace("&rsquo;", "\u{2019}")
        .replace("&hellip;", "\u{2026}")
        .replace("&#160;", " ")
        .replace("&#8594;", "→")
        .replace("&#8592;", "←")
        .replace("&#8593;", "↑")
        .replace("&#8595;", "↓")
}

fn normalize(input: &str) -> String {
    WHITESPACE_REGEX.replace_all(input.trim(), " ").to_string()
}

fn format(input: &str) -> String {
    let patterns = [
        (r"\[\u{2026}\]", ""),
        (r"\[\.\.\.\]", ""),
        (r"Read More\.\.\..*$", ""),
        (r"Continue reading.*$", ""),
        (r"Click here.*$", ""),
        (r"More info.*$", ""),
        (r"\s*\.\.\.\s*$", ""),
        (r"^\s*-\s*", ""),
        (r"^\s*\*\s*", ""),
        (r"\{'[^']*'[^}]*\}", ""),
        (r"\([^)]*'[^']*'[^)]*\)", ""),
        (r"an\.\.\.$", ""),
        (r"<[^>]*Value[^>]*>", ""),
        (r"object at 0x[a-fA-F0-9]+", ""),
    ];

    let mut result = input.to_string();
    for (pattern, replacement) in patterns {
        if let Ok(regex) = Regex::new(pattern) {
            result = regex.replace_all(&result, replacement).to_string();
        }
    }

    result
}

pub fn title(entry: &feed_rs::model::Entry) -> String {
    entry
        .title
        .as_ref()
        .map(|t| clean(&t.content))
        .unwrap_or_else(|| "Untitled".to_string())
}

pub fn description(entry: &feed_rs::model::Entry) -> String {
    let description = entry
        .summary
        .as_ref()
        .map(|s| clean(&s.content))
        .or_else(|| {
            entry
                .content
                .as_ref()
                .and_then(|c| c.body.as_ref().map(|body| clean(body)))
        })
        .unwrap_or_else(|| "No description available.".to_string());

    if description.len() > 1800 {
        let truncated = &description[..1800];
        if let Some(last_sentence) = truncated.rfind('.') {
            if last_sentence > 1400 {
                return format!("{}.", &truncated[..last_sentence]);
            }
        }
        if let Some(last_space) = truncated.rfind(' ') {
            if last_space > 1400 {
                return format!("{}…", &truncated[..last_space]);
            }
        }
        format!("{}…", &description[..1797])
    } else {
        description
    }
}

pub fn truncate(text: &str, max_length: usize) -> String {
    if text.len() <= max_length {
        return text.to_string();
    }

    let truncated = &text[..max_length];

    if let Some(last_sentence) = truncated.rfind('.') {
        if last_sentence > max_length * 3 / 4 {
            return format!("{}.", &truncated[..last_sentence]);
        }
    }

    if let Some(last_space) = truncated.rfind(' ') {
        if last_space > max_length * 3 / 4 {
            return format!("{}…", &truncated[..last_space]);
        }
    }

    if let Some(last_punct) = truncated.rfind(&['.', '!', '?', ',', ';']) {
        if last_punct > max_length * 3 / 4 {
            return format!("{}…", &truncated[..=last_punct]);
        }
    }

    format!("{}…", &truncated[..max_length.saturating_sub(1)])
}
