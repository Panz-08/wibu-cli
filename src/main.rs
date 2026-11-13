use std::{
    fmt,
    io::{self, IsTerminal},
    time::Duration,
};

use colored::Colorize;
use reqwest::Client;

use crate::{
    history::HistoryEntry,
    input::{choice, confirm, main_menu, optional_query, search_prompt, MenuAction},
    models::{Episode, Listing, Meta, Movie, SearchMode, Stream},
    source::{
        animeku::Animeku, embed, lk21::Lk21, otakudesu::Otakudesu, samehadaku::Samehadaku,
        tokusatsu::Tokusatsuindo, Source,
    },
    util::{
        clearscreen_and_show_banner, error, info, loading, open_in_browser, print_rule,
        show_meta_card, truncate, warn,
    },
};

mod history;
mod input;
mod models;
mod source;
mod util;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Animeku,
    Otakudesu,
    Samehadaku,
    Lk21,
    Tokusatsu,
}

enum BackendKind {
    Animeku(Animeku),
    Otakudesu(Otakudesu),
    Samehadaku(Samehadaku),
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
            Some(SourceKind::Samehadaku) => BackendKind::Samehadaku(Samehadaku::new()),
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
            BackendKind::Samehadaku(source) => source.name(),
            BackendKind::Lk21(source) => source.name(),
            BackendKind::Tokusatsu(source) => source.name(),
        }
    }

    fn allow_fallback(&self) -> bool {
        self.forced.is_none()
    }

    fn source_id(&self) -> &'static str {
        match &self.kind {
            BackendKind::Animeku(_) => "animeku",
            BackendKind::Otakudesu(_) => "otakudesu",
            BackendKind::Samehadaku(_) => "samehadaku",
            BackendKind::Lk21(_) => "jepang",
            BackendKind::Tokusatsu(_) => "tokusatsu",
        }
    }

    async fn search(&mut self, query: &str, mode: SearchMode) -> anyhow::Result<Vec<Movie>> {
        if matches!(self.kind, BackendKind::Lk21(_) | BackendKind::Tokusatsu(_)) {
            return match &mut self.kind {
                BackendKind::Lk21(source) => source.search(query, mode).await,
                BackendKind::Tokusatsu(source) => source.search(query, mode).await,
                _ => unreachable!(),
            };
        }

        if self.allow_fallback()
            && matches!(
                self.kind,
                BackendKind::Otakudesu(_) | BackendKind::Samehadaku(_)
            )
        {
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
                    let fallback_mode = if mode == SearchMode::Movie {
                        SearchMode::All
                    } else {
                        mode
                    };

                    let mut otakudesu = Otakudesu::new();
                    match otakudesu.search(query, fallback_mode).await {
                        Ok(items) => {
                            self.kind = BackendKind::Otakudesu(otakudesu);
                            warn("Beralih ke Otakudesu untuk sesi ini.");
                            return Ok(items);
                        }
                        Err(otaku_error) => {
                            warn(format!("Otakudesu gagal: {otaku_error:#}"));
                            let mut samehadaku = Samehadaku::new();
                            match samehadaku.search(query, fallback_mode).await {
                                Ok(items) => {
                                    self.kind = BackendKind::Samehadaku(samehadaku);
                                    warn("Beralih ke Samehadaku untuk sesi ini.");
                                    return Ok(items);
                                }
                                Err(sameh_error) => {
                                    anyhow::bail!(
                                        "AnimeKu: {error:#}\nOtakudesu: {otaku_error:#}\nSamehadaku: {sameh_error:#}"
                                    );
                                }
                            }
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }

        if let BackendKind::Otakudesu(_) = self.kind {
            let fallback_mode = if mode == SearchMode::Movie {
                SearchMode::All
            } else {
                mode
            };
            let result = match &mut self.kind {
                BackendKind::Otakudesu(source) => source.search(query, fallback_mode).await,
                _ => unreachable!(),
            };

            return match result {
                Ok(items) => Ok(items),
                Err(error) if self.allow_fallback() => {
                    warn(format!("Otakudesu gagal: {error:#}"));
                    let mut samehadaku = Samehadaku::new();
                    match samehadaku.search(query, fallback_mode).await {
                        Ok(items) => {
                            self.kind = BackendKind::Samehadaku(samehadaku);
                            warn("Beralih ke Samehadaku untuk sesi ini.");
                            Ok(items)
                        }
                        Err(sameh_error) => {
                            anyhow::bail!("Otakudesu: {error:#}\nSamehadaku: {sameh_error:#}")
                        }
                    }
                }
                Err(error) => Err(error),
            };
        }

        match &mut self.kind {
            BackendKind::Samehadaku(source) => source.search(query, mode).await,
            BackendKind::Animeku(_)
            | BackendKind::Otakudesu(_)
            | BackendKind::Lk21(_)
            | BackendKind::Tokusatsu(_) => unreachable!(),
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
                    let fallback_mode = if mode == SearchMode::Movie {
                        SearchMode::All
                    } else {
                        mode
                    };

                    let mut otakudesu = Otakudesu::new();
                    match otakudesu.search(query, fallback_mode).await {
                        Ok(items) => {
                            self.kind = BackendKind::Otakudesu(otakudesu);
                            warn("Beralih ke Otakudesu untuk sesi ini.");
                            return Ok(Listing { items, next: None });
                        }
                        Err(otaku_error) => {
                            warn(format!("Otakudesu gagal: {otaku_error:#}"));
                            let mut samehadaku = Samehadaku::new();
                            match samehadaku.search(query, fallback_mode).await {
                                Ok(items) => {
                                    self.kind = BackendKind::Samehadaku(samehadaku);
                                    warn("Beralih ke Samehadaku untuk sesi ini.");
                                    return Ok(Listing { items, next: None });
                                }
                                Err(sameh_error) => {
                                    anyhow::bail!(
                                        "AnimeKu: {error:#}\nOtakudesu: {otaku_error:#}\nSamehadaku: {sameh_error:#}"
                                    );
                                }
                            }
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }

        let Some(query) = query else {
            anyhow::bail!("Jelajah halaman hanya tersedia saat memakai AnimeKu.");
        };

        let fallback_mode = if mode == SearchMode::Movie {
            SearchMode::All
        } else {
            mode
        };

        if let BackendKind::Otakudesu(_) = self.kind {
            let result = match &mut self.kind {
                BackendKind::Otakudesu(source) => source.search(query, fallback_mode).await,
                _ => unreachable!(),
            };

            return match result {
                Ok(items) => Ok(Listing { items, next: None }),
                Err(error) if self.allow_fallback() => {
                    warn(format!("Otakudesu gagal: {error:#}"));
                    let mut samehadaku = Samehadaku::new();
                    match samehadaku.search(query, fallback_mode).await {
                        Ok(items) => {
                            self.kind = BackendKind::Samehadaku(samehadaku);
                            warn("Beralih ke Samehadaku untuk sesi ini.");
                            Ok(Listing { items, next: None })
                        }
                        Err(sameh_error) => {
                            anyhow::bail!("Otakudesu: {error:#}\nSamehadaku: {sameh_error:#}")
                        }
                    }
                }
                Err(error) => Err(error),
            };
        }

        match &mut self.kind {
            BackendKind::Samehadaku(source) => {
                let items = source.search(query, fallback_mode).await?;
                Ok(Listing { items, next: None })
            }
            BackendKind::Animeku(_)
            | BackendKind::Otakudesu(_)
            | BackendKind::Lk21(_)
            | BackendKind::Tokusatsu(_) => unreachable!(),
        }
    }

    async fn listing(&mut self, url: &str) -> anyhow::Result<Listing> {
        match &mut self.kind {
            BackendKind::Animeku(source) => source.listing(url).await,
            BackendKind::Lk21(source) => source.listing(url).await,
            BackendKind::Otakudesu(_) | BackendKind::Samehadaku(_) | BackendKind::Tokusatsu(_) => {
                anyhow::bail!("Halaman berikutnya tidak tersedia untuk sumber ini.")
            }
        }
    }

    async fn episodes(&self, movie: &Movie) -> anyhow::Result<(Vec<Episode>, Meta)> {
        match &self.kind {
            BackendKind::Animeku(source) => source.episodes(movie).await,
            BackendKind::Otakudesu(source) => source.episodes(movie).await,
            BackendKind::Samehadaku(source) => source.episodes(movie).await,
            BackendKind::Lk21(source) => source.episodes(movie).await,
            BackendKind::Tokusatsu(source) => source.episodes(movie).await,
        }
    }

    async fn streams(&self, episode: &Episode) -> anyhow::Result<Vec<Stream>> {
        match &self.kind {
            BackendKind::Animeku(source) => source.streams(episode).await,
            BackendKind::Otakudesu(source) => source.streams(episode).await,
            BackendKind::Samehadaku(source) => source.streams(episode).await,
            BackendKind::Lk21(source) => source.streams(episode).await,
            BackendKind::Tokusatsu(source) => source.streams(episode).await,
        }
    }

    async fn resolve(&self, stream: &Stream) -> anyhow::Result<Option<String>> {
        match &self.kind {
            BackendKind::Animeku(source) => source.resolve(stream).await,
            BackendKind::Otakudesu(source) => source.resolve(stream).await,
            BackendKind::Samehadaku(source) => source.resolve(stream).await,
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

fn kind_from_id(value: &str) -> Option<SourceKind> {
    match value {
        "animeku" => Some(SourceKind::Animeku),
        "otakudesu" => Some(SourceKind::Otakudesu),
        "samehadaku" => Some(SourceKind::Samehadaku),
        "jepang" => Some(SourceKind::Lk21),
        "tokusatsu" => Some(SourceKind::Tokusatsu),
        _ => None,
    }
}

fn parse_source_arg(args: &[String]) -> anyhow::Result<Option<SourceKind>> {
    let Some(position) = args.iter().position(|arg| arg == "--source") else {
        return Ok(None);
    };

    let value = args
        .get(position + 1)
        .map(|value| value.to_lowercase())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "--source butuh nilai animeku, otakudesu, samehadaku, jepang, atau tokusatsu"
            )
        })?;

    match value.as_str() {
        "animeku" | "animeku.lol" | "aku" => Ok(Some(SourceKind::Animeku)),
        "otakudesu" | "otakudesu.blog" | "otaku" => Ok(Some(SourceKind::Otakudesu)),
        "samehadaku" | "samehadaku.today" | "sameh" => Ok(Some(SourceKind::Samehadaku)),
        "jepang" | "lk21" | "layarkaca21" | "film-jepang" => Ok(Some(SourceKind::Lk21)),
        "tokusatsu" | "toku" | "tokusatsuindo" => Ok(Some(SourceKind::Tokusatsu)),
        other => anyhow::bail!(
            "Sumber '{other}' tidak dikenal. Gunakan animeku, otakudesu, samehadaku, jepang, atau tokusatsu."
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
  wibu-cli --source samehadaku  paksa sumber Samehadaku (cadangan ketiga)
  wibu-cli --source jepang     pakai LK21 untuk film/series Jepang
  wibu-cli --source tokusatsu  pakai Tokusatsuindo untuk Ultraman dkk.
  wibu-cli --probe <judul> [n] uji pencarian, episode, dan resolve
  wibu-cli --resolve <url>     uji resolve satu URL embed

Menu interaktif juga punya Lanjutkan tontonan, Film Jepang, Series & drama
Jepang, dan Tokusatsu (Ultraman, Kamen Rider, Super Sentai).
Riwayat tontonan disimpan di ~/.local/share/wibu-cli/history.json (mengikuti
XDG_DATA_HOME). Hapus lewat menu Lanjutkan tontonan atau hapus file tersebut.

Variabel lingkungan:
  WIBU_ANIMEKU_BASE     ganti domain AnimeKu (default: https://animeku.lol)
  WIBU_SAMEHADAKU_BASE  ganti domain Samehadaku (default: https://samehadaku.today)
  WIBU_LK21_BASE        ganti domain LK21 (default: https://layarkaca21.health)
  WIBU_TOKUSATSU_BASE   ganti domain Tokusatsuindo
  WIBU_NO_IMAGE=1       matikan poster di kartu metadata
  WIBU_BROWSER=1        selalu buka browser, jangan pakai MPV
  WIBU_DATA_DIR=<dir>   lokasi penyimpanan riwayat tontonan

Sumber anime: AnimeKu (https://animeku.lol), cadangan Otakudesu dan Samehadaku.
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
            MenuAction::Continue => continue_flow().await,
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
        Some(SourceKind::Samehadaku) => Some(SourceKind::Samehadaku),
        _ => None,
    };

    Backend::new(forced)
}

async fn continue_flow() -> anyhow::Result<()> {
    let entries = history::load();
    if entries.is_empty() {
        warn("Belum ada riwayat tontonan. Putar satu episode dulu, nanti muncul di sini.");
        return Ok(());
    }

    let mut options: Vec<HistoryPick> = entries.into_iter().map(HistoryPick::Entry).collect();
    options.push(HistoryPick::Clear);
    options.push(HistoryPick::Back);

    clearscreen_and_show_banner()?;
    match choice(options, "Lanjutkan tontonan", true)? {
        HistoryPick::Back => Ok(()),
        HistoryPick::Clear => {
            history::clear()?;
            info("Riwayat tontonan dihapus.");
            Ok(())
        }
        HistoryPick::Entry(entry) => {
            let Some(kind) = kind_from_id(&entry.source) else {
                warn("Sumber episode ini tidak dikenal lagi. Riwayat mungkin dari versi lama.");
                return Ok(());
            };

            let mut backend = Backend::new(Some(kind));
            let movie = Movie {
                id: entry.movie_id.clone(),
                title: entry.title.clone(),
                info: entry.info.clone(),
                poster_url: entry.poster_url.clone(),
            };
            let episode = Episode {
                id: entry.episode_id.clone(),
                title: entry.episode_title.clone(),
                number: entry.episode_number,
                is_series: entry.is_series,
            };

            clearscreen_and_show_banner()?;
            match play_episode(&backend, &movie, &episode).await {
                Ok(true) if episode.is_series => {
                    if confirm("Buka daftar episode untuk lanjut?", true)? {
                        show_anime(&mut backend, movie).await?;
                    }
                    Ok(())
                }
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            }
        }
    }
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

    // Server yang dipilih pengguna dicoba lebih dulu, lalu server lain dipakai
    // otomatis kalau resolve atau MPV gagal. Ini penting karena satu file
    // Google Drive bisa kena kuota sementara server lain masih normal.
    let ordered = if streams.len() == 1 {
        streams
    } else {
        let picked = choice(streams.clone(), "Pilih kualitas / server", false)?;
        let mut ordered = vec![picked.clone()];
        ordered.extend(
            streams
                .into_iter()
                .filter(|stream| stream.title != picked.title),
        );
        ordered
    };

    if browser_forced() {
        history::record(backend.source_id(), movie, episode);
        open_in_browser(&episode.id)?;
        return Ok(true);
    }

    if !crate::util::mpv_available() {
        warn("MPV tidak ditemukan di PATH.");
        if confirm("Buka di browser sebagai gantinya?", true)? {
            history::record(backend.source_id(), movie, episode);
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

    let probe_client = Client::builder()
        .user_agent(embed::USER_AGENT)
        .timeout(Duration::from_secs(25))
        .build()?;

    let total = ordered.len();
    let mut last_error = None;

    for (index, stream) in ordered.iter().enumerate() {
        if total > 1 {
            info(format!(
                "Server {}/{}: {}",
                (index + 1).to_string().green(),
                total,
                stream.title
            ));
        }

        loading("Menyiapkan tautan video ...");
        let url = match backend.resolve(stream).await {
            Ok(Some(url)) => {
                println!("berhasil");
                url
            }
            Ok(None) => {
                println!("gagal");
                warn("Server ini tidak mengembalikan tautan video.");
                continue;
            }
            Err(resolve_error) => {
                println!("gagal");
                warn(format!("Resolve gagal: {resolve_error:#}"));
                last_error = Some(format!("{resolve_error:#}"));
                continue;
            }
        };

        println!("  {} {}", "↳".cyan(), truncate(&url, 110).dimmed());

        if !embed::media_url_playable(&probe_client, &url).await {
            warn("Server ini tidak mengembalikan video yang bisa diputar.");
            last_error = Some(format!("{} tidak valid", stream.title));
            continue;
        }

        let referer = stream.referer.as_deref().unwrap_or(&episode.id);
        match crate::util::play_with_mpv(&url, &title, referer) {
            Ok(()) => {
                history::record(backend.source_id(), movie, episode);
                return Ok(true);
            }
            Err(mpv_error) => {
                error(format!("{mpv_error:#}"));
                last_error = Some(format!("{mpv_error:#}"));
                if index + 1 < total {
                    warn("Mencoba server berikutnya ...");
                }
            }
        }
    }

    if let Some(failure) = last_error {
        error(format!("Semua server gagal. Terakhir: {failure}"));
    } else {
        warn("Semua server gagal diputar.");
    }

    offer_browser(episode)?;
    Ok(false)
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
enum HistoryPick {
    Entry(HistoryEntry),
    Clear,
    Back,
}

impl fmt::Display for HistoryPick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryPick::Entry(entry) => write!(f, "{entry}"),
            HistoryPick::Clear => write!(f, "Hapus riwayat tontonan"),
            HistoryPick::Back => write!(f, "« Kembali ke menu"),
        }
    }
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
