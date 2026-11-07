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

    if is_direct_url(start) {
        return Some(start.to_string());
    }

    let body = fetch_text(client, start, referer).await?;

    if let Some(url) = resolve_encrypted_player(client, &body, start).await {
        return Some(url);
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
    for attempt in 0..3u64 {
        let mut request = client
            .get(url)
            .header(header::USER_AGENT, USER_AGENT)
            .header(header::ACCEPT_LANGUAGE, "id-ID,id;q=0.9,en;q=0.8,en;q=0.7");

        if !referer.is_empty() {
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

fn is_direct_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    (lower.contains(".mp4") || lower.contains(".m3u8")) && !lower.contains("/embed")
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
}
