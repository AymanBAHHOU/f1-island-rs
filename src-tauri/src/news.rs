//! F1 headlines from public RSS feeds, merged and newest first.
//! (Port of the Electron app's `src/news.js`.)

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const FEEDS: [(&str, &str); 4] = [
    ("Motorsport.com", "https://www.motorsport.com/rss/f1/news/"),
    (
        "The Race",
        "https://www.the-race.com/category/formula-1/feed/",
    ),
    ("Autosport", "https://www.autosport.com/rss/f1/news/"),
    (
        "BBC Sport",
        "https://feeds.bbci.co.uk/sport/formula1/rss.xml",
    ),
];
pub const MAX_ITEMS: usize = 12;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub title: String,
    pub link: String,
    pub source: String,
    /// ISO time, or `None` if the feed's date could not be read.
    pub date: Option<String>,
    pub summary: String,
}

/// The text between `<name ...>` and `</name>` in `block`, if the tag is there.
fn tag_text<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let mut from = 0;
    while let Some(found) = block[from..].find(&open) {
        let start = from + found + open.len();
        // `<link>` and `<link rel=..>` count; `<linkage>` does not.
        match block[start..].chars().next() {
            Some('>' | ' ' | '\n' | '\t' | '\r') => {
                let body_start = block[start..].find('>')? + start + 1;
                let close = format!("</{name}>");
                let body_end = block[body_start..].find(&close)? + body_start;
                return Some(&block[body_start..body_end]);
            }
            _ => from = start,
        }
    }
    None
}

fn decode_entity(name: &str) -> Option<String> {
    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        return u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .map(String::from);
    }
    if let Some(dec) = name.strip_prefix('#') {
        return dec
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(String::from);
    }
    let text = match name.to_ascii_lowercase().as_str() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "hellip" => "…",
        "rsquo" => "’",
        "lsquo" => "‘",
        "rdquo" => "”",
        "ldquo" => "“",
        "ndash" => "–",
        "mdash" => "—",
        _ => return None,
    };
    Some(text.to_string())
}

/// Plain text from a feed field: unwraps CDATA, drops tags, decodes entities, tidies spaces.
pub fn clean(raw: &str) -> String {
    // 1. CDATA wrappers
    let mut text = String::new();
    let mut rest = raw;
    while let Some(i) = rest.find("<![CDATA[") {
        text.push_str(&rest[..i]);
        rest = &rest[i + 9..];
        match rest.find("]]>") {
            Some(j) => {
                text.push_str(&rest[..j]);
                rest = &rest[j + 3..];
            }
            None => {
                text.push_str(rest);
                rest = "";
            }
        }
    }
    text.push_str(rest);

    // 2. tags become spaces
    let mut stripped = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                stripped.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => stripped.push(c),
            _ => {}
        }
    }

    // 3. entities
    let mut out = String::new();
    let mut rest = stripped.as_str();
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest[1..]
            .find(';')
            .filter(|&n| n <= 10)
            .and_then(|n| decode_entity(&rest[1..1 + n]).map(|t| (t, n)));
        match decoded {
            Some((t, n)) => {
                out.push_str(&t);
                rest = &rest[n + 2..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn parse_feed(xml: &str, source: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<item") {
        rest = &rest[i..];
        let Some(end) = rest.find("</item>") else {
            break;
        };
        let block = &rest[..end];
        rest = &rest[end + 7..];

        let field = |name| tag_text(block, name).map(clean).unwrap_or_default();
        let title = field("title");
        let link = field("link");
        if title.is_empty() || !(link.starts_with("http://") || link.starts_with("https://")) {
            continue;
        }
        let date = DateTime::parse_from_rfc2822(&field("pubDate"))
            .ok()
            .map(|d| {
                d.with_timezone(&Utc)
                    .to_rfc3339_opts(SecondsFormat::Millis, true)
            });
        let mut summary = field("description");
        if summary.chars().count() > 320 {
            let cut: String = summary.chars().take(317).collect();
            // back up to the last whole word
            let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
            summary = format!("{cut}…");
        }
        items.push(Item {
            title,
            link,
            source: source.to_string(),
            date,
            summary,
        });
    }
    items
}

/// Newest first, one entry per headline (the same story often comes from several feeds).
pub fn merge_feeds(mut items: Vec<Item>) -> Vec<Item> {
    items.sort_by(|a, b| b.date.cmp(&a.date));
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| {
            let key: String = item
                .title
                .to_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect();
            seen.insert(key)
        })
        .take(MAX_ITEMS)
        .collect()
}

const TTL: Duration = Duration::from_secs(15 * 60);

/// Fetches the feeds and remembers the result for a while. Handed to Tauri with `.manage`.
pub struct News {
    http: reqwest::Client,
    cache: Mutex<Option<(Instant, Vec<Item>)>>,
}

impl Default for News {
    fn default() -> Self {
        News {
            http: reqwest::Client::builder()
                .user_agent("F1Island/0.1")
                .timeout(Duration::from_secs(10))
                .build()
                .expect("a plain HTTP client can always be built"),
            cache: Mutex::new(None),
        }
    }
}

impl News {
    pub async fn headlines(&self) -> Result<Vec<Item>, String> {
        if let Some((at, items)) = &*self.cache.lock().unwrap() {
            if at.elapsed() < TTL {
                return Ok(items.clone());
            }
        }
        // All feeds at once; one being down only costs its own headlines.
        let fetches = FEEDS.iter().map(|(source, url)| async move {
            let body = self
                .http
                .get(*url)
                .send()
                .await
                .ok()?
                .error_for_status()
                .ok()?
                .text()
                .await
                .ok()?;
            Some(parse_feed(&body, source))
        });
        let all: Vec<Item> = futures_util::future::join_all(fetches)
            .await
            .into_iter()
            .flatten()
            .flatten()
            .collect();
        let items = merge_feeds(all);

        let mut cache = self.cache.lock().unwrap();
        if items.is_empty() {
            // stale headlines beat none
            return match &*cache {
                Some((_, old)) => Ok(old.clone()),
                None => Err("No headlines available right now".to_string()),
            };
        }
        *cache = Some((Instant::now(), items.clone()));
        Ok(items)
    }

    /// Only links we handed out may be opened.
    pub fn is_known_link(&self, url: &str) -> bool {
        self.cache
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|(_, items)| items.iter().any(|i| i.link == url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<rss><channel>
      <item><title><![CDATA[Verstappen &amp; Norris: &#8220;close&#8221;]]></title>
        <link>https://example.com/a</link>
        <pubDate>Sun, 25 Oct 2026 10:00:00 +0000</pubDate>
        <description><![CDATA[<p>Hello&nbsp;world</p>]]></description></item>
      <item><title>No link</title><link>nope</link></item>
      <item><title>Older</title><link>https://example.com/b</link>
        <pubDate>Sat, 24 Oct 2026 10:00:00 GMT</pubDate></item>
    </channel></rss>"#;

    #[test]
    fn parses_items_and_skips_bad_ones() {
        let items = parse_feed(XML, "Test");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Verstappen & Norris: “close”");
        assert_eq!(items[0].summary, "Hello world");
        assert_eq!(items[0].date.as_deref(), Some("2026-10-25T10:00:00.000Z"));
        assert_eq!(items[1].link, "https://example.com/b");
    }

    #[test]
    fn long_summaries_are_cut_at_a_word() {
        let long = "word ".repeat(100);
        let xml = format!(
            "<item><title>T</title><link>https://x.y</link><description>{long}</description></item>"
        );
        let summary = &parse_feed(&xml, "S")[0].summary;
        assert!(summary.ends_with("word…"));
        assert!(summary.chars().count() <= 318);
    }

    #[test]
    fn merges_newest_first_without_duplicates() {
        let mut a = parse_feed(XML, "A");
        let b = parse_feed(
            "<item><title>VERSTAPPEN &amp; Norris: “close”!</title><link>https://other/1</link></item>",
            "B",
        );
        a.extend(b);
        let merged = merge_feeds(a);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].source, "A");
    }
}
