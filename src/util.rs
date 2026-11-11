use std::{
    collections::hash_map::DefaultHasher,
    env,
    hash::{Hash, Hasher},
    io::{self, Write},
    path::PathBuf,
    process::Command,
    time::Duration,
};

use colored::Colorize;
use html_escape::decode_html_entities;
use image::{imageops::FilterType, DynamicImage, GenericImageView};
use reqwest::Client;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::models::Meta;

#[macro_export]
macro_rules! regex {
    ($re:literal $(,)?) => {{
        static RE: once_cell::sync::OnceCell<regex::Regex> = once_cell::sync::OnceCell::new();
        RE.get_or_init(|| regex::Regex::new($re).unwrap())
    }};
}

/// Ubah potongan HTML menjadi teks satu baris.
pub fn clean_text(input: &str) -> String {
    let cleaned = strip_noise_blocks(input);
    let no_tags = regex!(r"(?s)<[^>]*>").replace_all(&cleaned, " ");
    let decoded = decode_html_entities(&no_tags);
    let collapsed = regex!(r"\s+").replace_all(decoded.trim(), " ");
    collapsed
        .replace(" ,", ",")
        .replace(" .", ".")
        .replace(" ;", ";")
        .replace(" :", ":")
        .replace(" !", "!")
        .replace(" ?", "?")
        .replace(" )", ")")
        .replace("( ", "(")
        .trim()
        .to_string()
}

/// Buang blok yang isinya kode, bukan teks bacaan. Beberapa halaman sumber
/// menempelkan script iklan di dalam artikel, jadi jangan sampai ikut tampil.
fn strip_noise_blocks(input: &str) -> String {
    regex!(
        r"(?is)<script\b[^>]*>.*?</script\s*>|<style\b[^>]*>.*?</style\s*>|<noscript\b[^>]*>.*?</noscript\s*>|<!--.*?-->"
    )
    .replace_all(input, " ")
    .to_string()
}

/// Ubah HTML jadi teks yang masih mempertahankan paragraf.
pub fn html_to_text(input: &str) -> String {
    let cleaned = strip_noise_blocks(input);
    let with_breaks = regex!(r"(?i)</p\s*>|<br\s*/?>").replace_all(&cleaned, "\n");
    let no_tags = regex!(r"(?s)<[^>]*>").replace_all(&with_breaks, " ");
    let decoded = decode_html_entities(&no_tags);

    decoded
        .lines()
        .map(|line| regex!(r"\s+").replace_all(line.trim(), " ").to_string())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Bersihkan escape yang sering dipakai di URL hasil JavaScript / JSON.
pub fn unescape_js_url(input: &str) -> String {
    input
        .replace("\\/", "/")
        .replace("\\u003d", "=")
        .replace("\\u0026", "&")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

pub fn truncate(input: &str, max: usize) -> String {
    let mut chars = input.chars();
    let short: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

pub fn truncate_width(input: &str, width: usize) -> String {
    if UnicodeWidthStr::width(input) <= width {
        return input.to_string();
    }

    let mut output = String::new();
    let mut used = 0;
    let width = width.saturating_sub(1);

    for ch in input.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        output.push(ch);
        used += ch_width;
    }

    output.push('…');
    output
}

/// Bungkus teks menjadi beberapa baris tanpa memotong kata.
pub fn wrap_text(input: &str, width: usize) -> Vec<String> {
    let width = width.max(12);
    let mut lines = vec![];

    for paragraph in input.split('\n') {
        let mut current = String::new();
        let mut current_width = 0usize;

        for word in paragraph.split_whitespace() {
            let word_width = UnicodeWidthStr::width(word);

            if word_width > width {
                if !current.is_empty() {
                    lines.push(current);
                    current = String::new();
                    current_width = 0;
                }

                for ch in word.chars() {
                    let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
                    if current_width + ch_width > width && !current.is_empty() {
                        lines.push(current);
                        current = String::new();
                        current_width = 0;
                    }
                    current.push(ch);
                    current_width += ch_width;
                }
                continue;
            }

            if current_width > 0 && current_width + 1 + word_width > width {
                lines.push(current);
                current = String::new();
                current_width = 0;
            }

            if !current.is_empty() {
                current.push(' ');
                current_width += 1;
            }
            current.push_str(word);
            current_width += word_width;
        }

        if !current.is_empty() {
            lines.push(current);
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

pub fn info(message: impl AsRef<str>) {
    println!("  {} {}", "◆".blue(), message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    println!("  {} {}", "!".yellow(), message.as_ref().yellow());
}

pub fn error(message: impl AsRef<str>) {
    println!("  {} {}", "✖".red(), message.as_ref().red());
}

pub fn loading(message: impl AsRef<str>) {
    print!("  {} {} ", "◆".blue(), message.as_ref());
    let _ = io::stdout().flush();
}

pub fn clearscreen_and_show_banner() -> anyhow::Result<()> {
    clearscreen::clear()?;

    let banner = r#"
   ██╗    ██╗██╗██████╗ ██╗   ██╗
   ██║    ██║██║██╔══██╗██║   ██║
   ██║ █╗ ██║██║██████╔╝██║   ██║
   ██║███╗██║██║██╔══██╗██║   ██║
   ╚███╔███╔╝██║██████╔╝╚██████╔╝
    ╚══╝╚══╝ ╚═╝╚═════╝  ╚═════╝"#;

    println!("{}", banner.bright_cyan());
    println!(
        "  {}  {}",
        "Anime, film Jepang & tokusatsu.".bright_white(),
        format!("v{}  ·  panz-08", env!("CARGO_PKG_VERSION")).dimmed()
    );
    println!(
        "  {}  {}",
        "Sumber: AnimeKu · LK21 · Tokusatsuindo".dimmed(),
        "· Ctrl+C keluar.".dimmed()
    );
    println!();

    Ok(())
}

pub fn print_rule(label: &str) {
    let (width, _) = viuer::terminal_size();
    let width = (width as usize).clamp(30, 78);
    let label = format!(" {label} ");
    let line_width = width.saturating_sub(UnicodeWidthStr::width(label.as_str()) + 3);
    println!(
        "\n  {} {} {}",
        "◆".blue(),
        label.bright_white().bold(),
        "─".repeat(line_width).dimmed()
    );
}

pub async fn show_meta_card(title: &str, meta: &Meta, source: &str) {
    let (terminal_width, _) = viuer::terminal_size();
    let terminal_width = terminal_width.max(50) as usize;
    let show_poster = !image_disabled() && terminal_width >= 62;
    let poster_cols = if terminal_width >= 110 {
        22
    } else if terminal_width >= 90 {
        19
    } else {
        15
    };

    let poster = if show_poster {
        load_poster(meta.thumb_url.as_deref(), poster_cols).await
    } else {
        None
    };

    let text_width = match &poster {
        Some(poster) => terminal_width.saturating_sub(poster.width + 5),
        None => terminal_width.saturating_sub(5),
    };

    let text_lines = build_meta_lines(title, meta, source, text_width.max(24));
    println!();

    if let Some(poster) = poster {
        let total = poster.lines.len().max(text_lines.len());
        for index in 0..total {
            let left = poster
                .lines
                .get(index)
                .cloned()
                .unwrap_or_else(|| " ".repeat(poster.width));
            let right = text_lines.get(index).cloned().unwrap_or_default();

            if right.is_empty() {
                println!("  {left}");
            } else {
                println!("  {left}  {right}");
            }
        }
    } else {
        for line in text_lines {
            println!("  {line}");
        }
    }

    println!();
}

fn build_meta_lines(title: &str, meta: &Meta, source: &str, width: usize) -> Vec<String> {
    let mut lines = vec![];

    for line in wrap_text(title, width).into_iter().take(3) {
        lines.push(line.bright_cyan().bold().to_string());
    }

    let mut chips = vec![];
    if let Some(episodes) = meta.value("episode") {
        chips.push(format!("{} eps", episodes.trim()).bright_cyan().to_string());
    }
    if let Some(status) = meta.value("status") {
        let status = status.trim();
        if status.to_lowercase().contains("finished")
            || status.to_lowercase().contains("completed")
            || status.to_lowercase().contains("selesai")
        {
            chips.push(status.blue().to_string());
        } else {
            chips.push(status.green().to_string());
        }
    }
    if let Some(score) = meta.value("skor") {
        let score = score.trim();
        let score = score.trim_start_matches('★').trim();
        chips.push(format!("★ {score}").yellow().to_string());
    }

    let mut subtitle = format!("{}", source.dimmed());
    if !chips.is_empty() {
        subtitle.push_str(&format!("  {}", "·".dimmed()));
        subtitle.push_str(&format!(
            "  {}",
            chips.join(&format!("  {}  ", "·".dimmed()))
        ));
    }
    lines.push(subtitle);

    if let Some(genre) = meta.value("genre") {
        lines.push(format!(
            "{} {}",
            "Genre:".bright_white(),
            truncate_width(genre, width.saturating_sub(8))
        ));
    }

    let mut facts: Vec<&(String, String)> = meta
        .data
        .iter()
        .filter(|(key, value)| {
            let key = key.to_lowercase();
            !key.contains("genre")
                && !key.contains("episode")
                && !key.contains("status")
                && !key.contains("skor")
                && !key.contains("rating")
                && !value.trim().is_empty()
        })
        .collect();

    facts.sort_by_key(|(key, _)| fact_priority(key));

    let mut seen = std::collections::HashSet::new();
    for (key, value) in facts {
        if !seen.insert(key.to_lowercase()) {
            continue;
        }

        // "Asal" sering menduplikasi "Negara" dengan nilai yang sama.
        if key.to_lowercase().contains("asal") {
            if let Some(negara) = meta.value("negara") {
                if negara.trim().eq_ignore_ascii_case(value.trim()) {
                    continue;
                }
            }
        }

        lines.push(format!(
            "{} {}",
            format!("{key}:").bright_white(),
            truncate_width(
                value,
                width.saturating_sub(UnicodeWidthStr::width(key.as_str()) + 2)
            )
        ));
    }

    if let Some(synopsis) = &meta.synopsis {
        lines.push(String::new());
        let mut wrapped = wrap_text(synopsis, width);
        if wrapped.len() > 12 {
            wrapped.truncate(12);
            if let Some(last) = wrapped.last_mut() {
                last.push('…');
            }
        }
        for line in wrapped {
            lines.push(line.dimmed().to_string());
        }
    }

    lines
}

fn fact_priority(key: &str) -> u8 {
    let key = key.to_lowercase();
    if key.contains("studio") {
        1
    } else if key.contains("tipe") {
        2
    } else if key.contains("durasi") {
        3
    } else if key.contains("negara") || key.contains("asal") {
        4
    } else if key.contains("rilis") || key.contains("diperbarui") {
        5
    } else if key.contains("produser") {
        6
    } else if key.contains("japanese") || key.contains("judul") {
        7
    } else {
        8
    }
}

struct Poster {
    lines: Vec<String>,
    width: usize,
}

async fn load_poster(url: Option<&str>, cols: u32) -> Option<Poster> {
    let url = url?;
    let bytes = load_poster_bytes(url).await?;
    let image = image::load_from_memory(&bytes).ok()?;
    let (_, height) = image.dimensions();
    if height == 0 {
        return None;
    }

    let lines = render_halfblocks(&image, cols);
    Some(Poster {
        lines,
        width: cols as usize,
    })
}

async fn load_poster_bytes(url: &str) -> Option<Vec<u8>> {
    if let Some(path) = poster_cache_path(url) {
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes.len() > 512 {
                return Some(bytes);
            }
        }
    }

    let client = Client::builder()
        .user_agent(crate::source::embed::USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;

    let response = client
        .get(url)
        .header(reqwest::header::REFERER, "https://animeku.lol/")
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let bytes = response.bytes().await.ok()?.to_vec();
    if bytes.len() < 512 {
        return None;
    }

    if let Some(path) = poster_cache_path(url) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, &bytes);
    }

    Some(bytes)
}

fn poster_cache_path(url: &str) -> Option<PathBuf> {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;

    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);

    Some(
        base.join("wibu-cli")
            .join("posters")
            .join(format!("{:016x}.img", hasher.finish())),
    )
}

fn render_halfblocks(image: &DynamicImage, cols: u32) -> Vec<String> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return vec![];
    }

    let aspect = width as f32 / height as f32;
    let rows = ((cols as f32) / (aspect * 2.0)).round().clamp(5.0, 22.0) as u32;
    let resized = image
        .resize_exact(cols, rows * 2, FilterType::Triangle)
        .to_rgb8();

    let truecolor = truecolor_available();
    let mut lines = Vec::with_capacity(rows as usize);

    for y in 0..rows {
        let mut line = String::new();
        for x in 0..cols {
            let top = resized.get_pixel(x, y * 2);
            let bottom = resized.get_pixel(x, y * 2 + 1);

            if truecolor {
                line.push_str(&format!(
                    "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                    top[0], top[1], top[2], bottom[0], bottom[1], bottom[2]
                ));
            } else {
                line.push_str(&format!(
                    "\x1b[38;5;{}m\x1b[48;5;{}m▀",
                    ansi256(top[0], top[1], top[2]),
                    ansi256(bottom[0], bottom[1], bottom[2])
                ));
            }
        }
        line.push_str("\x1b[0m");
        lines.push(line);
    }

    lines
}

fn ansi256(r: u8, g: u8, b: u8) -> u8 {
    if r == g && g == b {
        if r < 8 {
            return 16;
        }
        if r > 248 {
            return 231;
        }
        return 232 + ((r as u16 - 8) * 24 / 247) as u8;
    }

    let to_cube = |value: u8| -> u8 {
        if value < 48 {
            0
        } else if value < 115 {
            1
        } else {
            ((value as u16 - 35) / 40) as u8
        }
    };

    16 + 36 * to_cube(r) + 6 * to_cube(g) + to_cube(b)
}

fn truecolor_available() -> bool {
    env::var("COLORTERM")
        .map(|value| {
            let value = value.to_lowercase();
            value.contains("truecolor") || value.contains("24bit")
        })
        .unwrap_or(false)
        || env::var("TERM")
            .map(|value| value.to_lowercase().contains("kitty"))
            .unwrap_or(false)
}

fn image_disabled() -> bool {
    env::var("WIBU_NO_IMAGE")
        .map(|value| {
            let value = value.to_lowercase();
            value == "1" || value == "true" || value == "yes"
        })
        .unwrap_or(false)
}

pub fn mpv_available() -> bool {
    Command::new("mpv")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Putar tautan dengan MPV. Opsi HLS ditambahkan karena sebagian host memakai
/// segmen tanpa ekstensi (ffmpeg 8 menolaknya kalau tidak diizinkan).
pub fn play_with_mpv(url: &str, title: &str, referer: &str) -> anyhow::Result<()> {
    use anyhow::Context;

    let mut command = Command::new("mpv");
    command
        .arg("--force-window=yes")
        .arg("--keep-open=no")
        .arg("--save-position-on-quit")
        .arg("--no-terminal")
        .arg("--msg-level=all=warn")
        .arg("--demuxer-lavf-o=extension_picky=0,allowed_segment_extensions=ALL")
        .arg(format!("--user-agent={}", crate::source::embed::USER_AGENT));

    if !crate::source::embed::host_blocks_referer(url) {
        command.arg(format!("--referrer={referer}"));
    }

    let status = command
        .arg(format!("--force-media-title={title}"))
        .arg(url)
        .status()
        .context("MPV tidak ditemukan atau gagal dijalankan")?;

    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("MPV keluar dengan status {status}")
    }
}

pub fn open_in_browser(url: &str) -> anyhow::Result<()> {
    use anyhow::Context;

    open::that(url).context("Gagal membuka browser")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_text_skips_scripts_and_styles() {
        let html = r#"
            <!-- komentar -->
            <p>Baris pertama.</p>
            <script>(function(s){s.dataset.zone='1'})()</script>
            <style>.ad { display: none }</style>
            <p>Baris kedua.</p>
        "#;

        assert_eq!(html_to_text(html), "Baris pertama.\nBaris kedua.");
        assert_eq!(clean_text(html), "Baris pertama. Baris kedua.");
    }
}
