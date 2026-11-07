use std::{collections::HashSet, time::Duration};

use anyhow::Context;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use regex::Regex;
use reqwest::{header, Client, Url};
use serde_json::Value;

use crate::{
    models::{Download, DownloadLink, Episode, Meta, Mirror, Movie, SearchMode, Stream},
    regex,
    source::{embed, Source},
    util::{clean_text, html_to_text, unescape_js_url},
};

const BASE: &str = "https://otakudesu.blog";
const AJAX: &str = "https://otakudesu.blog/wp-admin/admin-ajax.php";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

pub struct Otakudesu {
    client: Client,
}

impl Otakudesu {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(35))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { client }
    }

    async fn get_html(&self, url: &str) -> anyhow::Result<String> {
        let response = self
            .client
            .get(url)
            .header(header::REFERER, BASE)
            .send()
            .await
            .with_context(|| format!("Gagal menghubungi {url}"))?
            .error_for_status()
            .with_context(|| format!("Sumber menolak permintaan ke {url}"))?;

        let bytes = response.bytes().await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn resolve_mirror(&self, mirror: &Mirror) -> anyhow::Result<Option<String>> {
        if mirror.nonce_action.is_empty() || mirror.mirror_action.is_empty() {
            return Ok(None);
        }

        let nonce_response = self
            .client
            .post(AJAX)
            .header(header::REFERER, &mirror.referer)
            .header(header::ORIGIN, BASE)
            .header(
                header::CONTENT_TYPE,
                "application/x-www-form-urlencoded; charset=UTF-8",
            )
            .header("X-Requested-With", "XMLHttpRequest")
            .form(&[("action", mirror.nonce_action.as_str())])
            .send()
            .await?;

        let nonce_json: Value = nonce_response.json().await?;
        let Some(nonce) = nonce_json["data"].as_str() else {
            return Ok(None);
        };

        let form = vec![
            ("id".to_string(), mirror.id.to_string()),
            ("i".to_string(), mirror.index.to_string()),
            ("q".to_string(), mirror.quality.clone()),
            ("nonce".to_string(), nonce.to_string()),
            ("action".to_string(), mirror.mirror_action.clone()),
        ];

        let response = self
            .client
            .post(AJAX)
            .header(header::REFERER, &mirror.referer)
            .header(header::ORIGIN, BASE)
            .header(
                header::CONTENT_TYPE,
                "application/x-www-form-urlencoded; charset=UTF-8",
            )
            .header("X-Requested-With", "XMLHttpRequest")
            .form(&form)
            .send()
            .await?;

        let json: Value = response.json().await?;
        let Some(data) = json["data"].as_str() else {
            return Ok(None);
        };

        let decoded = STANDARD.decode(data)?;
        let html = String::from_utf8_lossy(&decoded);

        Ok(extract_iframe(&html))
    }
}

impl Default for Otakudesu {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Source for Otakudesu {
    fn name(&self) -> &'static str {
        "Otakudesu"
    }

    async fn search(&mut self, query: &str, mode: SearchMode) -> anyhow::Result<Vec<Movie>> {
        let mut url = Url::parse(&format!("{BASE}/"))?;
        url.query_pairs_mut()
            .append_pair("s", query)
            .append_pair("post_type", "anime");

        let html = self.get_html(url.as_str()).await?;
        let Some(block) = regex!(r#"(?s)<ul class="chivsrc">(.*?)</ul>"#).captures(&html) else {
            return Ok(vec![]);
        };

        let mut movies = vec![];
        for item in regex!(r#"(?s)<li[^>]*>(.*?)</li>"#).captures_iter(&block[1]) {
            let item = &item[1];

            let Some(title_cap) =
                regex!(r#"(?s)<h2[^>]*>\s*<a[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).captures(item)
            else {
                continue;
            };

            let title = clean_text(&title_cap[2]);
            if title.is_empty() {
                continue;
            }

            let status = cap_clean(regex!(r#"(?s)<b>Status</b>\s*:\s*([^<]+)"#), item);
            let status_lower = status.as_deref().unwrap_or_default().to_lowercase();

            if mode == SearchMode::Ongoing && !status_lower.contains("ongoing") {
                continue;
            }
            if mode == SearchMode::Completed && !status_lower.contains("completed") {
                continue;
            }

            let rating = cap_clean(regex!(r#"(?s)<b>Rating</b>\s*:\s*([^<]+)"#), item);

            let mut info_parts = vec![];
            if let Some(status) = status {
                info_parts.push(status);
            }
            if let Some(rating) = rating {
                info_parts.push(format!("★ {rating}"));
            }

            movies.push(Movie {
                id: normalize_url(&title_cap[1]),
                title,
                info: if info_parts.is_empty() {
                    None
                } else {
                    Some(info_parts.join(" • "))
                },
                poster_url: None,
            });
        }

        Ok(movies)
    }

    async fn episodes(&self, movie: &Movie) -> anyhow::Result<(Vec<Episode>, Meta)> {
        let html = self.get_html(&movie.id).await?;
        let mut meta = Meta::default();

        meta.thumb_url = regex!(r#"(?s)<div class=['"]fotoanime['"].*?<img[^>]+src="([^"]+)""#)
            .captures(&html)
            .map(|cap| unescape_js_url(&cap[1]));

        if let Some(info) = regex!(r#"(?s)<div class="infozingle">(.*?)</div>"#).captures(&html) {
            for paragraph in regex!(r#"(?s)<p[^>]*>(.*?)</p>"#).captures_iter(&info[1]) {
                let text = clean_text(&paragraph[1]);
                if let Some((key, value)) = text.split_once(':') {
                    let key = key.trim();
                    let value = value.trim();
                    if !key.is_empty() && !value.is_empty() {
                        meta.data.push((key.to_string(), value.to_string()));
                    }
                }
            }
        }

        if let Some(synopsis) =
            regex!(r#"(?s)<div class=['"]sinopc['"]>(.*?)</div>"#).captures(&html)
        {
            let text = html_to_text(&synopsis[1]);
            if !text.is_empty() {
                meta.synopsis = Some(text);
            }
        }

        let mut episodes = vec![];
        let mut seen = HashSet::new();

        for cap in regex!(r#"(?s)<a[^>]+href="([^"]+)"[^>]*>(.*?)</a>"#).captures_iter(&html) {
            let href = &cap[1];
            if !href.contains("/episode/") {
                continue;
            }

            let id = normalize_url(href);
            if !seen.insert(id.clone()) {
                continue;
            }

            let title = clean_text(&cap[2]);
            if title.is_empty() {
                continue;
            }

            let number = episode_number(&title);
            episodes.push(Episode {
                id,
                title,
                number,
                is_series: true,
            });
        }

        // Kalau ini halaman film / OVA dengan player langsung, jadikan satu episode.
        if episodes.is_empty() && html.contains("pembed") {
            episodes.push(Episode {
                id: movie.id.clone(),
                title: movie.title.clone(),
                number: Some(1),
                is_series: false,
            });
        }

        meta.downloads = parse_downloads(&html);

        Ok((episodes, meta))
    }

    async fn streams(&self, episode: &Episode) -> anyhow::Result<Vec<Stream>> {
        let html = self.get_html(&episode.id).await?;
        let (nonce_action, mirror_action) = extract_actions(&html);

        let mut streams = vec![];

        if let Some(cap) =
            regex!(r#"(?s)<div[^>]+id=['"]?pembed['"]?[^>]*>.*?<iframe[^>]+src="([^"]+)""#)
                .captures(&html)
        {
            let label = match default_quality(&html) {
                Some(quality) => format!("Auto • {quality} (default)"),
                None => "Auto • default".to_string(),
            };

            streams.push(Stream {
                title: label,
                url: Some(unescape_js_url(&cap[1])),
                referer: Some(episode.id.clone()),
                mirror: None,
            });
        }

        let mut mirrors = vec![];
        for ul in regex!(r#"(?s)<ul class="(m\d+p)"[^>]*>(.*?)</ul>"#).captures_iter(&html) {
            let quality = ul[1].trim_start_matches('m').to_string();

            for li in regex!(r##"(?s)<li>\s*<a href="#" data-content="([^"]+)"[^>]*>(.*?)</a>"##)
                .captures_iter(&ul[2])
            {
                let decoded = match STANDARD.decode(&li[1]) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let json: Value = match serde_json::from_slice(&decoded) {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                let id = json["id"].as_u64().unwrap_or(0);
                if id == 0 {
                    continue;
                }
                let index = json["i"].as_u64().unwrap_or(0);
                let quality = json["q"].as_str().unwrap_or(&quality).to_string();
                let server = clean_text(&li[2]);

                let title = if server.is_empty() {
                    quality.clone()
                } else {
                    format!("{quality} • {server}")
                };

                mirrors.push(Stream {
                    title,
                    url: None,
                    referer: Some(episode.id.clone()),
                    mirror: Some(Mirror {
                        id,
                        index,
                        quality,
                        referer: episode.id.clone(),
                        nonce_action: nonce_action.clone(),
                        mirror_action: mirror_action.clone(),
                    }),
                });
            }
        }

        mirrors.sort_by_key(|stream| {
            let quality = stream
                .mirror
                .as_ref()
                .map(|mirror| quality_rank(&mirror.quality))
                .unwrap_or(0);
            (std::cmp::Reverse(quality), server_rank(&stream.title))
        });
        streams.extend(mirrors);

        Ok(streams)
    }

    async fn resolve(&self, stream: &Stream) -> anyhow::Result<Option<String>> {
        let embed = if let Some(mirror) = &stream.mirror {
            self.resolve_mirror(mirror).await?
        } else {
            stream.url.clone()
        };

        let Some(embed) = embed else {
            return Ok(None);
        };

        Ok(embed::resolve(&self.client, &embed, &BASE).await)
    }
}

fn cap_clean(re: &Regex, input: &str) -> Option<String> {
    re.captures(input)
        .map(|cap| clean_text(&cap[1]))
        .filter(|value| !value.is_empty())
}

fn normalize_url(href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        format!("{BASE}{href}")
    } else {
        format!("{BASE}/{href}")
    }
}

fn parse_downloads(html: &str) -> Vec<Download> {
    let Some(block) = regex!(r#"(?s)<div class="download">(.*?)</div>"#).captures(html) else {
        return vec![];
    };

    let mut downloads = vec![];

    for item in regex!(r#"(?s)<li>(.*?)</li>"#).captures_iter(&block[1]) {
        let item = &item[1];

        let quality = regex!(r#"(?s)<strong>(.*?)</strong>"#)
            .captures(item)
            .map(|cap| clean_text(&cap[1]))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Download".to_string());

        let size = regex!(r#"(?s)<i>(.*?)</i>"#)
            .captures(item)
            .map(|cap| clean_text(&cap[1]))
            .filter(|value| !value.is_empty());

        let mut links = vec![];
        for link in regex!(r#"(?s)<a[^>]+href="([^"]+)"[^>]*>(.*?)</a>"#).captures_iter(item) {
            links.push(DownloadLink {
                url: link[1].to_string(),
                label: clean_text(&link[2]),
            });
        }

        if !links.is_empty() {
            downloads.push(Download {
                quality,
                size,
                links,
            });
        }
    }

    downloads
}

fn extract_actions(html: &str) -> (String, String) {
    let nonce = regex!(r#"data:\{\s*action\s*:\s*["']([a-f0-9]{16,})["']"#)
        .captures(html)
        .map(|cap| cap[1].to_string());

    let mirror = regex!(r#"nonce\s*:\s*[^,]+,\s*action\s*:\s*["']([a-f0-9]{16,})["']"#)
        .captures(html)
        .map(|cap| cap[1].to_string());

    if let (Some(nonce), Some(mirror)) = (nonce.as_ref(), mirror.as_ref()) {
        return (nonce.clone(), mirror.clone());
    }

    // Cadangan: ambil action unik sesuai urutan kemunculannya.
    let mut unique: Vec<String> = vec![];
    for cap in regex!(r#"action\s*:\s*["']([a-f0-9]{16,})["']"#).captures_iter(html) {
        let action = cap[1].to_string();
        if !unique.contains(&action) {
            unique.push(action);
        }
    }

    (
        nonce.or_else(|| unique.get(1).cloned()).unwrap_or_default(),
        mirror
            .or_else(|| unique.first().cloned())
            .unwrap_or_default(),
    )
}

fn default_quality(html: &str) -> Option<String> {
    let index = html.find("data-default=\"true\"")?;
    let before = &html[..index];

    regex!(r#"<ul class="(m\d+p)"#)
        .captures_iter(before)
        .last()
        .map(|cap| cap[1].trim_start_matches('m').to_string())
}

fn quality_rank(quality: &str) -> u32 {
    quality
        .trim_end_matches('p')
        .parse::<u32>()
        .unwrap_or_default()
}

fn server_rank(title: &str) -> u8 {
    let title = title.to_lowercase();

    if title.contains("odcdn") {
        0
    } else if title.contains("odstream") {
        1
    } else if title.contains("ondesu") {
        2
    } else if title.contains("filedon") {
        3
    } else if title.contains("vidhide") {
        4
    } else {
        5
    }
}

fn extract_iframe(html: &str) -> Option<String> {
    regex!(r#"(?s)<iframe[^>]+src=["']([^"']+)["']"#)
        .captures(html)
        .map(|cap| unescape_js_url(&cap[1]))
}

fn episode_number(title: &str) -> Option<u32> {
    regex!(r#"(?i)episode\s*(\d+)"#)
        .captures(title)
        .and_then(|cap| cap[1].parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_actions_from_minified_script() {
        let html = r#"window.__x__nonce?$.ajax("x",{data:{...e,nonce:window.__x__nonce,action:"2a3505c93b0035d3f455df82bf976b84"}}):$.ajax("x",{data:{action:"aa1208d27f29ca340c92c66d1926f13f"}})"#;

        let (nonce, mirror) = extract_actions(html);
        assert_eq!(nonce, "aa1208d27f29ca340c92c66d1926f13f");
        assert_eq!(mirror, "2a3505c93b0035d3f455df82bf976b84");
    }

    #[test]
    fn parses_default_quality() {
        let html = r##"<ul class="m360p"><li><a href="#">x</a></li></ul>
                     <ul class="m480p"><li><a data-default="true">x</a></li></ul>"##;

        assert_eq!(default_quality(html).as_deref(), Some("480p"));
    }
}
