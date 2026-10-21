# Pengantar

LinkScope memantau seberapa sibuk link Anda. LinkScope melakukan polling ke
router dan switch melalui SNMP, menyimpan riwayat setiap interface, dan
menampilkannya di antarmuka web. LinkScope adalah bagian dari keluarga
FlowScope dan PathScope dari Rinjani Analytics.

LinkScope adalah implementasi ulang [MRTG](https://oss.oetiker.ch/mrtg/), Multi
Router Traffic Grapher karya Tobias Oetiker dan para kontributornya, yang
ditulis dalam Rust. LinkScope diturunkan dari MRTG 2.17.10 dan merupakan
perangkat lunak bebas di bawah GNU General Public License, versi 2 atau (sesuai
pilihan Anda) versi yang lebih baru. LinkScope tidak berafiliasi dengan proyek
MRTG dan tidak didukung secara resmi olehnya.

## Pengganti langsung

LinkScope membaca `mrtg.cfg` Anda yang sudah ada dan melanjutkan berkas `.log`
Anda yang sudah ada. Log yang ditulisnya identik byte demi byte dengan log yang
ditulis MRTG 2.17.10, sehingga Anda dapat memindahkan instalasi tanpa
kehilangan riwayat, dan kembali lagi bila perlu.

LinkScope menerima opsi baris perintah dan kode keluar MRTG, serta menyertakan
`cfgmaker` untuk membuat konfigurasi dari sebuah perangkat.

## Apa yang ditambahkan

- **Antarmuka web di dalam binary.** Ringkasan semua target dan halaman per
  target dengan grafik harian, mingguan, bulanan, dan tahunan. Antarmuka ini
  menggantikan halaman HTML dan PNG statis serta `indexmaker`. Tidak perlu
  akses internet.
- **Metrik Prometheus.** `GET /metrics` pada listener yang sama dengan
  antarmuka web.
- **Peringatan yang sampai ke orang.** Peringatan ambang batas melalui webhook
  bertanda tangan dan melalui surel lewat STARTTLS dengan login, dengan
  pemberitahuan saat aktif dan saat pulih.
- **SNMP modern.** SNMP v1, v2c, dan v3, dengan autentikasi SHA-2 dan privasi
  AES.
- **Kecepatan.** Agen di-poll secara bersamaan dengan permintaan berkelompok,
  dan mesin perhitungan laju berjalan di dalam proses, bukan menjalankan
  `rateup` untuk setiap target.
- **Keamanan.** Konfigurasi tidak pernah dijalankan sebagai kode, perintah
  eksternal berjalan tanpa shell kecuali Anda mengizinkannya, dan keluaran web
  disanitasi. Lihat [Keamanan](security.md).

## Susunan panduan ini

| Halaman | Isi |
|---|---|
| [Instalasi dan menjalankan](install.md) | Membangun, memasang, menjalankan dari cron atau sebagai layanan |
| [Pindah dari MRTG](migrating-from-mrtg.md) | Memindahkan instalasi MRTG yang ada ke LinkScope |
| [Baris perintah](command-line.md) | Setiap opsi dan kode keluar |
| [Konfigurasi](configuration.md) | Kata kunci `mrtg.cfg` yang dibaca LinkScope, lama dan baru |
| [cfgmaker](cfgmaker.md) | Membuat konfigurasi dari sebuah perangkat |
| [Antarmuka web](web-ui.md) | Halaman ringkasan dan halaman target |
| [Metrik Prometheus](metrics.md) | Endpoint `/metrics` |
| [Peringatan](alerts.md) | Ambang batas, webhook, dan surel |
| [API JSON](api.md) | API yang dibaca antarmuka web |
| [Keamanan](security.md) | Cara LinkScope menutup kelemahan MRTG yang diketahui |
| [Kompatibilitas dengan MRTG](compatibility.md) | Semua yang berperilaku berbeda dari MRTG |
| [Pemecahan masalah](troubleshooting.md) | Masalah umum dan cara mengatasinya |
