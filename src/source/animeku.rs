use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use anyhow::Context;
use async_trait::async_trait;
use html_escape::decode_html_entities;
use reqwest::{header, Client, Url};

use crate::{
    models::{Episode, Listing, Meta, Movie, SearchMode, Stream},
    regex,
    source::{embed, Source},
    util::{clean_text, html_to_text, unescape_js_url},
};

const DEFAULT_BASE: &str = "https://animeku.lol";

/// Sumber utama: AnimeKu (WordPress ringan, katalog lengkap, player HLS).
///
/// Kunci kestabilannya: semua URL diambil dari HTML (bukan hardcode slug
/// kategori), domain utama `animeku.lol` selalu redirect ke mirror yang aktif,
/// dan player embed di-resolve langsung ke tautan HLS supaya MPV tidak perlu
/// membuka browser.
pub struct Animeku {
    client: Client,
    base: String,
    categories: HashMap<String, String>,
    categories_loaded: bool,
}

impl Animeku {
    pub fn new() -> Self {
        let base = std::env::var("WIBU_ANIMEKU_BASE")
            .ok()
            .filter(|value| value.starts_with("http"))
            .unwrap_or_else(|| DEFAULT_BASE.to_string())
            .trim_end_matches('/')
            .to_string();

        let client = Client::builder()
            .user_agent(embed::USER_AGENT)
            .timeout(Duration::from_secs(25))
            .connect_timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::limited(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .gzip(true)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            base,
            categories: HashMap::new(),
            categories_loaded: false,
        }
    }

    async fn get_html(&self, url: &str) -> anyhow::Result<String> {
        let mut last_error = None;

        for attempt in 0..3u64 {
            let request = self
                .client
                .get(url)
                .header(header::REFERER, &self.base)
                .header(header::ACCEPT_LANGUAGE, "id-ID,id;q=0.9,en;q=0.8,en;q=0.7")
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
                    if response.status().as_u16() == 404 || response.status().as_u16() == 410 {
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

    async fn ensure_categories(&mut self) -> anyhow::Result<()> {
        if self.categories_loaded {
            return Ok(());
        }

        let html = self.get_html(&format!("{}/", self.base)).await?;
        self.categories = parse_nav_categories(&html);
        self.categories_loaded = true;
        Ok(())
    }

    async fn category_url(&mut self, mode: SearchMode) -> anyhow::Result<String> {
        match mode {
            SearchMode::All => Ok(self.base.clone()),
            _ => {
                self.ensure_categories().await.ok();
                let key = match mode {
                    SearchMode::Ongoing => "ongoing",
                    SearchMode::Completed => "selesai",
                    SearchMode::Movie => "film",
                    SearchMode::All
                    | SearchMode::JapanMovie
                    | SearchMode::JapanSeries
                    | SearchMode::Tokusatsu => unreachable!(),
                };

                if let Some(url) = self.categories.get(key) {
                    return Ok(url.clone());
                }

                let fallback = match mode {
                    SearchMode::Ongoing => "/jalan-uf5ms8g/",
                    SearchMode::Completed => "/selesai-bjyzaqn/",
                    SearchMode::Movie => "/film-anime-6d23pb8/",
                    SearchMode::All
                    | SearchMode::JapanMovie
                    | SearchMode::JapanSeries
                    | SearchMode::Tokusatsu => "",
                };

                Ok(format!("{}{fallback}", self.base))
            }
        }
    }

    pub async fn listing(&self, url: &str) -> anyhow::Result<Listing> {
        let html = self.get_html(url).await?;
        Ok(parse_listing(&html, url))
    }

    pub async fn search_listing(
        &mut self,
        query: &str,
        mode: SearchMode,
    ) -> anyhow::Result<Listing> {
        let mut url = Url::parse(&self.category_url(mode).await?)?;

        if !query.trim().is_empty() {
            url.query_pairs_mut().append_pair("s", query.trim());
        }

        self.listing(url.as_str()).await
    }
}

impl Default for Animeku {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Source for Animeku {
    fn name(&self) -> &'static str {
        "AnimeKu"
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
        Ok(parse_streams(&html, episode))
    }

    async fn resolve(&self, stream: &Stream) -> anyhow::Result<Option<String>> {
        let Some(url) = &stream.url else {
            return Ok(None);
        };

        let referer = stream
            .referer
            .as_deref()
            .unwrap_or_else(|| self.base.as_str());

        Ok(embed::resolve(&self.client, url, referer).await)
    }
}

fn parse_nav_categories(html: &str) -> HashMap<String, String> {
    let mut categories = HashMap::new();

    let Some(nav) = regex!(r#"(?s)<nav[^>]+class=["'][^"']*aku-menu[^"']*["'][^>]*>(.*?)</nav>"#)
        .captures(html)
    else {
        return categories;
    };

    for cap in regex!(r#"(?s)<a[^>]+href=["']([^"']+)["'][^>]*>(.*?)</a>"#).captures_iter(&nav[1]) {
        let label = clean_text(&cap[2]).to_lowercase();
        let href = cap[1].to_string();

        let key = if label.contains("ongoing") {
            "ongoing"
        } else if label.contains("selesai") || label.contains("complete") {
            "selesai"
        } else if label.contains("film") || label.contains("movie") {
            "film"
        } else if label.contains("baru") || label.contains("terbaru") {
            "baru"
        } else if label.contains("favorit") {
            "favorit"
        } else {
            continue;
        };

        categories.entry(key.to_string()).or_insert(href);
    }

    categories
}

fn parse_listing(html: &str, page_url: &str) -> Listing {
    let mut items = vec![];
    let mut seen = HashSet::new();

    for cap in
        regex!(r#"(?s)<article[^>]*class=["'][^"']*\baku-sel\b[^"']*["'][^>]*>(.*?)</article>"#)
            .captures_iter(html)
    {
        if let Some(movie) = parse_movie_article(&cap[1], page_url) {
            if seen.insert(movie.id.clone()) {
                items.push(movie);
            }
        }
    }

    Listing {
        items,
        next: parse_next_page(html, page_url),
    }
}

fn parse_movie_article(chunk: &str, page_url: &str) -> Option<Movie> {
    let href = regex!(r#"(?s)<a[^>]+href=["']([^"']+)["']"#)
        .captures(chunk)
        .map(|cap| unescape_js_url(&cap[1]))?;
    let id = absolute_url(page_url, &href);

    if !id.contains("animeku") && !id.starts_with("http") {
        return None;
    }

    let title = regex!(r#"(?s)<h3[^>]*>.*?<a[^>]*>(.*?)</a>"#)
        .captures(chunk)
        .map(|cap| clean_text(&cap[1]))
        .filter(|title| !title.is_empty())
        .or_else(|| {
            regex!(r#"(?s)<img[^>]+alt=["']([^"']*)["']"#)
                .captures(chunk)
                .map(|cap| clean_text(&cap[1]))
                .filter(|title| !title.is_empty())
        })?;

    let poster_url = regex!(r#"(?s)<img[^>]+(?:data-src|src)=["']([^"']+)["']"#)
        .captures(chunk)
        .map(|cap| unescape_js_url(&cap[1]))
        .map(|src| absolute_url(page_url, &src));

    let episodes = regex!(r#"(?s)<span[^>]+class=["'][^"']*aku-eps[^"']*["'][^>]*>(.*?)</span>"#)
        .captures(chunk)
        .map(|cap| clean_text(&cap[1]))
        .filter(|value| !value.is_empty());

    let score = regex!(r#"(?s)<span[^>]+class=["'][^"']*aku-nilai[^"']*["'][^>]*>(.*?)</span>"#)
        .captures(chunk)
        .map(|cap| clean_text(&cap[1]))
        .filter(|value| !value.is_empty());

    let mut info_parts = vec![];
    if let Some(episodes) = episodes {
        if episodes.eq_ignore_ascii_case("movie") {
            info_parts.push("Film".to_string());
        } else {
            info_parts.push(episodes);
        }
    }
    if let Some(score) = score {
        info_parts.push(format!("★ {score}"));
    }

    Some(Movie {
        id,
        title,
        info: if info_parts.is_empty() {
            None
        } else {
            Some(info_parts.join(" • "))
        },
        poster_url,
    })
}

fn parse_next_page(html: &str, page_url: &str) -> Option<String> {
    let nav = regex!(r#"(?s)<nav[^>]+class=["'][^"']*aku-hal[^"']*["'][^>]*>(.*?)</nav>"#)
        .captures(html)
        .map(|cap| cap[1].to_string())?;

    for cap in regex!(r#"(?s)<a([^>]*)>(.*?)</a>"#).captures_iter(&nav) {
        let attrs = cap[1].to_lowercase();
        if !attrs.contains("next") {
            continue;
        }

        let href = regex!(r#"(?s)href=["']([^"']+)["']"#)
            .captures(&cap[1])
            .map(|cap| cap[1].to_string())?;

        return Some(absolute_url(page_url, &href));
    }

    None
}

fn parse_detail(html: &str, movie: &Movie) -> (Vec<Episode>, Meta) {
    let mut meta = Meta::default();
    meta.thumb_url = regex!(
        r#"(?s)<div[^>]+class=["'][^"']*aku-kepala-gbr[^"']*["'][^>]*>.*?<img[^>]+src=["']([^"']+)["']"#
    )
    .captures(html)
    .map(|cap| unescape_js_url(&cap[1]))
    .map(|src| absolute_url(&movie.id, &src));

    let mut title = regex!(r#"(?s)<h1[^>]*>(.*?)</h1>"#)
        .captures(html)
        .map(|cap| clean_text(&cap[1]))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| movie.title.clone());
    title = clean_anime_title(&title);

    // Baris fakta AnimeKu berbentuk pasangan sibling:
    // <div class="aku-fakta-k">Studio</div><div class="aku-fakta-v">Madhouse</div>
    for cap in regex!(
        r#"(?s)<div[^>]+class=["'][^"']*aku-fakta-k[^"']*["'][^>]*>(.*?)</div>\s*<div[^>]+class=["'][^"']*aku-fakta-v[^"']*["'][^>]*>(.*?)</div>"#
    )
    .captures_iter(html)
    {
        let key = clean_text(&cap[1]);
        let value = clean_text(&cap[2]);

        if !key.is_empty() && !value.is_empty() {
            meta.data.push((key, value));
        }
    }

    if let Some(score) =
        regex!(r#"(?s)<div[^>]+class=["'][^"']*aku-cap[^"']*["'][^>]*>(.*?)</div>"#)
            .captures(html)
            .and_then(|cap| {
                regex!(r#"(?s)<span[^>]*>(.*?)</span>"#)
                    .captures_iter(&cap[1])
                    .map(|span| clean_text(&span[1]))
                    .find(|text| text.contains('★'))
            })
    {
        if !meta
            .data
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("skor"))
        {
            meta.data.push(("Skor".to_string(), score));
        }
    }

    if let Some(synopsis) =
        regex!(r#"(?s)<div[^>]+class=["'][^"']*aku-cerita[^"']*["'][^>]*>(.*?)</div>"#)
            .captures(html)
    {
        let text = html_to_text(&synopsis[1]);
        if !text.is_empty() {
            meta.synopsis = Some(text);
        }
    }

    let mut episodes = vec![];
    let mut seen = HashSet::new();
    let mut has_episode_list = false;

    for cap in regex!(r#"(?s)<a[^>]+href=["']([^"']+)["'][^>]*>(.*?)</a>"#).captures_iter(html) {
        let href = unescape_js_url(&cap[1]);
        if !href.contains("/play/") {
            continue;
        }

        has_episode_list = true;
        let id = absolute_url(&movie.id, &href);
        if !seen.insert(id.clone()) {
            continue;
        }

        let label = clean_text(&cap[2]);
        let number = parse_episode_number(&label);
        let title = match number {
            Some(number) => format!("Episode {number}"),
            None if label.is_empty() => "Episode".to_string(),
            None => label,
        };

        episodes.push(Episode {
            id,
            title,
            number,
            is_series: true,
        });
    }

    episodes.sort_by(|a, b| match (a.number, b.number) {
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.title.cmp(&b.title),
    });

    let is_movie = meta.data.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("tipe") && value.to_lowercase().contains("movie")
    }) || meta
        .data
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("episode") && value.trim() == "1")
        && !has_episode_list;

    let series = !is_movie && (episodes.len() > 1 || has_episode_list);
    for episode in &mut episodes {
        episode.is_series = series;
    }

    // Kalau halaman tidak punya daftar episode tapi punya tombol putar,
    // anggap halaman itu satu tontonan (film / OVA).
    if episodes.is_empty() {
        if let Some(href) = regex!(r#"(?s)<a[^>]+href=["']([^"']*/play/[^"']*)["']"#)
            .captures(html)
            .map(|cap| unescape_js_url(&cap[1]))
        {
            episodes.push(Episode {
                id: absolute_url(&movie.id, &href),
                title: title.clone(),
                number: Some(1),
                is_series: false,
            });
        }
    }

    if episodes.len() == 1 {
        episodes[0].is_series = false;
    }

    // Supaya judul anime tetap terlihat kalau halaman detail tidak lengkap.
    if meta.data.is_empty() && !title.is_empty() {
        meta.data.push(("Judul".to_string(), title));
    }

    (episodes, meta)
}

fn parse_streams(html: &str, episode: &Episode) -> Vec<Stream> {
    let referer = Some(episode.id.clone());

    if let Some(iframe) = regex!(
        r#"(?s)<div[^>]+class=["'][^"']*aku-layar[^"']*["'][^>]*>.*?<iframe[^>]+src=["']([^"']+)["']"#
    )
    .captures(html)
    .map(|cap| unescape_js_url(&cap[1]))
    .or_else(|| {
        regex!(r#"(?s)<iframe[^>]+src=["']([^"']+)["']"#)
            .captures(html)
            .map(|cap| unescape_js_url(&cap[1]))
    }) {
        return vec![Stream {
            title: "Auto • HLS (AnimeKu)".to_string(),
            url: Some(absolute_url(&episode.id, &iframe)),
            referer,
            mirror: None,
        }];
    }

    if let Some(source) =
        regex!(r#"(?s)<(?:video|source)[^>]+src=["']([^"']+\.(?:mp4|m3u8)(?:\?[^"']*)?)["']"#)
            .captures(html)
            .map(|cap| unescape_js_url(&cap[1]))
    {
        return vec![Stream {
            title: "Auto • direct".to_string(),
            url: Some(absolute_url(&episode.id, &source)),
            referer,
            mirror: None,
        }];
    }

    vec![]
}

fn parse_episode_number(label: &str) -> Option<u32> {
    regex!(r#"(?i)(?:eps?|episode)\s*(\d+)"#)
        .captures(label)
        .and_then(|cap| cap[1].parse().ok())
        .or_else(|| {
            let trimmed = label.trim();
            if trimmed.len() <= 5 && trimmed.chars().all(|c| c.is_ascii_digit()) {
                trimmed.parse().ok()
            } else {
                None
            }
        })
}

fn clean_anime_title(input: &str) -> String {
    let mut title = clean_text(input);

    for prefix in ["Nonton Anime ", "Nonton ", "Streaming "] {
        if let Some(stripped) = title.strip_prefix(prefix) {
            title = stripped.to_string();
            break;
        }
    }

    for suffix in [
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
    fn parses_search_result_cards() {
        let html = r#"
          <div class="aku-petak">
            <article class="aku-sel"><a class="aku-bing" href="https://ww2.animeku.lol/sousou-no-frieren/"><img src="https://static.example/poster.jpg" alt="Sousou No Frieren" loading="lazy" width="200" height="300"><span class="aku-eps">Eps 28</span><span class="aku-nilai">9.26</span></a><h3><a href="https://ww2.animeku.lol/sousou-no-frieren/">Sousou No Frieren</a></h3></article>
          </div>
        "#;

        let listing = parse_listing(html, "https://ww2.animeku.lol/?s=frieren");
        assert_eq!(listing.items.len(), 1);
        assert_eq!(listing.items[0].title, "Sousou No Frieren");
        assert_eq!(
            listing.items[0].id,
            "https://ww2.animeku.lol/sousou-no-frieren/"
        );
        assert!(listing.items[0].info.as_deref().unwrap().contains("28"));
    }

    #[test]
    fn parses_episode_list_and_orders_newest_first() {
        let html = r#"
          <div class="aku-eplist">
            <a href="https://ww2.animeku.lol/play/sousou-no-frieren-episode-1-a/">Eps 1</a>
            <a href="https://ww2.animeku.lol/play/sousou-no-frieren-episode-2-b/">Eps 2</a>
            <a href="https://ww2.animeku.lol/play/sousou-no-frieren-episode-10-c/">Eps 10</a>
          </div>
        "#;
        let movie = Movie {
            id: "https://ww2.animeku.lol/sousou-no-frieren/".to_string(),
            title: "Sousou No Frieren".to_string(),
            ..Default::default()
        };

        let (episodes, _) = parse_detail(html, &movie);
        assert_eq!(episodes.len(), 3);
        assert_eq!(episodes[0].title, "Episode 10");
        assert_eq!(episodes[0].number, Some(10));
    }

    #[test]
    fn extracts_player_iframe() {
        let html = r#"<div class="aku-layar"><iframe src="https://putarin.biz/e/BGusdRwJje" allowfullscreen></iframe></div>"#;
        let episode = Episode {
            id: "https://ww2.animeku.lol/play/x/".to_string(),
            title: "Episode 1".to_string(),
            ..Default::default()
        };

        let streams = parse_streams(html, &episode);
        assert_eq!(streams.len(), 1);
        assert_eq!(
            streams[0].url.as_deref(),
            Some("https://putarin.biz/e/BGusdRwJje")
        );
    }

    #[test]
    fn parses_detail_facts_poster_and_synopsis() {
        let html = r#"
          <div class="aku-kepala-gbr"><img src="/img/poster.jpg" alt="Contoh"></div>
          <h1>Nonton Anime Contoh Sub Indo HD</h1>
          <div class="aku-fakta">
            <div class="aku-fakta-b"><div class="aku-fakta-k">Genre</div><div class="aku-fakta-v"><a href="/g/1/">Drama</a>, <a href="/g/2/">Fantasi</a></div></div>
            <div class="aku-fakta-b"><div class="aku-fakta-k">Studio</div><div class="aku-fakta-v">Madhouse</div></div>
            <div class="aku-fakta-b"><div class="aku-fakta-k">Status</div><div class="aku-fakta-v">Finished Airing</div></div>
            <div class="aku-fakta-b"><div class="aku-fakta-k">Episode</div><div class="aku-fakta-v">12</div></div>
          </div>
          <div class="aku-cerita"><p>Sinopsis contoh.</p></div>
          <div class="aku-epwrap"><a href="/play/contoh-episode-1/">Eps 1</a></div>
        "#;

        let movie = Movie {
            id: "https://ww2.animeku.lol/contoh/".to_string(),
            title: "Contoh".to_string(),
            ..Default::default()
        };

        let (episodes, meta) = parse_detail(html, &movie);
        assert_eq!(meta.value("studio"), Some("Madhouse"));
        assert_eq!(meta.value("status"), Some("Finished Airing"));
        assert_eq!(meta.value("episode"), Some("12"));
        assert_eq!(meta.value("genre"), Some("Drama, Fantasi"));
        assert_eq!(
            meta.thumb_url.as_deref(),
            Some("https://ww2.animeku.lol/img/poster.jpg")
        );
        assert_eq!(meta.synopsis.as_deref(), Some("Sinopsis contoh."));
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].number, Some(1));
        assert!(!episodes[0].is_series);
    }

    #[test]
    fn parses_next_page() {
        let html = r#"<nav class="aku-hal"><span class="page-numbers current">1</span><a class="page-numbers" href="https://ww2.animeku.lol/page/2/">2</a><a class="next page-numbers" href="https://ww2.animeku.lol/page/2/">&rsaquo;</a></nav>"#;
        assert_eq!(
            parse_next_page(html, "https://ww2.animeku.lol/"),
            Some("https://ww2.animeku.lol/page/2/".to_string())
        );
    }
}
