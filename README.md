# wibu-cli

[![CI](https://github.com/Panz-08/wibu-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/Panz-08/wibu-cli/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Nonton anime sub Indo dari terminal. Anime diambil dari AnimeKu, lalu diputar
langsung di MPV tanpa perlu buka browser.

Proyek ini lanjutan dari [animeku-cli](https://github.com/zevtyardt/animeku-cli)
yang sumbernya sudah tidak jalan. Saya pakai sendiri hampir tiap hari, jadi
sekalian saya rapikan dan taruh di sini. Kalau ada yang ikut pakai, silakan.

## Fitur

- Cari anime atau jelajahi kategori ongoing, selesai, dan film.
- Pilih episode dari yang terbaru, lengkap dengan navigasi halaman.
- Resolve player langsung ke HLS, lalu buka MPV.
- Fallback otomatis ke Otakudesu kalau AnimeKu sedang bermasalah.
- Poster dan info anime tampil di terminal. Posternya disembunyikan kalau
  terminal terlalu sempit.
- Link download ditampilkan kalau sumbernya menyediakan.

## Instalasi

Butuh Rust dan `mpv` di `PATH`.

### Langsung dari GitHub

```bash
cargo install --git https://github.com/Panz-08/wibu-cli --locked
```

Pastikan `~/.cargo/bin` ada di `PATH`. Setelah itu jalankan:

```bash
wibu-cli
```

### Binary siap pakai

Kalau tidak mau install Rust, ambil binary Linux x86_64 dari halaman
[Releases](https://github.com/Panz-08/wibu-cli/releases). Ekstrak, taruh
`wibu-cli` di `PATH`, lalu jalankan.

Untuk macOS dan Windows belum ada binary resmi. Build dari source di bawah.

### Dari source

```bash
git clone https://github.com/Panz-08/wibu-cli.git
cd wibu-cli
cargo install --path . --locked
```

Untuk coba tanpa install:

```bash
cargo run --release
```

### Install mpv

- Arch / Manjaro: `sudo pacman -S mpv`
- Debian / Ubuntu: `sudo apt install mpv`
- Fedora: `sudo dnf install mpv`
- macOS: `brew install mpv`
- Windows: unduh dari <https://mpv.io/installation/>, lalu tambahkan folder
  `mpv.exe` ke `PATH`.

## Pemakaian

```bash
wibu-cli                     # mode interaktif
wibu-cli --help
wibu-cli --version
```

Beberapa opsi yang berguna:

```bash
wibu-cli --source animeku    # paksa AnimeKu (default)
wibu-cli --source otakudesu  # paksa Otakudesu
wibu-cli --probe frieren 0   # tes cari + episode + resolve
wibu-cli --resolve 'https://putarin.biz/e/BGusdRwJje'
```

`--probe` dan `--resolve` dipakai buat ngecek apakah sumber dan resolver masih
jalan. Keduanya tidak butuh terminal interaktif.

## Variabel lingkungan

| Variabel | Fungsi |
| --- | --- |
| `WIBU_ANIMEKU_BASE` | Ganti domain AnimeKu. Default `https://animeku.lol`. |
| `WIBU_NO_IMAGE=1` | Matikan poster di kartu metadata. |
| `WIBU_BROWSER=1` | Selalu buka browser, jangan pakai MPV. |
| `XDG_CACHE_HOME` | Lokasi cache poster. Default `~/.cache/wibu-cli/posters`. |

## Cara kerja

1. Daftar anime diambil dari HTML AnimeKu. Slug kategori dibaca dari navigasi
   homepage, jadi tidak di-hardcode dan tetap jalan kalau slug berubah.
2. Halaman episode menyediakan iframe player, biasanya dari `putarin.biz` atau
   `puterin.biz`.
3. Player di-resolve dengan membaca `window.__PX`, meminta key ke `/api/pk`,
   lalu mendekripsi payload AES-256-GCM untuk mendapat URL `/api/hls?t=...`.
4. URL HLS diputar dengan MPV. Sebagian host memakai segmen tanpa ekstensi,
   jadi MPV diberi opsi:

   ```text
   --demuxer-lavf-o=extension_picky=0,allowed_segment_extensions=ALL
   ```

Kalau AnimeKu gagal, pencarian dicoba lewat Otakudesu. Sumber yang sedang
dipakai terlihat di kartu metadata.

## Troubleshooting

**MPV tidak ditemukan**

Install `mpv`, lalu cek dengan `mpv --version`. Kalau tetap tidak ketemu,
pastikan lokasi binary-nya ada di `PATH`.

**AnimeKu error atau lambat**

Coba lagi beberapa saat. Kalau domainnya berubah, set
`WIBU_ANIMEKU_BASE=https://ww2.animeku.lol` (atau mirror lain yang aktif).
Fallback Otakudesu juga bisa dipakai lewat `--source otakudesu`.

**Poster tidak muncul atau terlihat aneh**

Poster hanya muncul di terminal minimal 62 kolom. Kalau tetap berantakan,
matikan dengan `WIBU_NO_IMAGE=1`. Font monospace yang mendukung blok Unicode
(`▀`) lebih aman.

**Video gagal diputar**

Jalankan `wibu-cli --probe <judul>` untuk melihat sampai mana prosesnya
berhasil. Kalau resolve dapat URL tapi MPV gagal, coba buka URL tersebut
dengan `mpv` langsung untuk melihat pesan errornya.

## Status

Yang sudah dites: Linux x86_64. macOS dan Windows kemungkinan bisa di-build,
tapi belum saya coba. Kalau nemu masalah di platform lain, buka issue saja.

## Kredit

- [animeku-cli](https://github.com/zevtyardt/animeku-cli) oleh zevtyardt,
  sebagai dasar awal proyek ini.
- AnimeKu dan Otakudesu sebagai sumber data dan streaming.

## Lisensi

MIT. Lihat [LICENSE](LICENSE).

Proyek ini cuma alat bantu untuk mengakses konten publik dari sumbernya. Semua
hak cipta konten tetap milik sumber, studio, dan penerbitnya. Pakai dengan
bijak.
