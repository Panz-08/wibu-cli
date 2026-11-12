use std::{collections::HashSet, time::Duration};

use anyhow::Context;
use async_trait::async_trait;
use html_escape::decode_html_entities;
use reqwest::{
    header::{self, HeaderMap},
    Client, Url,
};
use serde_json::Value;

use crate::{
    models::{Episode, Listing, Meta, Movie, SearchMode, Stream},
    regex,
    source::{embed, Source},
    util::{clean_text, html_to_text, unescape_js_url},
};

const DEFAULT_BASE: &str = "https://layarkaca21.health";
const JAPAN_COUNTRY_FALLBACK: u64 = 1096;
const JAPAN_SERIES_CATEGORY: &str = "3535";
const PER_PAGE: u32 = 24;

/// Film Jepang dan series/drama Jepang dari LK21.
///
/// Katalog memakai REST WordPress (`/wp-json/wp/v2/posts` untuk film dan
/// `/tv` untuk series) dengan filter negara Japan. Halaman detail dan player
/// tetap diambil dari HTML biasa. Player LK21 dimuat lewat admin-ajax, lalu
/// iframe-nya di-resolve oleh `embed`.
pub struct Lk21 {
    client: Client,
    base: String,
    country_id: Option<u64>,
}

impl Lk21 {
    pub fn new() -> Self {
        let base = std::env::var("WIBU_LK21_BASE")
            .ok()
            .filter(|value| value.starts_with("http"))
            .unwrap_or_else(|| DEFAULT_BASE.to_string())
            .trim_end_matches('/')
            .to_string();

        let client = Client::builder()
            .user_agent(embed::USER_AGENT)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::limited(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .gzip(true)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            base,
            country_id: None,
        }
    }

    async fn get_html(&self, url: &str) -> anyhow::Result<String> {
        let mut last_error = None;

        for attempt in 0..3u64 {
            let request = self
                .client
                .get(url)
                .header(header::REFERER, &self.base)
                .header(header::ACCEPT_LANGUAGE, "id-ID,id;q=0.9,en;q=0.8")
                .header(
                    header::ACCEPT,
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
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

    async fn get_json(&self, url: &str) -> anyhow::Result<(Value, HeaderMap)> {
        let mut last_error = None;

        for attempt in 0..3u64 {
            let request = self
                .client
                .get(url)
                .header(header::REFERER, &self.base)
                .header(header::ACCEPT, "application/json,text/plain,*/*");

            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    let headers = response.headers().clone();
                    match response.json::<Value>().await {
                        Ok(value) => return Ok((value, headers)),
                        Err(error) => last_error = Some(error.into()),
                    }
                }
                Ok(response) => {
                    last_error = Some(anyhow::anyhow!(
                        "{} menolak permintaan ({})",
                        url,
                        response.status()
                    ));
                    if matches!(response.status().as_u16(), 400 | 404 | 410) {
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
            .with_context(|| format!("Gagal memuat katalog {url}"))
    }

    async fn get_ajax(&self, url: &str, referer: &str) -> anyhow::Result<String> {
        for attempt in 0..2u64 {
            let response = self
                .client
                .get(url)
                .header(header::REFERER, referer)
                .header("X-Requested-With", "XMLHttpRequest")
                .header(header::ACCEPT, "text/html,*/*")
                .send()
                .await;

            if let Ok(response) = response {
                if response.status().is_success() {
                    if let Ok(body) = response.text().await {
                        return Ok(body);
                    }
                }
            }

            if attempt == 0 {
                tokio::time::sleep(Duration::from_millis(350)).await;
            }
        }

        anyhow::bail!("Player LK21 tidak merespons untuk {referer}")
    }

    async fn country_id(&mut self) -> anyhow::Result<u64> {
        if let Some(id) = self.country_id {
            return Ok(id);
        }

        let url = format!(
            "{}/wp-json/wp/v2/muvicountry?slug=japan&_fields=id",
            self.base
        );
        let (value, _) = self.get_json(&url).await?;
        let id = value
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(Value::as_u64)
            .unwrap_or(JAPAN_COUNTRY_FALLBACK);

        self.country_id = Some(id);
        Ok(id)
    }

    fn rest_url(&self, kind: RestKind, country: u64, query: &str, page: u32) -> String {
        let Ok(mut url) = Url::parse(&format!("{}/wp-json/wp/v2/{}", self.base, kind.slug()))
        else {
            return format!(
                "{}/wp-json/wp/v2/{}?muvicountry={country}&per_page={PER_PAGE}&page={page}",
                self.base,
                kind.slug()
            );
        };

        {
            let mut pairs = url.query_pairs_mut();
            if kind == RestKind::Series {
                // Kategori `drama-jepang` berisi live-action Jepang. Filter
                // `muvicountry=1096` saja masih mencampur anime.
                pairs.append_pair("categories", JAPAN_SERIES_CATEGORY);
            } else {
                pairs.append_pair("muvicountry", &country.to_string());
            }
            pairs.append_pair("per_page", &PER_PAGE.to_string());
            pairs.append_pair("page", &page.to_string());
            pairs.append_pair("_embed", "wp:featuredmedia");
            pairs.append_pair("_fields", "id,link,title,type,class_list,_links,_embedded");

            if !query.trim().is_empty() {
                pairs.append_pair("search", query.trim());
            }
        }

        url.to_string()
    }

    pub async fn search_listing(
        &mut self,
        query: &str,
        mode: SearchMode,
    ) -> anyhow::Result<Listing> {
        let country = self.country_id().await?;

        let kinds: Vec<RestKind> = match mode {
            SearchMode::JapanMovie | SearchMode::Movie => vec![RestKind::Movie],
            SearchMode::JapanSeries | SearchMode::Ongoing | SearchMode::Completed => {
                vec![RestKind::Series]
            }
            _ => vec![RestKind::Movie, RestKind::Series],
        };

        if kinds.len() == 1 {
            let kind = kinds[0];
            let url = self.rest_url(kind, country, query, 1);
            let (value, headers) = self.get_json(&url).await?;
            let items = parse_rest_listing(&value, kind);
            return Ok(Listing {
                items,
                next: next_page_url(&url, &headers),
            });
        }

        let movie_url = self.rest_url(RestKind::Movie, country, query, 1);
        let series_url = self.rest_url(RestKind::Series, country, query, 1);
        let (movie_result, series_result) =
            tokio::join!(self.get_json(&movie_url), self.get_json(&series_url));

        let mut items = vec![];
        let mut errors = vec![];

        match movie_result {
            Ok((value, _)) => items.extend(parse_rest_listing(&value, RestKind::Movie)),
            Err(error) => errors.push(error),
        }
        match series_result {
            Ok((value, _)) => items.extend(parse_rest_listing(&value, RestKind::Series)),
            Err(error) => errors.push(error),
        }

        if items.is_empty() {
            if let Some(error) = errors.into_iter().next() {
                return Err(error);
            }
        }

        Ok(Listing { items, next: None })
    }

    pub async fn listing(&self, url: &str) -> anyhow::Result<Listing> {
        let (value, headers) = self.get_json(url).await?;
        let kind = if url.contains("/wp-json/wp/v2/tv") {
            RestKind::Series
        } else {
            RestKind::Movie
        };

        Ok(Listing {
            items: parse_rest_listing(&value, kind),
            next: next_page_url(url, &headers),
        })
    }
}

impl Default for Lk21 {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Source for Lk21 {
    fn name(&self) -> &'static str {
        "LK21"
    }

    async fn search(&mut self, query: &str, mode: SearchMode) -> anyhow::Result<Vec<Movie>> {
        Ok(self.search_listing(query, mode).await?.items)
    }

    async fn episodes(&self, movie: &Movie) -> anyhow::Result<(Vec<Episode>, Meta)> {
        let html = self.get_html(&movie.id).await?;
        Ok(parse_detail(&html, movie))
    }

    async fn streams(&self, episode: &Episode) -> anyhow::Result<Vec<Stream>> {
        let html = self.get_html(&episode.id).await?;
        let mut streams = vec![];
        let mut seen = HashSet::new();

        if let Some(post_id) = parse_player_id(&html) {
            for (index, tab) in ["p1", "p2", "p3"].iter().enumerate() {
                let url = format!(
                    "{}/wp-admin/admin-ajax.php?action=muvipro_player_content&tab={tab}&post_id={post_id}",
                    self.base
                );

                let Ok(body) = self.get_ajax(&url, &episode.id).await else {
                    continue;
                };

                for stream in extract_streams(&body, &episode.id) {
                    if seen.insert(stream.url.clone()) {
                        let mut stream = stream;
                        stream.title = format!("Server {} • {}", index + 1, stream.title);
                        streams.push(stream);
                    }
                }
            }
        }

        if streams.is_empty() {
            for stream in extract_streams(&html, &episode.id) {
                if seen.insert(stream.url.clone()) {
                    streams.push(stream);
                }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestKind {
    Movie,
    Series,
}

impl RestKind {
    fn slug(self) -> &'static str {
        match self {
            RestKind::Movie => "posts",
            RestKind::Series => "tv",
        }
    }

    fn label(self) -> &'static str {
        match self {
            RestKind::Movie => "Film",
            RestKind::Series => "Series",
        }
    }
}

fn parse_rest_listing(value: &Value, kind: RestKind) -> Vec<Movie> {
    let mut items = vec![];
    let mut seen = HashSet::new();

    let Some(entries) = value.as_array() else {
        return items;
    };

    for entry in entries {
        if is_anime_entry(entry) {
            continue;
        }

        let Some(movie) = parse_rest_movie(entry, kind) else {
            continue;
        };

        if seen.insert(movie.id.clone()) {
            items.push(movie);
        }
    }

    items
}

fn is_anime_entry(entry: &Value) -> bool {
    entry
        .get("class_list")
        .and_then(Value::as_array)
        .is_some_and(|classes| {
            classes.iter().filter_map(Value::as_str).any(|class| {
                class == "tag-anime" || class == "category-animation" || class == "category-anime"
            })
        })
}

fn parse_rest_movie(entry: &Value, kind: RestKind) -> Option<Movie> {
    let id = entry.get("link")?.as_str()?.to_string();
    let title = clean_text(entry.pointer("/title/rendered")?.as_str()?);

    if title.is_empty() {
        return None;
    }

    let media = entry.pointer("/_embedded/wp:featuredmedia/0");
    let poster_url = media
        .and_then(|media| media.pointer("/media_details/sizes/medium/source_url"))
        .or_else(|| media.and_then(|media| media.get("source_url")))
        .and_then(Value::as_str)
        .map(unescape_js_url);

    Some(Movie {
        id,
        title,
        info: Some(kind.label().to_string()),
        poster_url,
    })
}

fn next_page_url(url: &str, headers: &HeaderMap) -> Option<String> {
    let total_pages = headers
        .get("x-wp-totalpages")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok())?;

    let mut parsed = Url::parse(url).ok()?;
    let current_page = parsed
        .query_pairs()
        .find(|(key, _)| key == "page")
        .and_then(|(_, value)| value.parse::<u32>().ok())
        .unwrap_or(1);

    if current_page >= total_pages {
        return None;
    }

    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| key != "page")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();

    parsed.query_pairs_mut().clear().extend_pairs(pairs);
    parsed
        .query_pairs_mut()
        .append_pair("page", &(current_page + 1).to_string());

    Some(parsed.to_string())
}

fn parse_detail(html: &str, movie: &Movie) -> (Vec<Episode>, Meta) {
    let mut meta = Meta::default();
    let is_tv = movie.id.contains("/tv/") || movie.info.as_deref() == Some("Series");

    let page_title = regex!(r#"(?s)<title[^>]*>(.*?)</title>"#)
        .captures(html)
        .map(|cap| clean_text(&cap[1]));
    let title = regex!(r#"(?s)<h1[^>]+class=["'][^"']*entry-title[^"']*["'][^>]*>(.*?)</h1>"#)
        .captures(html)
        .map(|cap| clean_text(&cap[1]))
        .filter(|value| !value.is_empty())
        .or_else(|| page_title.clone().map(|value| clean_lk21_title(&value)))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| movie.title.clone());

    meta.thumb_url = regex!(
        r#"(?s)<div[^>]+class=["'][^"']*gmr-movie-data[^"']*["'][^>]*>.*?<img[^>]+src=["']([^"']+)["']"#
    )
    .captures(html)
    .map(|cap| unescape_js_url(&cap[1]))
    .or_else(|| {
        regex!(r#"(?s)<meta[^>]+property=["']og:image["'][^>]+content=["']([^"']+)["']"#)
            .captures(html)
            .map(|cap| unescape_js_url(&cap[1]))
    })
    .map(|src| absolute_url(&movie.id, &src));

    if let Some(facts) =
        regex!(r#"(?s)<dl[^>]+class=["'][^"']*lk21-guide-facts[^"']*["'][^>]*>(.*?)</dl>"#)
            .captures(html)
    {
        for cap in
            regex!(r#"(?s)<dt[^>]*>(.*?)</dt>\s*<dd[^>]*>(.*?)</dd>"#).captures_iter(&facts[1])
        {
            let key = clean_text(&cap[1]);
            let value = clean_text(&cap[2]);
            if !key.is_empty() && !value.is_empty() {
                meta.data.push((key, value));
            }
        }
    }

    if !meta
        .data
        .iter()
        .any(|(key, _)| key.to_lowercase().contains("rating"))
    {
        if let Some(score) =
            regex!(r#"(?s)itemprop=["']ratingValue["'][^>]*>(.*?)<"#).captures(html)
        {
            let score = clean_text(&score[1]);
            if !score.is_empty() {
                meta.data
                    .push(("Rating".to_string(), format!("{score}/10")));
            }
        }
    }

    if let Some(synopsis) =
        regex!(r#"(?s)<div[^>]+class=["'][^"']*lk21-guide-synopsis[^"']*["'][^>]*>(.*?)</div>"#)
            .captures(html)
    {
        let mut paragraphs = vec![];
        for cap in regex!(r#"(?s)<p[^>]*>(.*?)</p>"#).captures_iter(&synopsis[1]) {
            let text = clean_text(&cap[1]);
            if !text.is_empty() {
                paragraphs.push(text);
            }
        }

        let text = if paragraphs.is_empty() {
            html_to_text(&synopsis[1])
        } else {
            paragraphs.join("\n")
        };

        if !text.is_empty() {
            meta.synopsis = Some(text);
        }
    }

    let mut episodes = vec![];
    let mut seen = HashSet::new();

    for cap in regex!(r#"(?s)<a([^>]*lk21-episode-card[^>]*)>(.*?)</a>"#).captures_iter(html) {
        let Some(href) = regex!(r#"(?s)href=["']([^"']+)["']"#)
            .captures(&cap[1])
            .map(|cap| unescape_js_url(&cap[1]))
        else {
            continue;
        };

        let title_attr = regex!(r#"(?s)title=["']([^"']+)["']"#)
            .captures(&cap[1])
            .map(|cap| clean_text(&cap[1]));
        let label = title_attr.unwrap_or_else(|| clean_text(&cap[2]));
        let number = parse_episode_number(&label);
        let id = absolute_url(&movie.id, &href);

        if !seen.insert(id.clone()) {
            continue;
        }

        episodes.push(Episode {
            id,
            title: number
                .map(|number| format!("Episode {number}"))
                .unwrap_or(label),
            number,
            is_series: true,
        });
    }

    if episodes.is_empty() && is_tv {
        for cap in regex!(r#"(?s)<a[^>]+href=["']([^"']*/eps/[^"']+)["'][^>]*>(.*?)</a>"#)
            .captures_iter(html)
        {
            let id = absolute_url(&movie.id, &unescape_js_url(&cap[1]));
            if !seen.insert(id.clone()) {
                continue;
            }

            let label = clean_text(&cap[2]);
            let number = parse_episode_number(&label);

            episodes.push(Episode {
                id,
                title: number
                    .map(|number| format!("Episode {number}"))
                    .unwrap_or(label),
                number,
                is_series: true,
            });
        }
    }

    if !is_tv && episodes.is_empty() {
        episodes.push(Episode {
            id: movie.id.clone(),
            title: title.clone(),
            number: Some(1),
            is_series: false,
        });
    }

    episodes.sort_by(|a, b| match (a.number, b.number) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.title.cmp(&b.title),
    });

    if meta.data.is_empty() {
        meta.data.push(("Judul".to_string(), title));
    }
    meta.data.push((
        "Tipe".to_string(),
        if is_tv { "Series" } else { "Film" }.to_string(),
    ));

    (episodes, meta)
}

fn parse_player_id(html: &str) -> Option<u64> {
    regex!(r#"(?s)id=["']muvipro_player_content_id["'][^>]*data-id=["'](\d+)["']"#)
        .captures(html)
        .and_then(|cap| cap[1].parse().ok())
}

fn extract_streams(body: &str, page_url: &str) -> Vec<Stream> {
    let mut streams = vec![];
    let mut seen = HashSet::new();

    for cap in regex!(r#"(?is)<iframe[^>]+src=["']([^"']+)["']"#).captures_iter(body) {
        let url = absolute_url(page_url, &decode_html_entities(&cap[1]));
        if is_trailer(&url) || !seen.insert(url.clone()) {
            continue;
        }

        streams.push(Stream {
            title: host_label(&url),
            url: Some(url),
            referer: Some(page_url.to_string()),
            mirror: None,
        });
    }

    for cap in
        regex!(r#"(?is)<(?:video|source)[^>]+src=["']([^"']+\.(?:mp4|m3u8)(?:\?[^"']*)?)["']"#)
            .captures_iter(body)
    {
        let url = absolute_url(page_url, &decode_html_entities(&cap[1]));
        if !seen.insert(url.clone()) {
            continue;
        }

        streams.push(Stream {
            title: "Direct".to_string(),
            url: Some(url),
            referer: Some(page_url.to_string()),
            mirror: None,
        });
    }

    streams
}

fn is_trailer(url: &str) -> bool {
    let host = Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_lowercase))
        .unwrap_or_default();

    matches!(
        host.as_str(),
        "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "youtu.be"
            | "facebook.com"
            | "www.facebook.com"
            | "instagram.com"
            | "www.instagram.com"
            | "twitter.com"
            | "www.twitter.com"
            | "tiktok.com"
            | "www.tiktok.com"
    )
}

fn host_label(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.trim_start_matches("www.").to_string())
        })
        .unwrap_or_else(|| "embed".to_string())
}

fn parse_episode_number(label: &str) -> Option<u32> {
    regex!(r#"(?i)(?:episode|eps?)\s*(\d+)"#)
        .captures(label)
        .and_then(|cap| cap[1].parse().ok())
        .or_else(|| {
            let trimmed = label.trim();
            if trimmed.len() <= 5 && trimmed.chars().all(|ch| ch.is_ascii_digit()) {
                trimmed.parse().ok()
            } else {
                None
            }
        })
}

fn clean_lk21_title(input: &str) -> String {
    let mut title = clean_text(input);

    for prefix in ["Nonton Film ", "Nonton Serial ", "Nonton ", "Streaming "] {
        if let Some(stripped) = title.strip_prefix(prefix) {
            title = stripped.to_string();
            break;
        }
    }

    for suffix in [
        " LK21 Sub Indo",
        " Sub Indo HD",
        " Sub Indo",
        " Subtitle Indonesia",
        " Sub Indonesia",
    ] {
        if let Some(stripped) = title.strip_suffix(suffix) {
            title = stripped.to_string();
            break;
        }
    }

    decode_html_entities(&title).trim().to_string()
}

fn absolute_url(base: &str, href: &str) -> String {
    let href = unescape_js_url(href);
    if href.starts_with("http://") || href.starts_with("https://") {
        return href;
    }
    if href.starts_with("//") {
        return format!("https:{href}");
    }

    Url::parse(base)
        .ok()
        .and_then(|base| base.join(&href).ok())
        .map(|url| url.to_string())
        .unwrap_or(href)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_anime_from_japanese_catalog() {
        let value: Value = serde_json::from_str(
            r#"[
                {
                    "id": 1,
                    "type": "tv",
                    "link": "https://layarkaca21.health/tv/anime-contoh/",
                    "title": {"rendered": "Anime Contoh"},
                    "class_list": ["tv", "category-animation", "tag-anime", "muvicountry-japan"]
                },
                {
                    "id": 2,
                    "type": "tv",
                    "link": "https://layarkaca21.health/tv/drama-contoh/",
                    "title": {"rendered": "Drama Contoh"},
                    "class_list": ["tv", "category-drama-jepang", "muvicountry-japan"]
                }
            ]"#,
        )
        .unwrap();

        let items = parse_rest_listing(&value, RestKind::Series);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Drama Contoh");
    }

    #[test]
    fn parses_rest_listing_with_poster() {
        let value: Value = serde_json::from_str(
            r#"[
                {
                    "id": 1,
                    "type": "post",
                    "link": "https://layarkaca21.health/contoh-2026/",
                    "title": {"rendered": "Contoh &#038; Uji (2026)"},
                    "_embedded": {
                        "wp:featuredmedia": [{
                            "source_url": "https://layarkaca21.health/img/contoh.jpg"
                        }]
                    }
                }
            ]"#,
        )
        .unwrap();

        let items = parse_rest_listing(&value, RestKind::Movie);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Contoh & Uji (2026)");
        assert_eq!(items[0].info.as_deref(), Some("Film"));
        assert_eq!(
            items[0].poster_url.as_deref(),
            Some("https://layarkaca21.health/img/contoh.jpg")
        );
    }

    #[test]
    fn parses_movie_detail_facts_and_single_episode() {
        let html = r#"
            <title>Nonton Film Contoh (2026) LK21 Sub Indo | LAYARKACA21</title>
            <div class="gmr-movie-data"><figure><img src="/img/poster.jpg"></figure></div>
            <h1 class="entry-title">Contoh (2026)</h1>
            <dl class="lk21-guide-facts">
              <div class="lk21-guide-fact"><dt>Genre</dt><dd><a>Drama</a>, <a>Film</a></dd></div>
              <div class="lk21-guide-fact"><dt>Negara</dt><dd><a>Japan</a></dd></div>
              <div class="lk21-guide-fact"><dt>Sutradara</dt><dd>Orang Jepang</dd></div>
            </dl>
            <div class="lk21-guide-synopsis"><h3>Sinopsis Contoh</h3><p>Sinopsis contoh.</p></div>
        "#;

        let movie = Movie {
            id: "https://layarkaca21.health/contoh-2026/".to_string(),
            title: "Contoh (2026)".to_string(),
            ..Default::default()
        };

        let (episodes, meta) = parse_detail(html, &movie);
        assert_eq!(meta.value("genre"), Some("Drama, Film"));
        assert_eq!(meta.value("negara"), Some("Japan"));
        assert_eq!(meta.synopsis.as_deref(), Some("Sinopsis contoh."));
        assert_eq!(
            meta.thumb_url.as_deref(),
            Some("https://layarkaca21.health/img/poster.jpg")
        );
        assert_eq!(episodes.len(), 1);
        assert!(!episodes[0].is_series);
        assert_eq!(episodes[0].id, movie.id);
    }

    #[test]
    fn parses_tv_episode_cards_oldest_first() {
        let html = r#"
            <h1 class="entry-title">Contoh Series</h1>
            <a class="lk21-episode-card" href="/eps/contoh-episode-2/" title="Contoh Series &#8211; Episode 2">
              <span class="lk21-episode-index">02</span><span class="lk21-episode-copy"><strong>Episode 2</strong></span>
            </a>
            <a class="lk21-episode-card" href="/eps/contoh-episode-1/" title="Contoh Series &#8211; Episode 1">
              <span class="lk21-episode-index">01</span><span class="lk21-episode-copy"><strong>Episode 1</strong></span>
            </a>
        "#;

        let movie = Movie {
            id: "https://layarkaca21.health/tv/contoh-series/".to_string(),
            title: "Contoh Series".to_string(),
            ..Default::default()
        };

        let (episodes, _) = parse_detail(html, &movie);
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].number, Some(1));
        assert_eq!(episodes[1].number, Some(2));
        assert!(episodes[0].is_series);
        assert_eq!(
            episodes[0].id,
            "https://layarkaca21.health/eps/contoh-episode-1/"
        );
    }

    #[test]
    fn extracts_player_id_and_iframes() {
        let html = r#"<div id="muvipro_player_content_id" data-id="153221"></div>"#;
        assert_eq!(parse_player_id(html), Some(153221));

        let body = r#"<div class="gmr-embed-responsive"><IFRAME src="https://morencius.com/embed/abc"></IFRAME></div>"#;
        let streams = extract_streams(body, "https://layarkaca21.health/contoh-2026/");
        assert_eq!(streams.len(), 1);
        assert_eq!(
            streams[0].url.as_deref(),
            Some("https://morencius.com/embed/abc")
        );
        assert_eq!(streams[0].title, "morencius.com");
    }

    #[test]
    fn skips_youtube_trailer_iframes() {
        let body = r#"<iframe src="https://www.youtube.com/embed/NA"></iframe>"#;
        assert!(extract_streams(body, "https://layarkaca21.health/contoh/").is_empty());
    }
}
