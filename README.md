# wibu-cli

[![CI](https://github.com/Panz-08/wibu-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/Panz-08/wibu-cli/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Panz-08/wibu-cli?display_name=tag)](https://github.com/Panz-08/wibu-cli/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Nonton anime, film Jepang, series/drama Jepang, dan tokusatsu dari terminal.
Video diputar langsung di MPV tanpa perlu buka browser.

Sumber yang dipakai:

- **AnimeKu** untuk anime, dengan **Otakudesu** dan **Samehadaku** sebagai
  cadangan otomatis.
- **LK21** untuk film Jepang dan series/drama Jepang.
- **Tokusatsuindo** untuk Ultraman, Kamen Rider, Super Sentai, dan movie/special.

Proyek ini lanjutan dari [animeku-cli](https://github.com/zevtyardt/animeku-cli)
yang sumbernya sudah tidak jalan. Saya pakai sendiri hampir tiap hari, jadi
sekalian saya rapikan dan taruh di sini. Kalau ada yang ikut pakai, silakan.

## Fitur

- Cari anime atau jelajahi kategori ongoing, selesai, dan film.
- Cari film Jepang dan series/drama Jepang, atau jelajahi katalognya.
- Cari Ultraman, Kamen Rider, Super Sentai, dan movie/special.
- Pilih episode lewat daftar. Anime diurutkan dari episode terbaru; series
  Jepang dan tokusatsu diurutkan dari episode 1.
- **Lanjutkan tontonan**: episode terakhir yang berhasil diputar tersimpan
  otomatis, lengkap dengan sumber dan waktunya. Bisa dilanjutkan tanpa mencari
  ulang. Riwayat bisa dihapus dari menu yang sama.
- Resolve player langsung ke HLS, MP4, atau Google Drive, lalu buka MPV.
- Fallback otomatis ke Otakudesu lalu Samehadaku kalau AnimeKu bermasalah.
- Server episode dicoba berurutan: kalau resolve atau MPV gagal, server
  berikutnya dipakai tanpa perlu mengulang dari awal.
- Tautan Google Drive dan `googlevideo` dicek dulu supaya file yang kena kuota
  tidak dikirim ke MPV.
- Poster dan info judul tampil di terminal. Posternya disembunyikan kalau
  terminal terlalu sempit.
- Navigasi halaman untuk katalog anime dan film/series Jepang.
- Link download ditampilkan kalau sumbernya menyediakan.

## Tampilan

Gambar-gambar ini diambil dari aplikasi yang benar-benar dijalankan lewat PTY,
bukan mockup.

Menu utama:

<img src="assets/menu.png" alt="Menu utama wibu-cli" width="760">

Series Jepang dari LK21, lengkap dengan poster dan daftar episode:

<img src="assets/detail.png" alt="Detail series Jepang dengan poster dan daftar episode" width="820">

Ultraman Tiga dari Tokusatsuindo:

<img src="assets/tokusatsu.png" alt="Detail Ultraman Tiga dengan 52 episode" width="820">

Contoh `--probe` untuk film LK21 (HLS) dan Ultraman Tiga (Google Drive):

<img src="assets/probe.png" alt="Hasil probe film Jepang dan tokusatsu" width="820">

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

Kalau tidak mau install Rust, ambil binary dari halaman
[Releases](https://github.com/Panz-08/wibu-cli/releases):

- Linux x86_64: `wibu-cli-linux-x86_64.tar.gz`
- macOS Apple Silicon: `wibu-cli-macos-aarch64.tar.gz`
- Windows x86_64: `wibu-cli-windows-x86_64.zip`

Ekstrak, taruh `wibu-cli` (atau `wibu-cli.exe` di Windows) di `PATH`, lalu
jalankan. Binary Linux sudah dites; macOS dan Windows baru dibangun otomatis
oleh CI, belum dites jalan.

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

Di menu interaktif ada tiga entri baru:

- **Film Jepang** untuk film Jepang umum.
- **Series & drama Jepang** untuk series/drama Jepang.
- **Tokusatsu (Ultraman, Kamen Rider, Sentai)** untuk judul tokusatsu.

Beberapa opsi yang berguna:

```bash
wibu-cli --source animeku    # paksa AnimeKu (default)
wibu-cli --source otakudesu  # paksa Otakudesu
wibu-cli --source samehadaku # paksa Samehadaku (cadangan ketiga)
wibu-cli --source jepang     # paksa LK21 untuk film/series Jepang
wibu-cli --source tokusatsu  # paksa Tokusatsuindo
wibu-cli --probe frieren 0   # tes cari + episode + resolve
wibu-cli --resolve 'https://morencius.com/embed/68adzcvrsu2e'
```

Contoh uji sumber Jepang:

```bash
wibu-cli --source jepang --probe "godzilla minus one" 2
wibu-cli --source samehadaku --probe "sousou no frieren" 0
wibu-cli --source tokusatsu --probe "ultraman tiga" 0
```

`--probe` dan `--resolve` dipakai buat ngecek apakah sumber dan resolver masih
jalan. Keduanya tidak butuh terminal interaktif.

### Contoh output `--probe`

Output asli dipotong, tapi urutannya seperti ini:

```text
$ wibu-cli --probe frieren 0
[1/5] sumber: AnimeKu
[2/5] mencari 'frieren' ...
  - Sousou no Frieren
[3/5] memuat episode 'Sousou no Frieren' ...
  28 episode, 8 info, 2 link download
  - Episode 28
  - Episode 27
[4/5] memuat server 'Episode 28' ...
  - Server 1
  - Server 2
[5/5] resolve 'Server 1' ...
  OK: https://contoh-host/api/hls?t=...
```

Kalau resolve gagal, baris terakhir berubah menjadi `GAGAL resolve` dan proses
keluar dengan status 1.

## AnimeKu, Otakudesu, dan Samehadaku

| | AnimeKu (utama) | Otakudesu (cadangan 1) | Samehadaku (cadangan 2) |
| --- | --- | --- | --- |
| Kapan dipakai | default, atau `--source animeku` | otomatis saat AnimeKu gagal, atau `--source otakudesu` | otomatis saat keduanya gagal, atau `--source samehadaku` |
| Katalog | ongoing, selesai, film, dan halaman berikutnya | pencarian dan daftar episode | pencarian dan daftar episode |
| Pagination | didukung | tidak, hanya hasil pencarian | tidak, hanya hasil pencarian |
| Resolver | `putarin.biz` / `puterin.biz` lewat `window.__PX` | iframe player, resolve langsung | Blogger `video.g`, di-resolve ke MP4 `googlevideo` |
| Poster | tampil kalau terminal ≥ 62 kolom | tampil kalau sumber menyediakan | tampil kalau sumber menyediakan |
| Link download | kalau tersedia | kalau tersedia | belum diambil |

Fallback otomatis **hanya** terjadi kalau sumber tidak dipaksa. Kalau kamu
menjalankan `--source animeku`, kegagalan AnimeKu akan ditampilkan apa adanya
tanpa berpindah ke sumber lain.

Contoh membandingkan ketiga sumber:

```bash
wibu-cli --probe "sousou no frieren" --source animeku
wibu-cli --probe "sousou no frieren" --source otakudesu
wibu-cli --probe "sousou no frieren" --source samehadaku
wibu-cli --source otakudesu          # seluruh sesi memakai Otakudesu
```

## Variabel lingkungan

| Variabel | Fungsi |
| --- | --- |
| `WIBU_ANIMEKU_BASE` | Ganti domain AnimeKu. Default `https://animeku.lol`. |
| `WIBU_SAMEHADAKU_BASE` | Ganti domain Samehadaku. Default `https://samehadaku.today`. |
| `WIBU_LK21_BASE` | Ganti domain LK21. Default `https://layarkaca21.health`. |
| `WIBU_TOKUSATSU_BASE` | Ganti domain Tokusatsuindo. Default `https://www.tokusatsuindo.com`. |
| `WIBU_NO_IMAGE=1` | Matikan poster di kartu metadata. |
| `WIBU_BROWSER=1` | Selalu buka browser, jangan pakai MPV. |
| `WIBU_DATA_DIR=<dir>` | Ganti lokasi penyimpanan riwayat tontonan. |

Riwayat tontonan disimpan sebagai JSON di `~/.local/share/wibu-cli/history.json`
(atau mengikuti `XDG_DATA_HOME`). Isinya hanya URL episode, judul, sumber, dan
waktu tonton — tidak ada data akun atau kredensial. Hapus lewat menu
**Lanjutkan tontonan → Hapus riwayat tontonan**.
| `XDG_CACHE_HOME` | Lokasi cache poster. Default `~/.cache/wibu-cli/posters`. |

## Cara kerja

### AnimeKu dan Otakudesu

1. Daftar anime diambil dari HTML AnimeKu. Slug kategori dibaca dari navigasi
   homepage, jadi tidak di-hardcode dan tetap jalan kalau slug berubah.
2. Halaman episode menyediakan iframe player, biasanya dari `putarin.biz` atau
   `puterin.biz`.
3. Player di-resolve dengan membaca `window.__PX`, meminta key ke `/api/pk`,
   lalu mendekripsi payload AES-256-GCM untuk mendapat URL `/api/hls?t=...`.
4. URL HLS diputar dengan MPV.

Kalau AnimeKu gagal, pencarian dicoba lewat Otakudesu, lalu Samehadaku.
Sumber yang sedang dipakai terlihat di kartu metadata.

### Samehadaku

1. Pencarian memakai JSON API `/api/search?q=...`, bukan hasil HTML.
2. Halaman `/series/<slug>` memuat tautan `/watch/<slug>-episode-N`; episode
   diurutkan berdasarkan nomor.
3. Halaman episode menyimpan `streamingLinks.video` di payload Next.js RSC.
4. URL `blogger.com/video.g?token=...` di-resolve menjadi MP4 `googlevideo`.
   MPV wajib memakai User-Agent browser; `wibu-cli` sudah mengaturnya.

### Film dan series Jepang (LK21)

1. Katalog dibaca lewat REST WordPress:
   `/wp-json/wp/v2/posts` untuk film dengan filter `muvicountry=1096` (Japan),
   dan `/wp-json/wp/v2/tv` dengan kategori `drama-jepang` (`3535`) untuk
   series. Kategori itu penting karena filter negara saja masih mencampur
   anime.
2. Item yang punya `tag-anime` atau `category-animation` dibuang dari hasil,
   jadi menu series/drama Jepang hanya berisi live-action.
3. Pagination memakai header `X-WP-TotalPages`.
4. Halaman detail menyediakan fakta, sinopsis, poster, dan kartu episode.
5. Player dimuat lewat `admin-ajax.php` (`p1`, `p2`, `p3`), lalu iframe-nya
   di-resolve. Kalau satu server gagal, server berikutnya dicoba otomatis.
6. Player seperti `morencius.com` memakai packed JavaScript; URL HLS di-unpack
   lalu diputar. Player seperti `vkspeed.com` menghasilkan MP4 langsung.

### Tokusatsuindo

1. Katalog series dibangun dari empat halaman kategori: Ultraman, Kamen Rider,
   Super Sentai, dan Movie & Special. Pencarian dilakukan lokal dari katalog
   itu karena pencarian WordPress mengembalikan post episode, bukan series.
2. Halaman episode menyimpan iframe player dalam atribut `data-embed` berisi
   base64.
3. Chain player: `/x/` → ambil `data-src` → `/p/` → ID Google Drive dan
   `proxySources` (`stream.php`).
4. ID Google Drive diubah menjadi tautan direct
   `drive.usercontent.google.com/download?...` supaya MPV tidak butuh browser.
5. Sebelum diputar, tautan Google Drive dan proxy dicek dengan request Range.
   File yang dibalas `text/html` kosong atau JSON kuota dianggap gagal, lalu
   server berikutnya dicoba.

### MPV

Sebagian host memakai segmen HLS tanpa ekstensi, jadi MPV diberi opsi:

```text
--demuxer-lavf-o=extension_picky=0,allowed_segment_extensions=ALL
```

Referer halaman episode dikirim ke player yang membutuhkannya. Untuk
`vkspeed.com` dan CDN `vkcdn5.com`, Referer justru tidak dikirim karena host
itu menolak Referer asing.

## Troubleshooting

**`MPV tidak ditemukan di PATH.`**

Install `mpv`, lalu cek dengan `mpv --version`. Kalau tetap tidak ketemu,
pastikan lokasi binary-nya ada di `PATH`. Di Windows, tambahkan folder tempat
`mpv.exe` berada ke `PATH` lalu buka terminal baru.

**`wibu-cli harus dijalankan langsung di terminal interaktif.`**

Mode interaktif butuh stdin dan stdout berupa TTY. Jalankan `wibu-cli` langsung
dari terminal, bukan lewat pipe, cron, atau `nohup`. Untuk pengecekan otomatis,
pakai `--probe` atau `--resolve` yang tidak butuh TTY.

**`AnimeKu: ...` lalu `Otakudesu: ...` lalu `Samehadaku: ...`**

Ketiga sumber gagal dihubungi. Biasanya ini masalah jaringan atau domain
sumber berubah. Cek koneksi, lalu coba `WIBU_ANIMEKU_BASE=https://ww2.animeku.lol`.
Kalau hanya AnimeKu yang bermasalah, fallback Otakudesu dan Samehadaku akan
dipakai otomatis (tanpa `--source`).

**`Tidak ada server streaming untuk episode ini.`**

Episode ada, tetapi tidak ada iframe player yang bisa di-resolve. Coba episode
lain, atau jalankan `wibu-cli --probe <judul>` untuk melihat server apa yang
terdeteksi.

**`Server ini tidak mengembalikan tautan video.`**

Server mengembalikan halaman yang tidak berisi URL HLS. Coba server lain di
daftar, atau cek apakah resolver masih cocok dengan struktur halaman sumber.

**`MPV keluar dengan status N`**

Resolve berhasil tapi MPV menolak URL/stream. Coba buka URL hasil `--probe`
langsung dengan `mpv <url>` untuk melihat pesan lengkap. Kalau memakai
`WIBU_BROWSER=1`, video dibuka di browser, jadi masalah MPV tidak berlaku.

**LK21 atau Tokusatsuindo tidak bisa dibuka**

Domain sumber bisa berubah. Cek di browser, lalu set `WIBU_LK21_BASE` atau
`WIBU_TOKUSATSU_BASE` ke domain yang aktif. Kalau satu server episode gagal,
coba server lain atau episode lain.

**Episode LK21 tertentu gagal**

Sebagian episode memakai player `play.xtwap.top`, `bestx.stream`, atau
`emturbovid.com` yang dilindungi Cloudflare, sedang down, atau menolak alamat
IP tertentu. `wibu-cli` sudah mencoba semua tab server (`p1`/`p2`/`p3`) dan
semua server berurutan. Kalau semuanya gagal, player itu memang tidak bisa
di-resolve tanpa browser; biasanya film yang sama masih punya server
`vidhidevip.com` atau `morencius.com` yang bisa diputar.

**`Server ini tidak mengembalikan video yang bisa diputar.`**

Untuk Google Drive, ini biasanya berarti file kena kuota download
(`downloadQuotaExceeded`) atau pemiliknya membatasi akses. Coba server lain.
Kalau episode hanya punya satu file Drive yang terkena kuota, episode itu
memang tidak bisa diputar sampai file alternatif tersedia.

**Poster tidak muncul atau terlihat aneh**

Poster hanya muncul di terminal minimal 62 kolom. Kalau tetap berantakan,
matikan dengan `WIBU_NO_IMAGE=1`. Font monospace yang mendukung blok Unicode
(`▀`) lebih aman.

**Video gagal diputar**

Jalankan `wibu-cli --probe <judul>` untuk melihat sampai mana prosesnya
berhasil. Kalau resolve dapat URL tapi MPV gagal, coba buka URL tersebut
dengan `mpv` langsung untuk melihat pesan errornya.

## Status proyek

Proyek ini masih tahap awal dan masih saya kembangkan sambil dipakai. Tapi
bukan cuma rancangan: binary-nya sudah jalan, instalasi dari GitHub sudah
dicoba, dan alur cari judul sampai putar di MPV sudah dites langsung.

Versi 1.3.0 memperbaiki katalog series Jepang agar hanya berisi live-action,
menambahkan Samehadaku sebagai cadangan anime, dan membuat pemutaran mencoba
server lain saat satu server gagal. Pengujian live terakhir dilakukan pada
12 November 2026. Sumber film dan tokusatsu bisa mengubah struktur halaman,
mengganti domain, atau memasang proteksi tambahan kapan saja. Kalau itu
terjadi, parser dan resolver mungkin perlu disesuaikan.

## Pengujian

### Otomatis

```bash
cargo fmt --check
cargo check --locked
cargo test --locked
cargo build --release --locked
```

- 35 unit test lulus. Cakupannya: parser AnimeKu, Otakudesu, Samehadaku,
  LK21 (termasuk filter anime), Tokusatsuindo, resolver embed (AES, packed
  JavaScript, HLS, Google Drive, Blogger), dan pembersihan teks dari script
  iklan.
- CI GitHub Actions menjalankan format, check, test, dan build setiap push ke
  `main`.

### Manual

Pengujian live pada 12 November 2026:

- **Samehadaku**: pencarian `sousou no frieren` mengembalikan series, 10
  episode terbaca, `streamingLinks.video` berhasil di-resolve menjadi MP4
  `googlevideo`, dan MPV headless decode 30 frame dengan status 0.
- **Katalog series Jepang LK21**: `categories=3535` + filter `tag-anime` /
  `category-animation` mengembalikan live-action (mis. **Loveless Lovers**,
  **House of Ninjas**, **Perfect Propose**), bukan anime.
- **Film Jepang LK21**: **Godzilla Minus One (2023)** menemukan 3 server.
  Server 1 `vectorx.top` gagal resolve, server 2 `emturbovid.com` tidak
  merespons, lalu server 3 `vidhidevip.com` menghasilkan HLS dan MPV headless
  decode 30 frame dengan status 0.
- **Tokusatsuindo**: **Ultraman Tiga** episode 1-5 masing-masing punya 2 file
  Google Drive dan keduanya mengembalikan `video/mp4`; episode 1 dites dengan
  MPV headless decode 30 frame status 0. Server `API Source` (`stream.php`)
  juga terdeteksi.
- **Kamen Rider Kuuga** dan **Super Sentai Liveman** masih memakai satu file
  Google Drive yang terkena kuota (`text/html` kosong), jadi episode tersebut
  belum bisa diputar. Aplikasi sekarang menandai file itu tidak valid dan
  mencoba server berikutnya, bukan langsung menyerahkan URL mati ke MPV.
- Pemutaran dites dengan MPV headless (`--vo=null --ao=null --frames=30`):
  HLS LK21, MP4 Blogger Samehadaku, dan Google Drive Tokusatsuindo semuanya
  decode dan keluar dengan status 0.
- Fallback server dites lewat PTY dengan `mpv` palsu: Godzilla melewati
  server 1 dan 2 yang gagal, berhasil resolve di server 3, lalu MPV dipanggil.
  Kamen Rider Kuuga melewati dua server yang file Drive/proxy-nya tidak valid
  tanpa memanggil MPV sama sekali.
- Alur interaktif dites lewat PTY: menu, cari film Jepang, tampilkan detail,
  pilih server, dan kembali ke menu tanpa crash.
- Screenshot di README diambil dari sesi PTY tersebut, bukan mockup.
- `cargo install --git https://github.com/Panz-08/wibu-cli --locked` berhasil
  di direktori instalasi bersih pada rilis sebelumnya.

Yang belum dites: macOS dan Windows. Build-nya kemungkinan jalan, tapi belum
saya coba. Kalau nemu masalah di platform lain, buka issue.

| Platform | Status |
| --- | --- |
| Linux x86_64 | ✅ dites (build, install dari GitHub, putar di MPV) |
| macOS (Intel/Apple Silicon) | ⚠️ belum dites |
| Windows | ⚠️ belum dites |
| Linux ARM (Raspberry Pi, dll.) | ⚠️ belum dites |

## Kredit

- [animeku-cli](https://github.com/zevtyardt/animeku-cli) oleh zevtyardt,
  sebagai dasar awal proyek ini.
- AnimeKu, Otakudesu, dan Samehadaku sebagai sumber anime.
- LK21 sebagai sumber film dan series/drama Jepang.
- Tokusatsuindo sebagai sumber Ultraman, Kamen Rider, Super Sentai, dan
  movie/special.

## Lisensi

MIT. Lihat [LICENSE](LICENSE).

Proyek ini cuma alat bantu untuk mengakses konten publik dari sumbernya. Semua
hak cipta konten tetap milik sumber, studio, dan penerbitnya. Pakai dengan
bijak.
