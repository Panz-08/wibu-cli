use std::{collections::HashSet, time::Duration};

use anyhow::Context;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use html_escape::decode_html_entities;
use reqwest::{header, Client, Url};

use crate::{
    models::{Episode, Listing, Meta, Movie, SearchMode, Stream},
    regex,
    source::{embed, Source},
    util::{clean_text, html_to_text, unescape_js_url},
};

const DEFAULT_BASE: &str = "https://www.tokusatsuindo.com";

const CATEGORIES: [(&str, &str); 4] = [
    ("ultraman2", "Ultraman"),
    ("kamen-rider-2", "Kamen Rider"),
    ("super-sentai2", "Super Sentai"),
    ("movie-special", "Movie & Special"),
];

/// Katalog tokusatsu Indonesia: Ultraman, Kamen Rider, Super Sentai, dan
/// film/special.
///
/// Katalog series dibaca dari mega-menu tiap kategori. Halaman episode
/// menyimpan iframe player dalam atribut `data-embed` berisi base64. Player
/// tokusatsu memakai chain `/x/` -> `/p/` yang berujung ke Google Drive.
pub struct Tokusatsuindo {
    client: Client,
    base: String,
    catalog: Option<Vec<Movie>>,
    catalog_errors: Vec<String>,
}

impl Tokusatsuindo {
    pub fn new() -> Self {
        let base = std::env::var("WIBU_TOKUSATSU_BASE")
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
            catalog: None,
            catalog_errors: vec![],
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

    async fn ensure_catalog(&mut self) -> anyhow::Result<()> {
        if self.catalog.is_some() {
            return Ok(());
        }

        let ultraman_url = format!("{}/ultraman2/", self.base);
        let rider_url = format!("{}/kamen-rider-2/", self.base);
        let sentai_url = format!("{}/super-sentai2/", self.base);
        let movie_url = format!("{}/movie-special/", self.base);

        let (ultraman, rider, sentai, movie) = tokio::join!(
            self.get_html(&ultraman_url),
            self.get_html(&rider_url),
            self.get_html(&sentai_url),
            self.get_html(&movie_url),
        );

        let mut items = vec![];
        let mut errors = vec![];

        for (result, (_, label)) in [
            (ultraman, CATEGORIES[0]),
            (rider, CATEGORIES[1]),
            (sentai, CATEGORIES[2]),
            (movie, CATEGORIES[3]),
        ] {
            match result {
                Ok(html) => items.extend(parse_category(&html, label, &self.base)),
                Err(error) => errors.push(format!("{label}: {error:#}")),
            }
        }

        let mut seen = HashSet::new();
        items.retain(|movie| seen.insert(movie.id.clone()));

        if items.is_empty() {
            anyhow::bail!("Katalog tokusatsu tidak bisa dimuat. {}", errors.join("; "));
        }

        self.catalog = Some(items);
        self.catalog_errors = errors;
        Ok(())
    }

    pub async fn search_listing(
        &mut self,
        query: &str,
        _mode: SearchMode,
    ) -> anyhow::Result<Listing> {
        self.ensure_catalog().await?;

        let tokens: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(ToOwned::to_owned)
            .collect();

        let items = self
            .catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .iter()
                    .filter(|movie| {
                        if tokens.is_empty() {
                            return true;
                        }

                        let haystack = format!("{} {}", movie.title, movie.id).to_lowercase();
                        tokens.iter().all(|token| haystack.contains(token))
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(Listing { items, next: None })
    }

    pub async fn resolve_player_url(client: &Client, start: &str, referer: &str) -> Option<String> {
        if let Some(id) = drive_file_id(start) {
            return Some(google_drive_direct(&id));
        }

        if start.contains("player.tokusatsuindo.com") {
            let sources = player_stream_sources(client, start, referer).await;
            if let Some(id) = sources.drive_ids.first() {
                return Some(google_drive_direct(id));
            }
            if let Some(url) = sources.proxy_urls.first() {
                return Some(url.clone());
            }
        }

        None
    }
}

impl Default for Tokusatsuindo {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Source for Tokusatsuindo {
    fn name(&self) -> &'static str {
        "Tokusatsuindo"
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

        if let Some(embed_url) = extract_embed_url(&html) {
            if embed_url.contains("player.tokusatsuindo.com") {
                let sources = player_stream_sources(&self.client, &embed_url, &episode.id).await;
                for (index, id) in sources.drive_ids.iter().enumerate() {
                    let url = google_drive_direct(id);
                    if seen.insert(url.clone()) {
                        streams.push(Stream {
                            title: format!("Google Drive {}", index + 1),
                            url: Some(url),
                            referer: Some(episode.id.clone()),
                            mirror: None,
                        });
                    }
                }

                for (index, url) in sources.proxy_urls.iter().enumerate() {
                    if seen.insert(url.clone()) {
                        streams.push(Stream {
                            title: format!("API Source {}", index + 1),
                            url: Some(url.clone()),
                            referer: Some(episode.id.clone()),
                            mirror: None,
                        });
                    }
                }
            }

            if streams.is_empty() {
                if let Some(id) = drive_file_id(&embed_url) {
                    let url = google_drive_direct(&id);
                    if seen.insert(url.clone()) {
                        streams.push(Stream {
                            title: "Google Drive".to_string(),
                            url: Some(url),
                            referer: Some(episode.id.clone()),
                            mirror: None,
                        });
                    }
                } else if seen.insert(embed_url.clone()) {
                    streams.push(Stream {
                        title: host_label(&embed_url),
                        url: Some(embed_url),
                        referer: Some(episode.id.clone()),
                        mirror: None,
                    });
                }
            }
        }

        if streams.is_empty() {
            for id in extract_drive_ids(&html) {
                let url = google_drive_direct(&id);
                if seen.insert(url.clone()) {
                    streams.push(Stream {
                        title: "Google Drive".to_string(),
                        url: Some(url),
                        referer: Some(episode.id.clone()),
                        mirror: None,
                    });
                }
            }
        }

        Ok(streams)
    }

    async fn resolve(&self, stream: &Stream) -> anyhow::Result<Option<String>> {
        let Some(url) = &stream.url else {
            return Ok(None);
        };

        if is_google_drive_direct(url) {
            return Ok(Some(url.clone()));
        }

        if let Some(id) = drive_file_id(url) {
            return Ok(Some(google_drive_direct(&id)));
        }

        if url.contains("/stream.php") {
            return Ok(Some(url.clone()));
        }

        let referer = stream.referer.as_deref().unwrap_or(self.base.as_str());

        if url.contains("player.tokusatsuindo.com") {
            if let Some(resolved) =
                Tokusatsuindo::resolve_player_url(&self.client, url, referer).await
            {
                return Ok(Some(resolved));
            }
        }

        Ok(embed::resolve(&self.client, url, referer).await)
    }
}

fn parse_category(html: &str, label: &str, base: &str) -> Vec<Movie> {
    let mut items = vec![];
    let mut seen = HashSet::new();

    if label == "Movie & Special" {
        for cap in regex!(r#"(?s)<a([^>]*panel-title[^>]*)>(.*?)</a>"#).captures_iter(html) {
            let Some(href) = regex!(r#"(?s)href=["']([^"']+)["']"#)
                .captures(&cap[1])
                .map(|cap| unescape_js_url(&cap[1]))
            else {
                continue;
            };

            let id = absolute_url(base, &href);
            if !seen.insert(id.clone()) {
                continue;
            }

            let title = clean_toku_title(&clean_text(&cap[2]));
            if title.is_empty() {
                continue;
            }

            items.push(Movie {
                id,
                title,
                info: Some(label.to_string()),
                poster_url: None,
            });
        }

        return items;
    }

    let prefix = format!("{base}/{}/", category_slug(label));
    for cap in regex!(r#"(?s)<a[^>]+href=["']([^"']+)["'][^>]*>(.*?)</a>"#).captures_iter(html) {
        let href = unescape_js_url(&cap[1]);
        if !href.starts_with(&prefix) {
            continue;
        }

        let rest = &href[prefix.len()..];
        let slug = rest.trim_end_matches('/');
        if slug.is_empty() || slug.contains('/') || slug.contains('?') || slug.contains('#') {
            continue;
        }

        let id = format!("{prefix}{slug}/");
        if !seen.insert(id.clone()) {
            continue;
        }

        let poster_url = regex!(r#"(?s)<img[^>]+(?:data-src|src)=["']([^"']+)["']"#)
            .captures(&cap[2])
            .map(|cap| absolute_url(base, &unescape_js_url(&cap[1])));

        items.push(Movie {
            id,
            title: prettify_series_title(slug, label),
            info: Some(label.to_string()),
            poster_url,
        });
    }

    items
}

fn category_slug(label: &str) -> &'static str {
    match label {
        "Ultraman" => "ultraman2",
        "Kamen Rider" => "kamen-rider-2",
        "Super Sentai" => "super-sentai2",
        _ => "movie-special",
    }
}

fn category_label(url: &str) -> &'static str {
    if url.contains("/ultraman2/") {
        "Ultraman"
    } else if url.contains("/kamen-rider-2/") {
        "Kamen Rider"
    } else if url.contains("/super-sentai2/") {
        "Super Sentai"
    } else {
        "Movie & Special"
    }
}

fn parse_detail(html: &str, movie: &Movie) -> (Vec<Episode>, Meta) {
    let mut meta = Meta::default();
    let category = category_label(&movie.id);
    let force_single = category == "Movie & Special";

    let title = parse_page_title(html).unwrap_or_else(|| movie.title.clone());
    meta.thumb_url = regex!(
        r#"(?s)<meta[^>]+property=["']og:image["'][^>]+content=["']([^"']+)["']"#
    )
    .captures(html)
    .map(|cap| unescape_js_url(&cap[1]))
    .or_else(|| {
        regex!(
            r#"(?s)<article[^>]+class=["'][^"']*gmp-page-content[^"']*["'][^>]*>.*?<img[^>]+src=["']([^"']+)["']"#
        )
        .captures(html)
        .map(|cap| unescape_js_url(&cap[1]))
    })
    .map(|src| absolute_url(&movie.id, &src))
    .or_else(|| movie.poster_url.clone());

    if let Some(description) =
        regex!(r#"(?s)<meta[^>]+name=["']description["'][^>]+content=["']([^"']*)["']"#)
            .captures(html)
            .map(|cap| clean_text(&cap[1]))
    {
        if !description.is_empty() && !description.to_lowercase().contains("tokusatsuindo.com") {
            meta.synopsis = Some(description);
        }
    }

    if meta.synopsis.is_none() {
        if let Some(content) = regex!(
            r#"(?s)<article[^>]+class=["'][^"']*gmp-page-content[^"']*["'][^>]*>(.*?)</article>"#
        )
        .captures(html)
        {
            let text = html_to_text(&content[1]);
            let first = text
                .lines()
                .map(str::trim)
                .filter(|line| {
                    let lower = line.to_lowercase();
                    !line.is_empty()
                        && !lower.contains("player siap")
                        && !lower.contains("sumber:")
                        && !lower.contains("sub indonesia")
                        && !lower.contains("sub indo")
                        && !lower.contains("baca juga")
                        && !line.starts_with("/*")
                        && !line.starts_with("*/")
                        && !lower.contains("function(")
                        && !lower.contains("dataset.")
                        && !lower.contains("appendchild")
                        && !lower.contains("createelement")
                        && !lower.starts_with("document.")
                })
                .take(3)
                .collect::<Vec<_>>()
                .join("\n");

            if !first.is_empty() {
                meta.synopsis = Some(first);
            }
        }
    }

    let mut episodes = vec![];
    let mut seen = HashSet::new();

    if !force_single {
        if let Some(list) =
            regex!(r#"(?s)<ul[^>]+class=["'][^"']*lcp_catlist[^"']*["'][^>]*>(.*?)</ul>"#)
                .captures(html)
        {
            for cap in
                regex!(r#"(?s)<a[^>]+href=["']([^"']+)["'][^>]*>(.*?)</a>"#).captures_iter(&list[1])
            {
                let href = unescape_js_url(&cap[1]);
                let label = clean_text(&cap[2]);
                let lower = label.to_lowercase();

                if !lower.contains("episode") && !href.to_lowercase().contains("-episode-") {
                    continue;
                }

                let id = absolute_url(&movie.id, &href);
                if !seen.insert(id.clone()) {
                    continue;
                }

                let number = parse_episode_number(&label);
                episodes.push(Episode {
                    id,
                    title: number
                        .map(|number| format!("Episode {number}"))
                        .unwrap_or_else(|| clean_toku_title(&label)),
                    number,
                    is_series: true,
                });
            }
        }
    }

    episodes.sort_by(|a, b| match (a.number, b.number) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.title.cmp(&b.title),
    });

    let series = !force_single && !episodes.is_empty();
    for episode in &mut episodes {
        episode.is_series = series;
    }

    if episodes.is_empty() {
        episodes.push(Episode {
            id: movie.id.clone(),
            title: title.clone(),
            number: Some(1),
            is_series: false,
        });
    }

    if !episodes.is_empty() && episodes.iter().any(|episode| episode.is_series) {
        meta.data
            .push(("Episode".to_string(), episodes.len().to_string()));
    }

    meta.data
        .push(("Kategori".to_string(), category.to_string()));
    meta.data.push((
        "Tipe".to_string(),
        if series { "Series" } else { "Film" }.to_string(),
    ));
    meta.data.push(("Negara".to_string(), "Japan".to_string()));
    meta.data
        .push(("Genre".to_string(), "Tokusatsu".to_string()));

    (episodes, meta)
}

fn extract_embed_url(html: &str) -> Option<String> {
    if let Some(cap) = regex!(r#"(?s)data-embed=["']([^"']+)["']"#).captures(html) {
        let raw = decode_html_entities(&cap[1]).to_string();
        if let Ok(bytes) = STANDARD.decode(raw.trim()) {
            let decoded = String::from_utf8_lossy(&bytes);
            if let Some(src) = first_iframe_src(&decoded) {
                return Some(src);
            }
        }

        if raw.starts_with("http://") || raw.starts_with("https://") {
            return Some(unescape_js_url(&raw));
        }
    }

    for cap in regex!(r#"(?is)<iframe[^>]+src=["']([^"']+)["']"#).captures_iter(html) {
        let src = unescape_js_url(&decode_html_entities(&cap[1]));
        if src.contains("player.tokusatsuindo.com") || src.contains("drive.google.com") {
            return Some(src);
        }
    }

    None
}

fn first_iframe_src(html: &str) -> Option<String> {
    regex!(r#"(?is)<iframe[^>]+src=["']([^"']+)["']"#)
        .captures(html)
        .map(|cap| unescape_js_url(&decode_html_entities(&cap[1])))
}

struct PlayerSources {
    drive_ids: Vec<String>,
    proxy_urls: Vec<String>,
}

async fn player_stream_sources(client: &Client, start: &str, referer: &str) -> PlayerSources {
    let mut drive_ids = vec![];
    let mut proxy_urls = vec![];

    if let Some(x_html) = fetch_text(client, start, referer).await {
        drive_ids.extend(extract_drive_ids(&x_html));

        if let Some(p_url) = extract_player_next(&x_html) {
            if let Some(p_html) = fetch_text(client, &p_url, start).await {
                drive_ids.extend(extract_drive_ids(&p_html));
                proxy_urls.extend(extract_proxy_urls(&p_html));
            }
        }
    }

    let mut seen = HashSet::new();
    drive_ids.retain(|id| seen.insert(id.clone()));

    let mut seen = HashSet::new();
    proxy_urls.retain(|url| seen.insert(url.clone()));

    PlayerSources {
        drive_ids,
        proxy_urls,
    }
}

fn extract_proxy_urls(html: &str) -> Vec<String> {
    let mut urls = vec![];
    let mut seen = HashSet::new();

    for cap in regex!(r#"(?s)(https?://player\.tokusatsuindo\.com/stream\.php\?[^"'\s<>\\]+)"#)
        .captures_iter(html)
    {
        let url = unescape_js_url(&decode_html_entities(&cap[1]));
        if seen.insert(url.clone()) {
            urls.push(url);
        }
    }

    urls
}

fn extract_player_next(html: &str) -> Option<String> {
    for cap in regex!(r#"(?s)data-src=["']([^"']+)["']"#).captures_iter(html) {
        let url = unescape_js_url(&decode_html_entities(&cap[1]));
        if url.starts_with("http") && url.contains("player.tokusatsuindo.com") {
            return Some(url);
        }
    }

    None
}

fn extract_drive_ids(html: &str) -> Vec<String> {
    let mut ids = vec![];
    let mut seen = HashSet::new();

    for cap in regex!(
        r#"(?s)(?:drive\.google\.com/file/d/|drive\.usercontent\.google\.com/download\?id=)([A-Za-z0-9_-]{10,})"#
    )
    .captures_iter(html)
    {
        if seen.insert(cap[1].to_string()) {
            ids.push(cap[1].to_string());
        }
    }

    for cap in regex!(r#"(?s)drive\.google\.com/(?:uc|open)\?[^"'<>]*id=([A-Za-z0-9_-]{10,})"#)
        .captures_iter(html)
    {
        if seen.insert(cap[1].to_string()) {
            ids.push(cap[1].to_string());
        }
    }

    ids
}

fn drive_file_id(url: &str) -> Option<String> {
    if !url.contains("drive.google.com") && !url.contains("drive.usercontent.google.com") {
        return None;
    }

    regex!(r#"/file/d/([A-Za-z0-9_-]{10,})"#)
        .captures(url)
        .or_else(|| regex!(r#"[?&]id=([A-Za-z0-9_-]{10,})"#).captures(url))
        .map(|cap| cap[1].to_string())
}

fn google_drive_direct(id: &str) -> String {
    format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t")
}

fn is_google_drive_direct(url: &str) -> bool {
    url.contains("drive.usercontent.google.com/download") && url.contains("export=download")
}

async fn fetch_text(client: &Client, url: &str, referer: &str) -> Option<String> {
    for attempt in 0..2u64 {
        let response = client
            .get(url)
            .header(header::USER_AGENT, embed::USER_AGENT)
            .header(header::REFERER, referer)
            .header(
                header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .send()
            .await;

        if let Ok(response) = response {
            if response.status().is_success() {
                if let Ok(body) = response.text().await {
                    return Some(body);
                }
            }
        }

        if attempt == 0 {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    None
}

fn parse_page_title(html: &str) -> Option<String> {
    let title = regex!(
        r#"(?s)<section[^>]+class=["'][^"']*gmp-page-hero[^"']*["'][^>]*>.*?<h1[^>]*>(.*?)</h1>"#
    )
    .captures(html)
    .map(|cap| clean_text(&cap[1]))
    .or_else(|| {
        regex!(r#"(?s)<title[^>]*>(.*?)</title>"#)
            .captures(html)
            .map(|cap| clean_text(&cap[1]))
    })?;

    let title = clean_toku_title(&title);
    if title.is_empty() {
        return None;
    }

    if title.chars().all(|ch| !ch.is_uppercase()) {
        Some(title_case_words(&title))
    } else {
        Some(title)
    }
}

fn clean_toku_title(input: &str) -> String {
    let mut title = clean_text(input);

    if let Some((before, _)) = title.split_once(" — tokusatsuindo.com") {
        title = before.to_string();
    }

    for suffix in [
        " sub indonesia tamat",
        " sub indonesia",
        " sub indo",
        " subtitle indonesia",
        " sub indonesia hd",
    ] {
        title = strip_suffix_ci(&title, suffix);
    }

    title.trim().to_string()
}

fn strip_suffix_ci(input: &str, suffix: &str) -> String {
    let lower = input.to_lowercase();
    let suffix = suffix.to_lowercase();

    if lower.ends_with(&suffix) {
        input[..input.len() - suffix.len()].trim().to_string()
    } else {
        input.to_string()
    }
}

fn prettify_series_title(slug: &str, label: &str) -> String {
    let base = slug.strip_suffix("-2").unwrap_or(slug);
    let mut title = title_case_words(&base.replace('-', " "));

    let lower = title.to_lowercase();
    let prefix = match label {
        "Ultraman" => "Ultraman",
        "Kamen Rider" => "Kamen Rider",
        "Super Sentai" => "Super Sentai",
        _ => return title,
    };

    if !lower.starts_with(&prefix.to_lowercase()) {
        title = format!("{prefix} {title}");
    }

    title
}

fn title_case_words(input: &str) -> String {
    input
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_episode_number(label: &str) -> Option<u32> {
    regex!(r#"(?i)(?:episode|eps?)\s*(\d+)"#)
        .captures(label)
        .and_then(|cap| cap[1].parse().ok())
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
    fn parses_series_links_from_category_menu() {
        let html = r#"
          <a href="https://www.tokusatsuindo.com/ultraman2/ultraman-tiga/">
            <span class="gmp-menu-image"><img src="https://img/tiga.png"></span><span>tiga</span>
          </a>
          <a href="https://www.tokusatsuindo.com/kamen-rider-2/kamen-rider-kuuga/">
            <span>kuuga</span>
          </a>
          <a href="https://www.tokusatsuindo.com/ultraman2/">semua</a>
        "#;

        let items = parse_category(html, "Ultraman", "https://www.tokusatsuindo.com");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Ultraman Tiga");
        assert_eq!(
            items[0].id,
            "https://www.tokusatsuindo.com/ultraman2/ultraman-tiga/"
        );
        assert_eq!(items[0].poster_url.as_deref(), Some("https://img/tiga.png"));
    }

    #[test]
    fn parses_movie_special_cards() {
        let html = r#"
          <div class="pt-cv-content-item">
            <a class="panel-title" href="https://www.tokusatsuindo.com/ultraman-tiga-final-odyssey-sub-indonesia/">
              Ultraman Tiga The Final Odyssey sub Indonesia
            </a>
          </div>
        "#;

        let items = parse_category(html, "Movie & Special", "https://www.tokusatsuindo.com");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Ultraman Tiga The Final Odyssey");
        assert_eq!(items[0].info.as_deref(), Some("Movie & Special"));
    }

    #[test]
    fn parses_series_detail_episodes_oldest_first() {
        let html = r#"
          <section class="gmp-page-hero"><h1>ultraman tiga</h1></section>
          <meta property="og:image" content="https://img/tiga.jpg">
          <meta name="description" content="Sinopsis Ultraman Tiga.">
          <ul class="lcp_catlist">
            <li><a href="https://www.tokusatsuindo.com/ultraman-tiga-episode-2-sub-indonesia/">ultraman tiga episode 2 sub indonesia</a></li>
            <li><a href="https://www.tokusatsuindo.com/ultraman-tiga-episode-1-sub-indonesia/">ultraman tiga episode 1 sub indonesia</a></li>
            <li><a href="https://www.tokusatsuindo.com/ultraman-tiga-movie-sub-indonesia/">ultraman tiga movie sub indonesia</a></li>
          </ul>
        "#;

        let movie = Movie {
            id: "https://www.tokusatsuindo.com/ultraman2/ultraman-tiga/".to_string(),
            title: "Ultraman Tiga".to_string(),
            info: Some("Ultraman".to_string()),
            ..Default::default()
        };

        let (episodes, meta) = parse_detail(html, &movie);
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].number, Some(1));
        assert_eq!(episodes[1].number, Some(2));
        assert!(episodes[0].is_series);
        assert_eq!(meta.value("kategori"), Some("Ultraman"));
        assert_eq!(meta.value("episode"), Some("2"));
        assert_eq!(meta.synopsis.as_deref(), Some("Sinopsis Ultraman Tiga."));
    }

    #[test]
    fn treats_movie_special_as_single_play() {
        let html = r#"
          <section class="gmp-page-hero"><h1>Ultraman Tiga The Final Odyssey</h1></section>
          <ul class="lcp_catlist">
            <li><a href="https://www.tokusatsuindo.com/ultraman-tiga-episode-1-sub-indonesia/">ultraman tiga episode 1 sub indonesia</a></li>
          </ul>
        "#;

        let movie = Movie {
            id: "https://www.tokusatsuindo.com/ultraman-tiga-the-final-odyssey-sub-indonesia/"
                .to_string(),
            title: "Ultraman Tiga The Final Odyssey".to_string(),
            info: Some("Movie & Special".to_string()),
            ..Default::default()
        };

        let (episodes, _) = parse_detail(html, &movie);
        assert_eq!(episodes.len(), 1);
        assert!(!episodes[0].is_series);
        assert_eq!(episodes[0].id, movie.id);
    }

    #[test]
    fn ignores_ad_script_in_synopsis() {
        let html = r#"
          <section class="gmp-page-hero"><h1>Ultraman Tiga</h1></section>
          <article class="gmp-page-content">
            <script>(function(s){s.dataset.zone='1'})()</script>
            <p>Sinopsis yang bersih.</p>
            <p>Paragraf kedua.</p>
          </article>
          <ul class="lcp_catlist">
            <li><a href="https://www.tokusatsuindo.com/ultraman-tiga-episode-1-sub-indonesia/">ultraman tiga episode 1 sub indonesia</a></li>
          </ul>
        "#;

        let movie = Movie {
            id: "https://www.tokusatsuindo.com/ultraman2/ultraman-tiga/".to_string(),
            title: "Ultraman Tiga".to_string(),
            ..Default::default()
        };

        let (_, meta) = parse_detail(html, &movie);
        assert_eq!(
            meta.synopsis.as_deref(),
            Some("Sinopsis yang bersih.\nParagraf kedua.")
        );
    }

    #[test]
    fn decodes_base64_player_embed() {
        let iframe =
            r#"<iframe src="https://player.tokusatsuindo.com/x/?t=abc" allowfullscreen></iframe>"#;
        let encoded = STANDARD.encode(iframe);
        let html = format!(r#"<div class="gmp-player-frame" data-embed="{encoded}"></div>"#);

        assert_eq!(
            extract_embed_url(&html).as_deref(),
            Some("https://player.tokusatsuindo.com/x/?t=abc")
        );
    }

    #[test]
    fn extracts_proxy_stream_sources() {
        let html = r#"
          const proxySources = [{"label":"API Source 1","url":"https://player.tokusatsuindo.com/stream.php?t=abc&amp;start=1"},{"label":"API Source 1","url":"https://player.tokusatsuindo.com/stream.php?t=abc&amp;start=1"}];
        "#;

        let urls = extract_proxy_urls(html);
        assert_eq!(urls.len(), 1);
        assert_eq!(
            urls[0],
            "https://player.tokusatsuindo.com/stream.php?t=abc&start=1"
        );
    }

    #[test]
    fn extracts_google_drive_ids() {
        let html = r#"
          https://drive.google.com/file/d/1ECi5F9-PLvCUG-Kad4r4uIQJs8NA5bVl/preview
          https://drive.google.com/file/d/1ECi5F9-PLvCUG-Kad4r4uIQJs8NA5bVl/preview
          https://drive.google.com/uc?export=download&id=1CZ5DT71oeCjoO7lcuj_XgTj97Rr8UUAL
        "#;

        let ids = extract_drive_ids(html);
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "1ECi5F9-PLvCUG-Kad4r4uIQJs8NA5bVl");
        assert_eq!(ids[1], "1CZ5DT71oeCjoO7lcuj_XgTj97Rr8UUAL");
        assert_eq!(
            google_drive_direct(&ids[0]),
            "https://drive.usercontent.google.com/download?id=1ECi5F9-PLvCUG-Kad4r4uIQJs8NA5bVl&export=download&confirm=t"
        );
    }
}
