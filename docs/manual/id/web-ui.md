# Antarmuka web

LinkScope memiliki antarmuka web yang sudah tertanam di dalam binary. Antarmuka ini menggantikan
halaman HTML dan PNG yang dulu ditulis oleh MRTG dan `indexmaker`. Antarmuka ini hanya untuk
dibaca, tidak memerlukan akses internet, dan menggambar grafik di browser hanya saat Anda
membukanya.

Panduan ini juga tersedia di antarmuka pada `/docs`, dalam dua bahasa (EN dan ID).

## Mengaktifkannya

Berikan alamat yang akan didengarkan LinkScope, di konfigurasi atau di baris perintah:

```text
WebListen: 127.0.0.1:8080
```

```sh
linkscope --daemon --user mrtg --listen 127.0.0.1:8080 /etc/mrtg/mrtg.cfg
```

`--listen` menggantikan `WebListen:`. LinkScope membuka port sebelum melepas hak aksesnya,
sehingga ia dapat memakai port rendah dan tetap berjalan sebagai `--user`. Jalankan sebagai daemon
(`--daemon`, atau `RunAsDaemon: yes`) agar antarmuka tetap aktif di antara siklus polling.

Listener yang sama melayani:

| Path | Isi |
|---|---|
| `/` | Ringkasan |
| `/target/<name>` | Halaman satu target |
| `/docs` | Panduan ini |
| `/api/...` | [API JSON](api.md) yang dibaca antarmuka |
| `/metrics` | [Metrik Prometheus](metrics.md) |
| `/healthz` | Menjawab `ok` selama server berjalan |

## Siapa yang dapat melihatnya

Antarmuka ini tidak memiliki login. Siapa pun yang dapat menjangkau port tersebut dapat melihat
setiap target, laju trafiknya, dan grafiknya. Dengarkan di `127.0.0.1` atau di jaringan
manajemen, atau pasang reverse proxy dengan autentikasi di depannya. Jangan pernah membukanya ke
internet.

Antarmuka ini tidak pernah menampilkan rahasia. Community SNMP di `Target[]` ditampilkan sebagai
`•••`, dan kata sandi SNMPv3, baris perintah eksternal, URL webhook, serta kata sandi email tidak
pernah keluar dari proses. Lihat [Keamanan](security.md).

## Ringkasan

Ringkasan terbuka di `/`. Bagian atasnya menampilkan:

- **Targets OK**: target yang berhasil di-poll pada siklus terakhir.
- **With problems**: target dengan status warn, error, atau stale, dirinci per status.
- **Pending**: target yang belum di-poll sejak LinkScope dimulai.
- **Last cycle**: kapan siklus polling terakhir berjalan dan berapa lama.
- **Interval**: jeda antar siklus polling.
- **Uptime**: berapa lama daemon sudah berjalan.

Di bawahnya ada satu kartu per target. Kartu menampilkan judul target, statusnya, nama sistem yang
dilaporkan perangkat, laju masuk dan keluar saat ini (diformat seperti cetakan MRTG, dengan
persentase terhadap `MaxBytes`), sparkline satu hari terakhir, dan peringatan ambang yang sedang
aktif atau menunggu.

Anda dapat mempersempit dan mengurutkan daftar:

- **Cari** berdasarkan nama target, judul, atau nama sistem.
- **Saring** berdasarkan status: semua, bermasalah (warn, error, dan stale), atau satu status.
- **Urutkan** berdasarkan nama, judul, atau status.
- **Kelompokkan** berdasarkan `Directory[]`. Pengelompokan tersedia bila target memakai lebih dari
  satu direktori.

Pencarian, saringan, urutan, pengelompokan, dan halaman tersimpan di bilah alamat, sehingga tautan
atau tombol kembali memulihkannya. Ringkasan diperbarui setiap 30 detik.

## Halaman target

Klik sebuah kartu untuk membuka `/target/<name>`. Halaman ini menampilkan:

- Judul, status, dan peringatan pada target beserta sudah berapa lama peringatan itu aktif.
- Nama sistem, uptime, dan kecepatan maksimum (`MaxBytes`), kecuali `Options[]: noinfo` diatur,
  serta waktu poll terakhir.
- Nama target dan teks `Target[]`-nya, dengan community disamarkan.
- `PageTop[]` dan `PageFoot[]`, dibersihkan menjadi subset HTML yang aman (tanpa skrip, gaya, atau
  gambar).
- Satu grafik per periode: hari, minggu, bulan, dan tahun terakhir. `Suppress[]` menyembunyikan
  periode.

Di bawah setiap grafik ada tabel MRTG berisi **Max**, **Average**, dan **Current** untuk masuk dan
keluar, ditambah **Average max** bila `Options[]: avgpeak` diatur. Grafik mengikuti pengaturan
target:

- `Options[]`: `bits`, `growright`, `logscale`, `perminute`, `perhour`, `integer`,
  `nopercent`, `noi`, `noo`, `nolegend`, dan opsi lain yang dipakai MRTG untuk menggambar.
- `kMG[]`, `Kilo[]`, `Factor[]`, `ShortLegend[]`, `YLegend[]`, `LegendI[]`, `LegendO[]`, dan
  `Legend1[]` sampai `Legend5[]`.
- `WithPeak[]` (garis puncak untuk periode yang disebutkan) dan `Unscaled[]` (sumbu y tetap pada
  `MaxBytes`).
- `Colours[]`.

Halaman memuat ulang target setiap 30 detik dan setiap grafik setiap menit. Grafik yang belum
memiliki log menampilkan "No data collected yet".

## Status

Setiap target memiliki satu status. API dan `/metrics` memakai nama di kolom pertama.

| Status | Label di antarmuka | Arti |
|---|---|---|
| `ok` | OK | Berhasil di-poll pada siklus terakhir. |
| `warn` | Warning | Berhasil di-poll, tetapi satu arah tidak diketahui, atau peringatan ambang sedang aktif. |
| `error` | Error | Tidak ada data pada siklus ini, atau log tidak dapat dibaca atau ditulis. |
| `stale` | Stale | Sampel terakhir lebih lama dari dua interval ditambah satu menit. |
| `pending` | Pending | Belum di-poll sejak LinkScope dimulai. |

Batas ambang yang terlampaui tetapi belum mencapai jumlah poll `ThreshFor[]` ditampilkan sebagai
menunggu di bawah target ("1 of 3 polls before it fires"), dan target tetap `ok`. Lihat
[Peringatan](alerts.md).

## Tema

Tombol di kanan atas beralih antara terang dan gelap. Tanpa pilihan yang tersimpan, antarmuka
mengikuti pengaturan sistem.
