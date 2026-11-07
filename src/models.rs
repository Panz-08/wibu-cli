#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    All,
    Ongoing,
    Completed,
    Movie,
}

impl SearchMode {
    pub fn label(self) -> &'static str {
        match self {
            SearchMode::All => "semua anime",
            SearchMode::Ongoing => "anime ongoing",
            SearchMode::Completed => "anime complete",
            SearchMode::Movie => "film anime",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            SearchMode::All => "semua",
            SearchMode::Ongoing => "ongoing",
            SearchMode::Completed => "selesai",
            SearchMode::Movie => "film",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Movie {
    /// URL lengkap halaman anime.
    pub id: String,
    pub title: String,
    /// Info tambahan, misalnya status dan rating.
    pub info: Option<String>,
    /// URL poster kalau tersedia dari halaman daftar.
    pub poster_url: Option<String>,
}

impl std::fmt::Display for Movie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.title.trim())?;
        if let Some(info) = &self.info {
            if !info.trim().is_empty() {
                write!(f, "  [{}]", info)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Episode {
    /// URL lengkap halaman episode.
    pub id: String,
    pub title: String,
    /// Nomor episode kalau bisa dibaca dari judul.
    pub number: Option<u32>,
    pub is_series: bool,
}

impl std::fmt::Display for Episode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.title.trim())
    }
}

/// Hasil satu halaman daftar anime (hasil pencarian atau kategori).
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub items: Vec<Movie>,
    pub next: Option<String>,
}

/// Data yang dibutuhkan untuk meminta embed player lewat admin-ajax Otakudesu.
#[derive(Debug, Clone, Default)]
pub struct Mirror {
    pub id: u64,
    pub index: u64,
    pub quality: String,
    pub referer: String,
    pub nonce_action: String,
    pub mirror_action: String,
}

#[derive(Debug, Clone, Default)]
pub struct Stream {
    pub title: String,
    /// Kalau sudah berupa URL embed / direct, isi di sini.
    pub url: Option<String>,
    /// Halaman yang meng-host player, dipakai sebagai Referer saat resolve.
    pub referer: Option<String>,
    /// Kalau masih perlu request admin-ajax dulu.
    pub mirror: Option<Mirror>,
}

impl std::fmt::Display for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.title.trim())
    }
}

#[derive(Debug, Clone, Default)]
pub struct DownloadLink {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Default)]
pub struct Download {
    pub quality: String,
    pub size: Option<String>,
    pub links: Vec<DownloadLink>,
}

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub thumb_url: Option<String>,
    pub data: Vec<(String, String)>,
    pub synopsis: Option<String>,
    pub downloads: Vec<Download>,
}

impl Meta {
    pub fn value(&self, key: &str) -> Option<&str> {
        let wanted = key.to_lowercase();
        self.data
            .iter()
            .find(|(k, _)| k.to_lowercase().contains(&wanted))
            .map(|(_, v)| v.as_str())
    }
}
