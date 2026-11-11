use std::{
    fmt,
    io::{self, IsTerminal},
    time::Duration,
};

use colored::Colorize;
use reqwest::Client;

use crate::{
    input::{choice, confirm, main_menu, optional_query, search_prompt, MenuAction},
    models::{Episode, Listing, Meta, Movie, SearchMode, Stream},
    source::{
        animeku::Animeku, embed, lk21::Lk21, otakudesu::Otakudesu, tokusatsu::Tokusatsuindo, Source,
    },
    util::{
        clearscreen_and_show_banner, error, info, loading, open_in_browser, print_rule,
        show_meta_card, truncate, warn,
    },
};

mod input;
mod models;
mod source;
mod util;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Animeku,
    Otakudesu,
    Lk21,
    Tokusatsu,
}

enum BackendKind {
    Animeku(Animeku),
    Otakudesu(Otakudesu),
    Lk21(Lk21),
    Tokusatsu(Tokusatsuindo),
}

struct Backend {
    kind: BackendKind,
    forced: Option<SourceKind>,
}

impl Backend {
    fn new(forced: Option<SourceKind>) -> Self {
        let kind = match forced {
            Some(SourceKind::Otakudesu) => BackendKind::Otakudesu(Otakudesu::new()),
            Some(SourceKind::Lk21) => BackendKind::Lk21(Lk21::new()),
            Some(SourceKind::Tokusatsu) => BackendKind::Tokusatsu(Tokusatsuindo::new()),
            _ => BackendKind::Animeku(Animeku::new()),
        };

        Self { kind, forced }
    }

    fn name(&self) -> &'static str {
        match &self.kind {
            BackendKind::Animeku(source) => source.name(),
            BackendKind::Otakudesu(source) => source.name(),
            BackendKind::Lk21(source) => source.name(),
            BackendKind::Tokusatsu(source) => source.name(),
        }
    }

    fn allow_fallback(&self) -> bool {
        self.forced.is_none()
    }

    async fn search(&mut self, query: &str, mode: SearchMode) -> anyhow::Result<Vec<Movie>> {
        if matches!(self.kind, BackendKind::Lk21(_) | BackendKind::Tokusatsu(_)) {
            return match &mut self.kind {
                BackendKind::Lk21(source) => source.search(query, mode).await,
                BackendKind::Tokusatsu(source) => source.search(query, mode).await,
                _ => unreachable!(),
            };
        }

        if self.allow_fallback() && matches!(self.kind, BackendKind::Otakudesu(_)) {
            let mut primary = Animeku::new();
            if let Ok(items) = primary.search(query, mode).await {
                self.kind = BackendKind::Animeku(primary);
                warn("AnimeKu kembali normal, memakai sumber utama.");
                return Ok(items);
            }
        }

        if let BackendKind::Animeku(_) = self.kind {
            let result = match &mut self.kind {
                BackendKind::Animeku(source) => source.search(query, mode).await,
                _ => unreachable!(),
            };

            match result {
                Ok(items) => return Ok(items),
                Err(error) if self.allow_fallback() => {
                    warn(format!("AnimeKu gagal: {error:#}"));
                    let mut fallback = Otakudesu::new();
                    let fallback_mode = if mode == SearchMode::Movie {
                        SearchMode::All
                    } else {
                        mode
                    };

                    match fallback.search(query, fallback_mode).await {
                        Ok(items) => {
                            self.kind = BackendKind::Otakudesu(fallback);
                            warn("Beralih ke Otakudesu untuk sesi ini.");
                            return Ok(items);
                        }
                        Err(fallback_error) => {
                            anyhow::bail!("AnimeKu: {error:#}\nOtakudesu: {fallback_error:#}");
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }

        match &mut self.kind {
            BackendKind::Otakudesu(source) => {
                let fallback_mode = if mode == SearchMode::Movie {
                    SearchMode::All
                } else {
                    mode
                };
                source.search(query, fallback_mode).await
            }
            BackendKind::Animeku(_) | BackendKind::Lk21(_) | BackendKind::Tokusatsu(_) => {
                unreachable!()
            }
        }
    }

    async fn search_listing(
        &mut self,
        query: Option<&str>,
        mode: SearchMode,
    ) -> anyhow::Result<Listing> {
        if matches!(self.kind, BackendKind::Lk21(_) | BackendKind::Tokusatsu(_)) {
            return match &mut self.kind {
                BackendKind::Lk21(source) => source.search_listing(query.unwrap_or(""), mode).await,
                BackendKind::Tokusatsu(source) => {
                    source.search_listing(query.unwrap_or(""), mode).await
                }
                _ => unreachable!(),
            };
        }

        if let BackendKind::Animeku(_) = self.kind {
            let result = match &mut self.kind {
                BackendKind::Animeku(source) => {
                    source.search_listing(query.unwrap_or(""), mode).await
                }
                _ => unreachable!(),
            };

            match result {
                Ok(listing) => return Ok(listing),
                Err(error) if self.allow_fallback() => {
                    let Some(query) = query else {
                        anyhow::bail!(
                            "AnimeKu sedang tidak bisa diakses: {error:#}\nCoba menu 'Cari anime' atau ulangi beberapa saat lagi."
                        );
                    };

                    warn(format!("AnimeKu gagal: {error:#}"));
                    let mut fallback = Otakudesu::new();
                    let fallback_mode = if mode == SearchMode::Movie {
                        SearchMode::All
                    } else {
                        mode
                    };

                    match fallback.search(query, fallback_mode).await {
                        Ok(items) => {
                            self.kind = BackendKind::Otakudesu(fallback);
                            warn("Beralih ke Otakudesu untuk sesi ini.");
                            return Ok(Listing { items, next: None });
                        }
                        Err(fallback_error) => {
                            anyhow::bail!("AnimeKu: {error:#}\nOtakudesu: {fallback_error:#}");
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }

        let Some(query) = query else {
            anyhow::bail!("Jelajah halaman hanya tersedia saat memakai AnimeKu.");
        };

        match &mut self.kind {
            BackendKind::Otakudesu(source) => {
                let fallback_mode = if mode == SearchMode::Movie {
                    SearchMode::All
                } else {
                    mode
                };
                let items = source.search(query, fallback_mode).await?;
                Ok(Listing { items, next: None })
            }
            BackendKind::Animeku(_) | BackendKind::Lk21(_) | BackendKind::Tokusatsu(_) => {
                unreachable!()
            }
        }
    }

    async fn listing(&mut self, url: &str) -> anyhow::Result<Listing> {
        match &mut self.kind {
            BackendKind::Animeku(source) => source.listing(url).await,
            BackendKind::Lk21(source) => source.listing(url).await,
            BackendKind::Otakudesu(_) | BackendKind::Tokusatsu(_) => {
                anyhow::bail!("Halaman berikutnya tidak tersedia untuk sumber ini.")
            }
        }
    }

    async fn episodes(&self, movie: &Movie) -> anyhow::Result<(Vec<Episode>, Meta)> {
        match &self.kind {
            BackendKind::Animeku(source) => source.episodes(movie).await,
            BackendKind::Otakudesu(source) => source.episodes(movie).await,
            BackendKind::Lk21(source) => source.episodes(movie).await,
            BackendKind::Tokusatsu(source) => source.episodes(movie).await,
        }
    }

    async fn streams(&self, episode: &Episode) -> anyhow::Result<Vec<Stream>> {
        match &self.kind {
            BackendKind::Animeku(source) => source.streams(episode).await,
            BackendKind::Otakudesu(source) => source.streams(episode).await,
            BackendKind::Lk21(source) => source.streams(episode).await,
            BackendKind::Tokusatsu(source) => source.streams(episode).await,
        }
    }

    async fn resolve(&self, stream: &Stream) -> anyhow::Result<Option<String>> {
        match &self.kind {
            BackendKind::Animeku(source) => source.resolve(stream).await,
            BackendKind::Otakudesu(source) => source.resolve(stream).await,
            BackendKind::Lk21(source) => source.resolve(stream).await,
            BackendKind::Tokusatsu(source) => source.resolve(stream).await,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("\n {} {}\n", "■".red(), format!("{error:#}").yellow());
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("wibu-cli v{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }

    let source = parse_source_arg(&args)?;

    if let Some(position) = args.iter().position(|arg| arg == "--resolve") {
        let url = args
            .get(position + 1)
            .map(String::as_str)
            .ok_or_else(|| anyhow::anyhow!("--resolve butuh URL embed"))?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        return runtime.block_on(resolve_probe(url));
    }

    if let Some(position) = args.iter().position(|arg| arg == "--probe") {
        let query = args
            .get(position + 1)
            .map(String::as_str)
            .unwrap_or("frieren");
        let stream_index = args
            .get(position + 2)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        return runtime.block_on(probe(query, stream_index, source));
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!(
            "{} wibu-cli harus dijalankan langsung di terminal interaktif.",
            "■".red()
        );
        eprintln!("   Contoh: wibu-cli");
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(app(source))
}

fn parse_source_arg(args: &[String]) -> anyhow::Result<Option<SourceKind>> {
    let Some(position) = args.iter().position(|arg| arg == "--source") else {
        return Ok(None);
    };

    let value = args
        .get(position + 1)
        .map(|value| value.to_lowercase())
        .ok_or_else(|| {
            anyhow::anyhow!("--source butuh nilai animeku, otakudesu, jepang, atau tokusatsu")
        })?;

    match value.as_str() {
        "animeku" | "animeku.lol" | "aku" => Ok(Some(SourceKind::Animeku)),
        "otakudesu" | "otakudesu.blog" | "otaku" => Ok(Some(SourceKind::Otakudesu)),
        "jepang" | "lk21" | "layarkaca21" | "film-jepang" => Ok(Some(SourceKind::Lk21)),
        "tokusatsu" | "toku" | "tokusatsuindo" => Ok(Some(SourceKind::Tokusatsu)),
        other => anyhow::bail!(
            "Sumber '{other}' tidak dikenal. Gunakan animeku, otakudesu, jepang, atau tokusatsu."
        ),
    }
}

fn print_help() {
    println!(
        "wibu-cli v{}

Streaming anime, film Jepang, dan tokusatsu lewat terminal, langsung ke MPV.

Pemakaian:
  wibu-cli                     jalankan mode interaktif
  wibu-cli --version           tampilkan versi
  wibu-cli --help              tampilkan bantuan
  wibu-cli --source animeku    paksa sumber AnimeKu (default)
  wibu-cli --source otakudesu  paksa sumber Otakudesu
  wibu-cli --source jepang     pakai LK21 untuk film/series Jepang
  wibu-cli --source tokusatsu  pakai Tokusatsuindo untuk Ultraman dkk.
  wibu-cli --probe <judul> [n] uji pencarian, episode, dan resolve
  wibu-cli --resolve <url>     uji resolve satu URL embed

Menu interaktif juga punya Film Jepang, Series & drama Jepang, dan
Tokusatsu (Ultraman, Kamen Rider, Super Sentai).

Variabel lingkungan:
  WIBU_ANIMEKU_BASE     ganti domain AnimeKu (default: https://animeku.lol)
  WIBU_LK21_BASE        ganti domain LK21 (default: https://layarkaca21.health)
  WIBU_TOKUSATSU_BASE   ganti domain Tokusatsuindo
  WIBU_NO_IMAGE=1       matikan poster di kartu metadata
  WIBU_BROWSER=1        selalu buka browser, jangan pakai MPV

Sumber anime: AnimeKu (https://animeku.lol), cadangan Otakudesu.
Sumber Jepang: LK21 (film & series) dan Tokusatsuindo (Ultraman dkk.).
Dibuat oleh panz-08 - https://github.com/Panz-08/wibu-cli",
        env!("CARGO_PKG_VERSION")
    );
}

async fn app(source: Option<SourceKind>) -> anyhow::Result<()> {
    clearscreen_and_show_banner()?;

    loop {
        let action = match main_menu() {
            Ok(action) => action,
            Err(_) => break,
        };

        let result = match action {
            MenuAction::Search => {
                let mut backend = anime_backend(source);
                search_flow(&mut backend, SearchMode::All).await
            }
            MenuAction::Browse(mode) => {
                let mut backend = anime_backend(source);
                search_flow(&mut backend, mode).await
            }
            MenuAction::Japan(mode) => {
                let japan_source = if mode == SearchMode::Tokusatsu {
                    SourceKind::Tokusatsu
                } else {
                    SourceKind::Lk21
                };

                let mut backend = Backend::new(Some(japan_source));
                search_flow(&mut backend, mode).await
            }
            MenuAction::Quit => break,
        };

        if let Err(err) = result {
            clearscreen_and_show_banner()?;
            error(format!("{err:#}"));
            let _ = confirm("Kembali ke menu?", true);
        }

        clearscreen_and_show_banner()?;
    }

    println!("\n  {} Sampai jumpa, wibu! ヽ(・∀・)ﾉ\n", "◆".bright_cyan());

    Ok(())
}

fn anime_backend(forced: Option<SourceKind>) -> Backend {
    let forced = match forced {
        Some(SourceKind::Animeku) => Some(SourceKind::Animeku),
        Some(SourceKind::Otakudesu) => Some(SourceKind::Otakudesu),
        _ => None,
    };

    Backend::new(forced)
}

async fn search_flow(backend: &mut Backend, mode: SearchMode) -> anyhow::Result<()> {
    let query = match mode {
        SearchMode::All => Some(search_prompt("Cari judul anime")?),
        SearchMode::JapanMovie => optional_query("Cari film Jepang (kosongkan untuk jelajah)")?,
        SearchMode::JapanSeries => {
            optional_query("Cari series/drama Jepang (kosongkan untuk jelajah)")?
        }
        SearchMode::Tokusatsu => optional_query("Cari tokusatsu (kosongkan untuk jelajah)")?,
        _ => optional_query(&format!(
            "Cari di {} (kosongkan untuk jelajah)",
            mode.short_label()
        ))?,
    };

    clearscreen_and_show_banner()?;

    match &query {
        Some(query) => loading(format!("Mencari '{}' ...", query.green())),
        None => loading(format!("Memuat daftar {} ...", mode.label())),
    }

    let listing = match backend.search_listing(query.as_deref(), mode).await {
        Ok(listing) => {
            println!("{} judul", listing.items.len().to_string().green());
            listing
        }
        Err(error) => {
            println!("gagal");
            return Err(error);
        }
    };

    if listing.items.is_empty() {
        warn("Tidak ada judul yang cocok.");
        return Ok(());
    }

    let prompt = if mode.is_japan() {
        "Pilih judul"
    } else {
        "Pilih anime"
    };
    let Some(movie) = pick_listing(backend, listing, prompt).await? else {
        return Ok(());
    };

    show_anime(backend, movie).await
}

async fn pick_listing(
    backend: &mut Backend,
    listing: Listing,
    prompt: &str,
) -> anyhow::Result<Option<Movie>> {
    let mut current = listing;
    let mut history: Vec<Listing> = vec![];

    loop {
        let mut options = vec![];
        options.extend(current.items.iter().cloned().map(Pick::Movie));
        if current.next.is_some() {
            options.push(Pick::Next);
        }
        if !history.is_empty() {
            options.push(Pick::Previous);
        }
        options.push(Pick::Back);

        match choice(options, prompt, false)? {
            Pick::Movie(movie) => return Ok(Some(movie)),
            Pick::Back => return Ok(None),
            Pick::Previous => {
                if let Some(previous) = history.pop() {
                    current = previous;
                }
            }
            Pick::Next => {
                let Some(next) = current.next.clone() else {
                    continue;
                };

                loading("Memuat halaman berikutnya ...");
                match backend.listing(&next).await {
                    Ok(listing) => {
                        println!("{} judul", listing.items.len().to_string().green());
                        history.push(current);
                        current = listing;
                    }
                    Err(err) => {
                        println!("gagal");
                        error(format!("{err:#}"));
                    }
                }
            }
        }
    }
}

async fn show_anime(backend: &mut Backend, movie: Movie) -> anyhow::Result<()> {
    clearscreen_and_show_banner()?;
    loading(format!("Memuat detail '{}' ...", movie.title.green()));

    let (episodes, mut meta) = match backend.episodes(&movie).await {
        Ok(result) => {
            println!("berhasil");
            result
        }
        Err(error) => {
            println!("gagal");
            return Err(error);
        }
    };

    // Poster dari halaman daftar dipakai kalau halaman detail tidak
    // menyertakan gambar.
    if meta.thumb_url.is_none() {
        meta.thumb_url = movie.poster_url.clone();
    }

    clearscreen_and_show_banner()?;
    show_meta_card(&movie.title, &meta, backend.name()).await;

    if episodes.is_empty() {
        warn("Tidak ada episode yang bisa diputar dari judul ini.");
        return Ok(());
    }

    info(format!(
        "{} episode tersedia",
        episodes.len().to_string().green()
    ));

    if !meta.downloads.is_empty() && confirm("Tampilkan link download?", false)? {
        print_downloads(&meta);
    }

    if episodes.len() == 1 {
        return match play_episode(backend, &movie, &episodes[0]).await {
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        };
    }

    loop {
        let mut options: Vec<EpisodePick> =
            episodes.iter().cloned().map(EpisodePick::Episode).collect();
        options.push(EpisodePick::Back);

        let picked = choice(options, "Pilih episode", true)?;
        let episode = match picked {
            EpisodePick::Back => return Ok(()),
            EpisodePick::Episode(episode) => episode,
        };

        match play_episode(backend, &movie, &episode).await {
            Ok(played) => {
                if !played || !episode.is_series {
                    return Ok(());
                }

                if !confirm("Tonton episode lain?", true)? {
                    return Ok(());
                }
            }
            Err(err) => {
                error(format!("{err:#}"));
                if !confirm("Coba episode lain?", true)? {
                    return Ok(());
                }
            }
        }
    }
}

async fn play_episode(backend: &Backend, movie: &Movie, episode: &Episode) -> anyhow::Result<bool> {
    loading(format!("Menyiapkan '{}' ...", episode.title.green()));

    let streams = match backend.streams(episode).await {
        Ok(streams) => {
            if streams.is_empty() {
                println!("gagal");
                warn("Tidak ada server streaming untuk episode ini.");
                return Ok(false);
            }
            println!("{} server", streams.len().to_string().green());
            streams
        }
        Err(error) => {
            println!("gagal");
            return Err(error);
        }
    };

    let stream = if streams.len() == 1 {
        streams[0].clone()
    } else {
        choice(streams, "Pilih kualitas / server", false)?
    };

    loading("Menyiapkan tautan video ...");
    let url = match backend.resolve(&stream).await {
        Ok(Some(url)) => {
            println!("berhasil");
            url
        }
        Ok(None) => {
            println!("gagal");
            warn("Server ini tidak mengembalikan tautan video.");
            offer_browser(episode)?;
            return Ok(false);
        }
        Err(err) => {
            println!("gagal");
            error(format!("{err:#}"));
            offer_browser(episode)?;
            return Ok(false);
        }
    };

    println!("  {} {}", "↳".cyan(), truncate(&url, 110).dimmed());

    if browser_forced() {
        open_in_browser(&episode.id)?;
        return Ok(true);
    }

    if !crate::util::mpv_available() {
        warn("MPV tidak ditemukan di PATH.");
        if confirm("Buka di browser sebagai gantinya?", true)? {
            open_in_browser(&episode.id)?;
            return Ok(true);
        }
        return Ok(false);
    }

    let title = if episode.title == movie.title {
        movie.title.clone()
    } else {
        format!("{} - {}", movie.title, episode.title)
    };
    let referer = stream.referer.as_deref().unwrap_or(&episode.id);
    match crate::util::play_with_mpv(&url, &title, referer) {
        Ok(()) => Ok(true),
        Err(err) => {
            error(format!("{err:#}"));
            if confirm("Buka di browser sebagai gantinya?", true)? {
                open_in_browser(&episode.id)?;
            }
            Ok(false)
        }
    }
}

fn offer_browser(episode: &Episode) -> anyhow::Result<()> {
    if confirm("Buka halaman episode di browser?", false)? {
        open_in_browser(&episode.id)?;
    }
    Ok(())
}

fn print_downloads(meta: &Meta) {
    print_rule("Download");

    for download in &meta.downloads {
        let size = download
            .size
            .as_ref()
            .map(|size| format!(" ({size})"))
            .unwrap_or_default();

        println!("\n  {}{}", download.quality.bright_yellow(), size.dimmed());

        for link in &download.links {
            println!("    {} {}", link.label.bright_white(), link.url.dimmed());
        }
    }

    println!();
}

fn browser_forced() -> bool {
    std::env::var("WIBU_BROWSER")
        .map(|value| {
            let value = value.to_lowercase();
            value == "1" || value == "true" || value == "yes"
        })
        .unwrap_or(false)
}

async fn probe(query: &str, stream_index: usize, source: Option<SourceKind>) -> anyhow::Result<()> {
    let mut backend = Backend::new(source);

    println!("[1/5] sumber: {}", backend.name());
    println!("[2/5] mencari '{query}' ...");
    let movies = backend.search(query, SearchMode::All).await?;
    for movie in movies.iter().take(5) {
        println!("  - {movie}");
    }

    let Some(movie) = movies.first() else {
        println!("tidak ada hasil");
        return Ok(());
    };

    println!("[3/5] memuat episode '{}' ...", movie.title);
    let (episodes, meta) = backend.episodes(movie).await?;
    println!(
        "  {} episode, {} info, {} link download",
        episodes.len(),
        meta.data.len(),
        meta.downloads.len()
    );
    for episode in episodes.iter().take(4) {
        println!("  - {episode}");
    }

    let Some(episode) = episodes.first() else {
        println!("tidak ada episode");
        return Ok(());
    };

    println!("[4/5] memuat server '{}' ...", episode.title);
    let streams = backend.streams(episode).await?;
    for stream in streams.iter().take(8) {
        println!("  - {stream}");
    }

    let Some(stream) = streams.get(stream_index).or_else(|| streams.first()) else {
        println!("tidak ada server");
        return Ok(());
    };

    println!("[5/5] resolve '{}' ...", stream.title);
    match backend.resolve(stream).await? {
        Some(url) => println!("  OK: {}", truncate(&url, 180)),
        None => println!("  GAGAL resolve"),
    }

    Ok(())
}

async fn resolve_probe(url: &str) -> anyhow::Result<()> {
    let client = Client::builder()
        .user_agent(embed::USER_AGENT)
        .timeout(Duration::from_secs(25))
        .build()?;

    println!("resolve: {url}");

    let mut resolved = None;
    if url.contains("player.tokusatsuindo.com") || url.contains("drive.google.com") {
        resolved =
            Tokusatsuindo::resolve_player_url(&client, url, "https://www.tokusatsuindo.com/").await;
    }

    if resolved.is_none() {
        resolved = embed::resolve(&client, url, "https://animeku.lol/").await;
    }

    match resolved {
        Some(resolved) => println!("OK: {resolved}"),
        None => println!("GAGAL resolve"),
    }

    Ok(())
}

#[derive(Clone)]
enum Pick {
    Movie(Movie),
    Next,
    Previous,
    Back,
}

impl fmt::Display for Pick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pick::Movie(movie) => write!(f, "{movie}"),
            Pick::Next => write!(f, "» Halaman berikutnya"),
            Pick::Previous => write!(f, "« Halaman sebelumnya"),
            Pick::Back => write!(f, "« Kembali ke menu"),
        }
    }
}

#[derive(Clone)]
enum EpisodePick {
    Episode(Episode),
    Back,
}

impl fmt::Display for EpisodePick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EpisodePick::Episode(episode) => write!(f, "{episode}"),
            EpisodePick::Back => write!(f, "« Kembali ke menu"),
        }
    }
}
