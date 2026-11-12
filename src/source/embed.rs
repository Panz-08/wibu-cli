use std::{process::Command, time::Duration};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use html_escape::decode_html_entities;
use reqwest::{header, Client, Url};
use serde_json::Value;

use crate::{regex, util::unescape_js_url};

pub const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

/// Cek apakah URL media benar-benar mengembalikan video. Google Drive kadang
/// membalas 200 dengan `text/html` kosong saat file kena kuota, dan URL
/// `googlevideo` bisa kedaluwarsa atau menolak User-Agent non-browser.
/// URL host lain dianggap valid supaya tidak menambah latensi.
pub async fn media_url_playable(client: &Client, url: &str) -> bool {
    if !(url.contains("drive.usercontent.google.com")
        || url.contains("googlevideo.com")
        || url.contains("player.tokusatsuindo.com/stream.php"))
    {
        return true;
    }

    let response = client
        .get(url)
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::RANGE, "bytes=0-2048")
        .timeout(Duration::from_secs(20))
        .send()
        .await;

    let Ok(response) = response else {
        return false;
    };

    if !response.status().is_success() {
        return false;
    }

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    if content_type.contains("text/html") || content_type.contains("application/json") {
        return false;
    }

    let content_length = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    if content_length == Some(0) {
        return false;
    }

    match response.bytes().await {
        Ok(bytes) => !bytes.is_empty(),
        Err(_) => false,
    }
}

/// Resolve URL embed (iframe player) menjadi tautan video langsung yang bisa
/// dimainkan MPV. Mendukung player putarin/puterin yang memakai config
/// terenkripsi AES-256-GCM, iframe berlapis, dan jatuh ke yt-dlp kalau perlu.
pub async fn resolve(client: &Client, start: &str, referer: &str) -> Option<String> {
    resolve_inner(client, start, referer, 0).await
}

async fn resolve_inner(
    client: &Client,
    start: &str,
    referer: &str,
    depth: usize,
) -> Option<String> {
    if depth > 4 {
        return None;
    }

    if let Some(id) = drive_file_id(start) {
        return Some(google_drive_direct(&id));
    }

    if is_direct_url(start) {
        return Some(start.to_string());
    }

    let body = fetch_text(client, start, referer).await?;

    if let Some(url) = resolve_encrypted_player(client, &body, start).await {
        return Some(url);
    }

    if let Some(unpacked) = unpack_packed_js(&body) {
        if let Some(url) = extract_direct_url(&unpacked) {
            return Some(url);
        }
    }

    if let Some(url) = extract_direct_url(&body) {
        return Some(url);
    }

    if let Some(next) = extract_iframe(&body) {
        if next != start {
            let next = absolutize(start, &next);
            if let Some(url) = Box::pin(resolve_inner(client, &next, start, depth + 1)).await {
                return Some(url);
            }
        }
    }

    // Kalau semua parser lokal gagal, coba serahkan ke yt-dlp. Ini berguna
    // untuk host yang memakai JavaScript berat.
    ytdlp_url(start)
}

async fn fetch_text(client: &Client, url: &str, referer: &str) -> Option<String> {
    // vkspeed menolak semua Referer asing ("embed restricted for this domain"),
    // tetapi tetap melayani halaman player kalau Referer tidak dikirim.
    let skip_referer = host_blocks_referer(url);

    for attempt in 0..3u64 {
        let mut request = client
            .get(url)
            .header(header::USER_AGENT, USER_AGENT)
            .header(header::ACCEPT_LANGUAGE, "id-ID,id;q=0.9,en;q=0.8,en;q=0.7");

        if !referer.is_empty() && !skip_referer {
            request = request.header(header::REFERER, referer);
        }

        match request.send().await {
            Ok(response) if response.status().is_success() => {
                if let Ok(body) = response.text().await {
                    return Some(body);
                }
            }
            Ok(response) => {
                if response.status().as_u16() == 404 || response.status().as_u16() == 410 {
                    return None;
                }
            }
            Err(_) => {}
        }

        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(350 * (attempt + 1))).await;
        }
    }

    None
}

/// Player seperti putarin.biz mengirim `window.__PX={n,d}`. Kunci diambil dari
/// `/api/pk?n=<n>`, lalu `d` (base64 IV + ciphertext) didekripsi AES-GCM.
async fn resolve_encrypted_player(client: &Client, html: &str, page_url: &str) -> Option<String> {
    let parsed: Value = regex!(r#"(?s)window\.__PX\s*=\s*(\{.*?\})\s*;"#)
        .captures(html)
        .and_then(|cap| serde_json::from_str(&cap[1]).ok())?;

    let nonce_id = parsed["n"].as_str()?;
    let payload = parsed["d"].as_str()?;

    let origin = Url::parse(page_url).ok()?.origin().ascii_serialization();
    let key_url = format!("{origin}/api/pk?n={nonce_id}");
    let key_hex = fetch_text(client, &key_url, page_url).await?;
    let key_hex = key_hex.trim();

    let plain = decrypt_px_payload(key_hex, payload)?;
    let config: Value = serde_json::from_str(&plain).ok()?;

    let file = config["file"]
        .as_str()
        .or_else(|| config["sources"].as_array()?.first()?.get("file")?.as_str())?;

    Some(absolutize(page_url, file))
}

fn decrypt_px_payload(key_hex: &str, payload_b64: &str) -> Option<String> {
    let key = hex_decode(key_hex)?;
    if key.len() != 32 {
        return None;
    }

    let raw = STANDARD.decode(payload_b64.trim()).ok()?;
    if raw.len() <= 12 {
        return None;
    }

    let (iv, ciphertext) = raw.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let nonce = Nonce::from_slice(iv);
    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;

    String::from_utf8(plaintext).ok()
}

fn hex_decode(input: &str) -> Option<Vec<u8>> {
    let input = input.trim();
    if input.len() % 2 != 0 {
        return None;
    }

    let mut bytes = Vec::with_capacity(input.len() / 2);
    let chars: Vec<char> = input.chars().collect();
    for pair in chars.chunks_exact(2) {
        let high = pair[0].to_digit(16)? as u8;
        let low = pair[1].to_digit(16)? as u8;
        bytes.push((high << 4) | low);
    }

    Some(bytes)
}

fn absolutize(base: &str, href: &str) -> String {
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

fn extract_iframe(html: &str) -> Option<String> {
    regex!(r#"(?s)<iframe[^>]+src=["']([^"']+)["']"#)
        .captures(html)
        .map(|cap| unescape_js_url(&cap[1]))
}

fn extract_direct_url(body: &str) -> Option<String> {
    let decoded = decode_html_entities(body).to_string();
    let normalized = decoded
        .replace("\\/", "/")
        .replace("\\u003d", "=")
        .replace("\\u003D", "=")
        .replace("\\u0026", "&")
        .replace("\\u002F", "/");

    if let Some(cap) =
        regex!(r#"(?s)(?:const\s+)?videoURL\s*=\s*["']([^"']+)["']"#).captures(&normalized)
    {
        return Some(unescape_js_url(&cap[1]));
    }

    // Player seperti morencius menyimpan beberapa kandidat di objek `links`.
    // Utamakan HLS `.m3u8`, lalu URL pertama kalau hanya ada master `.txt`.
    let mut fallback = None;
    for cap in
        regex!(r#"(?s)["']hls[0-9]["']\s*:\s*["'](https?://[^"']+)["']"#).captures_iter(&normalized)
    {
        let url = unescape_js_url(&cap[1]);
        if url.contains(".m3u8") {
            return Some(url);
        }
        fallback.get_or_insert(url);
    }
    if fallback.is_some() {
        return fallback;
    }

    if let Some(cap) = regex!(r#"(?s)file\s*:\s*["']([^"']+)["']"#).captures(&normalized) {
        let url = unescape_js_url(&cap[1]);
        if url.starts_with("http") || url.starts_with("//") || url.starts_with('/') {
            return Some(url);
        }
    }

    if let Some(cap) = regex!(r#"(?s)<source[^>]+src=["']([^"']+\.(?:mp4|m3u8)(?:\?[^"']*)?)["']"#)
        .captures(&normalized)
    {
        return Some(unescape_js_url(&cap[1]));
    }

    // Video Blogger / Google Video. URL-nya memakai escape \u003d dan \u0026.
    if let Some(cap) =
        regex!(r#"(?s)(https?://[^"'\s<>\\]*googlevideo\.com[^"'\s<>\\]+)"#).captures(&normalized)
    {
        return Some(unescape_js_url(&cap[1]));
    }

    if let Some(cap) = regex!(
        r#"(?s)(https?://[^"'\s<>\\]*cloudflarestorage[^"'\s<>\\]+\.(?:mp4|mkv|m3u8)[^"'\s<>\\]*)"#
    )
    .captures(&normalized)
    {
        return Some(unescape_js_url(&cap[1]));
    }

    if let Some(cap) = regex!(r#"(?s)(https?://[^"'\s<>\\]+?\.(?:mp4|m3u8)(?:\?[^"'\s<>\\]*)?)"#)
        .captures(&normalized)
    {
        return Some(unescape_js_url(&cap[1]));
    }

    None
}

/// Packer Dean Edwards menyamarkan URL di dalam `eval(function(p,a,c,k,e,d){...})`.
/// Fungsi ini mengembalikan isi yang sudah di-unpack supaya parser URL di atas
/// bisa membaca `links.hls2` / `links.hls3`.
fn unpack_packed_js(body: &str) -> Option<String> {
    let cap = regex!(
        r#"(?s)eval\(function\(p,a,c,k,e,d\)\{.*?\}\('((?:\\.|[^'])*)',\s*(\d+)\s*,\s*(\d+)\s*,\s*'((?:\\.|[^'])*)'\.split\('\|'\)\)\)"#
    )
    .captures(body)?;

    let packed = &cap[1];
    let radix: u32 = cap[2].parse().ok()?;
    let count: usize = cap[3].parse().ok()?;
    let words: Vec<&str> = cap[4].split('|').collect();

    let mut output = String::with_capacity(packed.len());
    let mut last = 0;

    for token in regex!(r"[0-9A-Za-z]+").find_iter(packed) {
        let Some(index) = decode_packed_index(token.as_str(), radix) else {
            continue;
        };
        if index >= count {
            continue;
        }
        let Some(replacement) = words.get(index).filter(|word| !word.is_empty()) else {
            continue;
        };

        output.push_str(&packed[last..token.start()]);
        output.push_str(replacement);
        last = token.end();
    }

    output.push_str(&packed[last..]);
    Some(output)
}

/// Decode angka base-a dengan pemetaan yang dipakai packer: 0-9, a-z (10-35),
/// A-Z (36-61). Token yang memakai karakter di luar radix dibiarkan apa adanya.
fn decode_packed_index(token: &str, radix: u32) -> Option<usize> {
    if radix < 2 {
        return None;
    }

    let mut value: usize = 0;
    for ch in token.chars() {
        let digit = match ch {
            '0'..='9' => ch as u32 - '0' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 10,
            'A'..='Z' => ch as u32 - 'A' as u32 + 36,
            _ => return None,
        };
        if digit >= radix {
            return None;
        }

        value = value
            .checked_mul(radix as usize)?
            .checked_add(digit as usize)?;
    }

    Some(value)
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

fn is_direct_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    (lower.contains(".mp4") || lower.contains(".m3u8")) && !lower.contains("/embed")
}

/// vkspeed dan CDN-nya menolak Referer asing. Host lain tetap memakai Referer
/// halaman episode karena banyak player HLS membutuhkannya.
pub fn host_blocks_referer(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_lowercase))
        .is_some_and(|host| {
            host == "vkspeed.com"
                || host.ends_with(".vkspeed.com")
                || host == "vkcdn5.com"
                || host.ends_with(".vkcdn5.com")
        })
}

fn ytdlp_url(url: &str) -> Option<String> {
    let output = Command::new("yt-dlp")
        .args([
            "-g",
            "-f",
            "best[ext=mp4]/best",
            "--no-warnings",
            "--no-playlist",
            "--no-check-certificate",
            url,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("http"))
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_url_from_odcdn_embed() {
        let html = r#"<script>
            const videoURL =
                "https://cdn.odcloud.net/anime/Contoh_720p.mp4";
        </script>"#;

        assert_eq!(
            extract_direct_url(html).as_deref(),
            Some("https://cdn.odcloud.net/anime/Contoh_720p.mp4")
        );
    }

    #[test]
    fn parses_blogger_streaming_data() {
        let html = r#"<script>var x = {"streamingData":{"formats":[{"itag":18,"url":"https:\/\/rr1---sn.example.googlevideo.com\/videoplayback?expire\u003d1\u0026id\u003dabc","height":360,"mimeType":"video\/mp4"}]}};</script>"#;

        let url = extract_direct_url(html).expect("url");
        assert!(url
            .starts_with("https://rr1---sn.example.googlevideo.com/videoplayback?expire=1&id=abc"));
    }

    #[test]
    fn decrypts_putarin_payload() {
        let key = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        let payload = "AAECAwQFBgcICQoLPCCwcqmA4CGtY7jqwYBXBe+luEDNGj0fCVXWpzFJIsZ4YMveleEw8BjXXZBcnf06Fr9Axk0JfWAwLSMG";

        let plain = decrypt_px_payload(key, payload).expect("plaintext");
        let json: Value = serde_json::from_str(&plain).expect("json");
        assert_eq!(json["file"], "/api/hls?t=abc123");
    }

    #[test]
    fn absolutizes_relative_player_urls() {
        assert_eq!(
            absolutize("https://putarin.biz/e/abc", "/api/hls?t=xyz"),
            "https://putarin.biz/api/hls?t=xyz"
        );
        assert_eq!(
            absolutize("https://putarin.biz/e/abc", "//cdn.example/video.m3u8"),
            "https://cdn.example/video.m3u8"
        );
    }

    #[test]
    fn unpacks_dean_edwards_packed_url() {
        let html = r#"<script>
            eval(function(p,a,c,k,e,d){return p}('0/1/2?token=3',36,4,'https://cdn.example|video|master.m3u8|abc'.split('|')))
        </script>"#;

        let unpacked = unpack_packed_js(html).expect("unpacked");
        assert_eq!(unpacked, "https://cdn.example/video/master.m3u8?token=abc");
        assert_eq!(
            extract_direct_url(&unpacked).as_deref(),
            Some("https://cdn.example/video/master.m3u8?token=abc")
        );
    }

    #[test]
    fn decodes_packed_index_for_base_36_and_62() {
        assert_eq!(decode_packed_index("10", 36), Some(36));
        assert_eq!(decode_packed_index("A", 62), Some(36));
        assert_eq!(decode_packed_index("a", 62), Some(10));
        assert_eq!(decode_packed_index("Z", 36), None);
    }

    #[test]
    fn unpacks_vkspeed_jwplayer_sources() {
        let html = r#"<script>
            eval(function(p,a,c,k,e,d){while(c--)if(k[c])p=p.replace(new RegExp('\\b'+c.toString(a)+'\\b','g'),k[c]);return p}('5.4=[{3:"0://1/2/v.6"}]',7,7,'https|phbg00.vkcdn5.com|abc|file|sources|jwplayer|mp4'.split('|')))
        </script>"#;

        let unpacked = unpack_packed_js(html).expect("unpacked");
        assert_eq!(
            unpacked,
            "jwplayer.sources=[{file:\"https://phbg00.vkcdn5.com/abc/v.mp4\"}]"
        );
        assert_eq!(
            extract_direct_url(&unpacked).as_deref(),
            Some("https://phbg00.vkcdn5.com/abc/v.mp4")
        );
    }

    #[test]
    fn skips_referer_only_for_vkspeed() {
        assert!(host_blocks_referer("https://vkspeed.com/embed-abc.html"));
        assert!(host_blocks_referer("https://cdn.vkspeed.com/e/abc"));
        assert!(host_blocks_referer("https://phbg00.vkcdn5.com/abc/v.mp4"));
        assert!(!host_blocks_referer("https://morencius.com/embed/abc"));
    }

    #[test]
    fn converts_google_drive_preview_to_direct_download() {
        let id = "1ECi5F9-PLvCUG-Kad4r4uIQJs8NA5bVl";
        assert_eq!(
            drive_file_id(&format!("https://drive.google.com/file/d/{id}/preview")).as_deref(),
            Some(id)
        );
        assert_eq!(
            google_drive_direct(id),
            format!(
                "https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t"
            )
        );
    }
}
