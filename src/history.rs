//! Riwayat tontonan sederhana.
//!
//! Data disimpan sebagai JSON di direktori data pengguna (mengikuti XDG di
//! Linux) supaya menu "Lanjutkan tontonan" tetap ada antar sesi. Kalau direktori
//! tidak bisa ditulis, wibu-cli tetap jalan tanpa riwayat.

use std::{
    env, fmt, fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::models::{Episode, Movie};

const MAX_ENTRIES: usize = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub movie_id: String,
    pub title: String,
    pub info: Option<String>,
    pub poster_url: Option<String>,
    pub episode_id: String,
    pub episode_title: String,
    pub episode_number: Option<u32>,
    pub is_series: bool,
    pub source: String,
    pub updated_at: u64,
}

impl HistoryEntry {
    pub fn from_play(source: &str, movie: &Movie, episode: &Episode) -> Self {
        Self {
            movie_id: movie.id.clone(),
            title: movie.title.clone(),
            info: movie.info.clone(),
            poster_url: movie.poster_url.clone(),
            episode_id: episode.id.clone(),
            episode_title: episode.title.clone(),
            episode_number: episode.number,
            is_series: episode.is_series,
            source: source.to_string(),
            updated_at: now_millis(),
        }
    }

    fn ago(&self) -> String {
        let seconds = (now_millis() / 1000).saturating_sub(self.updated_at / 1000);
        match seconds {
            0..=59 => "baru saja".to_string(),
            60..=3599 => format!("{} menit lalu", seconds / 60),
            3600..=86_399 => format!("{} jam lalu", seconds / 3600),
            _ => format!("{} hari lalu", seconds / 86_400),
        }
    }
}

impl fmt::Display for HistoryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.title.trim())?;
        if self.episode_title.trim() != self.title.trim() {
            write!(f, " — {}", self.episode_title.trim())?;
        }
        write!(f, "  [{} · {}]", source_label(&self.source), self.ago())
    }
}

pub fn source_label(source: &str) -> &'static str {
    match source {
        "animeku" => "AnimeKu",
        "otakudesu" => "Otakudesu",
        "samehadaku" => "Samehadaku",
        "jepang" => "LK21",
        "tokusatsu" => "Tokusatsuindo",
        _ => "Sumber lain",
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn history_file() -> Option<PathBuf> {
    if let Ok(custom) = env::var("WIBU_DATA_DIR") {
        let custom = custom.trim();
        if !custom.is_empty() {
            return Some(PathBuf::from(custom).join("history.json"));
        }
    }

    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        let xdg = xdg.trim();
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("wibu-cli").join("history.json"));
        }
    }

    env::var("HOME")
        .ok()
        .filter(|home| !home.trim().is_empty())
        .map(|home| {
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("wibu-cli")
                .join("history.json")
        })
}

pub fn load() -> Vec<HistoryEntry> {
    let Some(path) = history_file() else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut entries: Vec<HistoryEntry> = serde_json::from_str(&text).unwrap_or_default();
    entries.retain(|entry| !entry.episode_id.trim().is_empty());
    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    entries.truncate(MAX_ENTRIES);
    entries
}

pub fn save(entries: &[HistoryEntry]) -> anyhow::Result<()> {
    let Some(path) = history_file() else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(entries)?)?;
    Ok(())
}

/// Simpan episode yang berhasil diputar. Episode yang sama dipindah ke atas,
/// bukan diduplikasi, supaya riwayat tetap ringkas.
pub fn record(source: &str, movie: &Movie, episode: &Episode) {
    let entry = HistoryEntry::from_play(source, movie, episode);
    let mut entries = load();
    entries.retain(|item| item.episode_id != entry.episode_id);
    entries.insert(0, entry);
    entries.truncate(MAX_ENTRIES);
    let _ = save(&entries);
}

pub fn clear() -> anyhow::Result<()> {
    let Some(path) = history_file() else {
        return Ok(());
    };
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(episode_id: &str, updated_at: u64) -> HistoryEntry {
        HistoryEntry {
            movie_id: "movie".into(),
            title: "Judul".into(),
            info: None,
            poster_url: None,
            episode_id: episode_id.into(),
            episode_title: "Episode 1".into(),
            episode_number: Some(1),
            is_series: true,
            source: "animeku".into(),
            updated_at,
        }
    }

    #[test]
    fn label_sumber_dikenal() {
        assert_eq!(source_label("tokusatsu"), "Tokusatsuindo");
        assert_eq!(source_label("tidak-dikenal"), "Sumber lain");
    }

    #[test]
    fn entry_display_tidak_mengulang_judul_film() {
        let mut item = entry("ep", now_millis());
        item.episode_title = item.title.clone();
        assert_eq!(item.to_string(), "Judul  [AnimeKu · baru saja]");
    }

    #[test]
    fn serialisasi_bolak_balik() {
        let items = vec![entry("ep-1", 123), entry("ep-2", 456)];
        let text = serde_json::to_string(&items).unwrap();
        let parsed: Vec<HistoryEntry> = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].episode_id, "ep-1");
        assert_eq!(parsed[1].updated_at, 456);
    }
}
