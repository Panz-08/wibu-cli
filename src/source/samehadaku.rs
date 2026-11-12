use std::{collections::HashSet, time::Duration};

use anyhow::Context;
use async_trait::async_trait;
use reqwest::{header, Client};
use serde_json::Value;

use crate::{
    models::{Episode, Meta, Movie, SearchMode, Stream},
    regex,
    source::{embed, Source},
    util::unescape_js_url,
};

const DEFAULT_BASE: &str = "https://samehadaku.today";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

/// Sumber anime cadangan ketiga.
///
/// Samehadaku memakai Next.js. Pencarian dilayani JSON API, halaman series
/// memuat tautan episode, dan halaman episode menyimpan tautan streaming
/// Blogger di payload RSC. URL Blogger di-resolve oleh `embed` menjadi MP4
/// `googlevideo` yang bisa diputar MPV selama User-Agent browser dipakai.
pub struct Samehadaku {
    client: Client,
    base: String,
}

impl Samehadaku {
    pub fn new() -> Self {
        let base = std::env::var("WIBU_SAMEHADAKU_BASE")
            .ok()
            .filter(|value| value.starts_with("http"))
            .unwrap_or_else(|| DEFAULT_BASE.to_string())
            .trim_end_matches('/')
            .to_string();

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::limited(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .gzip(true)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { client, base }
    }

    async fn get_text(&self, url: &str) -> anyhow::Result<String> {
        let mut last_error = None;

        for attempt in 0..3u64 {
            let request = self
                .client
                .get(url)
                .header(header::REFERER, &self.base)
                .header(header::ACCEPT_LANGUAGE, "id-ID,id;q=0.9,en;q=0.8")
                .header(
                    header::ACCEPT,
                    "text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8",
                );

            match request.send().await {
                Ok(response) if response.status().is_success() => match response.text().await {
                    Ok(body) => return Ok(body),
                    Err(error) => last_error = Some(error.into()),
                },
                Ok(response) => {
                    last_error = Some(anyhow::anyhow!(
                        "{} menolak permintaan ({})",
                        url,
                        response.status()
                    ));
                    if matches!(response.status().as_u16(), 404 | 410) {
                        break;
                    }
                }
                Err(error) => last_error = Some(error.into()),
            }

            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(400 * (attempt + 1))).await;
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Gagal menghubungi {url}")))
            .with_context(|| format!("Gagal memuat {url}"))
    }
}

impl Default for Samehadaku {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Source for Samehadaku {
    fn name(&self) -> &'static str {
        "Samehadaku"
    }

    async fn search(&mut self, query: &str, _mode: SearchMode) -> anyhow::Result<Vec<Movie>> {
        let query = query.trim();
        if query.is_empty() {
            anyhow::bail!("Samehadaku butuh kata kunci pencarian.");
        }

        let url = format!("{}/api/search?q={}&page=1", self.base, urlencode(query));
        let body = self.get_text(&url).await?;
        let value: Value = serde_json::from_str(&body).with_context(|| {
            format!("Samehadaku mengembalikan data yang tidak valid untuk '{query}'")
        })?;

        let mut movies = vec![];
        let mut seen = HashSet::new();

        for item in value["data"].as_array().into_iter().flatten() {
            let Some(slug) = item["slug"].as_str() else {
                continue;
            };
            let Some(title) = item["title"].as_str() else {
                continue;
            };

            let id = format!("{}/series/{}", self.base, slug);
            if !seen.insert(id.clone()) {
                continue;
            }

            let kind = item["type"].as_str().unwrap_or("TV");
            let status = item["status"].as_str().unwrap_or("");
            let episode = item["latest_episode"]
                .as_str()
                .filter(|value| !value.is_empty())
                .map(|value| format!("Ep {value}"));

            let info = [Some(kind), Some(status), episode.as_deref()]
                .into_iter()
                .flatten()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" • ");

            movies.push(Movie {
                id,
                title: title.trim().to_string(),
                info: Some(info),
                poster_url: item["poster"].as_str().map(ToOwned::to_owned),
            });
        }

        Ok(movies)
    }

    async fn episodes(&self, movie: &Movie) -> anyhow::Result<(Vec<Episode>, Meta)> {
        let html = self.get_text(&movie.id).await?;
        let mut meta = Meta::default();

        meta.thumb_url =
            regex!(r#"(?s)<meta[^>]+property=["']og:image["'][^>]+content=["']([^"']+)["']"#)
                .captures(&html)
                .map(|cap| unescape_js_url(&cap[1]));

        let mut episodes = parse_watch_links(&html, &self.base);
        episodes.sort_by_key(|episode: &Episode| episode.number.unwrap_or(u32::MAX));

        if episodes.is_empty() {
            // Sebagian judul bertipe film tidak punya daftar episode. Halaman
            // series tetap bisa berisi satu tautan tonton langsung.
            if let Some(url) = first_watch_link(&html, &self.base) {
                episodes.push(Episode {
                    id: url,
                    title: movie.title.clone(),
                    number: None,
                    is_series: false,
                });
            }
        }

        if let Some(status) =
            regex!(r#"(?s)(?:Status|status)["']?\s*[:=]\s*["']?([A-Za-z ]{3,20})["']?"#)
                .captures(&html)
        {
            meta.data.push(("Status".to_string(), cap_text(&status[1])));
        }

        meta.data
            .push(("Episode".to_string(), episodes.len().to_string()));
        meta.data
            .push(("Sumber".to_string(), "Samehadaku".to_string()));

        Ok((episodes, meta))
    }

    async fn streams(&self, episode: &Episode) -> anyhow::Result<Vec<Stream>> {
        let html = self.get_text(&episode.id).await?;

        let mut streams = vec![];
        let mut seen = HashSet::new();

        if let Some(url) = extract_streaming_video(&html) {
            if seen.insert(url.clone()) {
                streams.push(Stream {
                    title: "Blogger • MP4".to_string(),
                    url: Some(url),
                    referer: Some(episode.id.clone()),
                    mirror: None,
                });
            }
        }

        Ok(streams)
    }

    async fn resolve(&self, stream: &Stream) -> anyhow::Result<Option<String>> {
        let Some(url) = &stream.url else {
            return Ok(None);
        };

        let referer = stream.referer.as_deref().unwrap_or(self.base.as_str());
        Ok(embed::resolve(&self.client, url, referer).await)
    }
}

fn parse_watch_links(html: &str, base: &str) -> Vec<Episode> {
    let mut episodes = vec![];
    let mut seen = HashSet::new();

    for cap in regex!(r#"(?s)href=["'](/watch/[^"']+)["']"#).captures_iter(html) {
        let href = unescape_js_url(&cap[1]);
        let Some(number) = regex!(r#"(?i)-episode-(\d+)(?:-|$)"#)
            .captures(&href)
            .and_then(|cap| cap[1].parse::<u32>().ok())
        else {
            continue;
        };

        let id = format!("{}{}", base, href);
        if !seen.insert(id.clone()) {
            continue;
        }

        episodes.push(Episode {
            id,
            title: format!("Episode {number}"),
            number: Some(number),
            is_series: true,
        });
    }

    episodes
}

fn first_watch_link(html: &str, base: &str) -> Option<String> {
    regex!(r#"(?s)href=["'](/watch/[^"']+)["']"#)
        .captures(html)
        .map(|cap| format!("{}{}", base, unescape_js_url(&cap[1])))
}

/// Payload RSC Samehadaku meng-escape tanda kutip, jadi normalisasi dulu
/// sebelum mengambil `streamingLinks.video`.
fn extract_streaming_video(html: &str) -> Option<String> {
    let normalized = html.replace("\\\"", "\"");

    regex!(r#"(?s)"streamingLinks"\s*:\s*\{[^{}]*?"video"\s*:\s*"([^"]+)""#)
        .captures(&normalized)
        .map(|cap| cap[1].to_string())
        .or_else(|| {
            regex!(r#"(?s)"video"\s*:\s*"(https?://[^"]+)""#)
                .captures(&normalized)
                .map(|cap| cap[1].to_string())
        })
        .map(|url| unescape_js_url(&url))
        .filter(|url| url.starts_with("http"))
}

fn cap_text(input: &str) -> String {
    crate::util::clean_text(input)
}

fn urlencode(input: &str) -> String {
    input
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_results() {
        let json = r#"{
          "data": [
            {"title":"Sousou no Frieren","slug":"sousou-no-frieren","poster":"https://img/f.jpg","status":"Completed","type":"TV","latest_episode":"28"},
            {"title":"Sousou no Frieren","slug":"sousou-no-frieren","poster":"https://img/f.jpg","status":"Completed","type":"TV"}
          ]
        }"#;

        let value: Value = serde_json::from_str(json).unwrap();
        let items = value["data"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["slug"], "sousou-no-frieren");
    }

    #[test]
    fn parses_watch_links_in_numeric_order() {
        let html = r#"
          <a href="/watch/frieren-episode-10-subtitle-indonesia">Ep 10</a>
          <a href="/watch/frieren-episode-1-subtitle-indonesia">Ep 1</a>
          <a href="/watch/frieren-episode-2-subtitle-indonesia">Ep 2</a>
          <a href="/watch/frieren-episode-2-subtitle-indonesia">Ep 2</a>
          <a href="/series/frieren">Semua Episode</a>
        "#;

        let mut episodes = parse_watch_links(html, "https://samehadaku.today");
        episodes.sort_by_key(|episode| episode.number.unwrap_or(u32::MAX));

        assert_eq!(episodes.len(), 3);
        assert_eq!(episodes[0].number, Some(1));
        assert_eq!(episodes[1].number, Some(2));
        assert_eq!(episodes[2].number, Some(10));
        assert!(episodes[0].is_series);
    }

    #[test]
    fn extracts_blogger_video_from_rsc_payload() {
        let html = r#"self.__next_f.push([1,"{\"streamingLinks\":{\"video\":\"https://www.blogger.com/video.g?token=AD6v5abc\"},\"title\":\"Episode 1\"}"])"#;

        assert_eq!(
            extract_streaming_video(html).as_deref(),
            Some("https://www.blogger.com/video.g?token=AD6v5abc")
        );
    }

    #[test]
    fn encodes_search_query() {
        assert_eq!(urlencode("sousou no frieren"), "sousou%20no%20frieren");
    }
}
