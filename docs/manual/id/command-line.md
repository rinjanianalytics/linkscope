# Baris perintah

```text
linkscope [options] [config]
linkscope cfgmaker [cfgmaker options] community@router ...
```

`config` adalah berkas konfigurasi dalam format MRTG. Bawaannya
`/etc/mrtg/mrtg.cfg`.

Tanpa `--daemon` (dan tanpa `RunAsDaemon: yes` di konfigurasi), LinkScope
melakukan polling ke setiap target sekali lalu keluar, seperti MRTG dari cron.
Dengan opsi itu, LinkScope terus berjalan dan melakukan polling setiap
`Interval`.

`linkscope cfgmaker` menjalankan pembuat konfigurasi; begitu juga symlink ke
`linkscope` yang bernama `cfgmaker`. Lihat [cfgmaker](cfgmaker.md).

## Opsi dari MRTG

LinkScope menerima setiap opsi perintah `mrtg` milik MRTG.

| Opsi | Fungsi |
|---|---|
| `--user USER` | Berjalan sebagai user ini (nama atau uid) setelah mulai. LinkScope lebih dulu membaca konfigurasi, mengambil kunci, menulis berkas PID, dan membuka listener web, baru kemudian menurunkan hak akses |
| `--group GROUP` | Berjalan sebagai grup ini (nama atau gid). Memerlukan `--user`. Tanpa opsi ini, grup milik user tersebut yang dipakai |
| `--lock-file FILE` | Berkas kunci. Bawaan: path konfigurasi diikuti `_l` |
| `--confcache-file FILE` | Cache interface. Bawaan: path konfigurasi dengan ekstensi `.ok`. Status peringatan disimpan di sampingnya, dengan ekstensi `.alerts` |
| `--logging FILE` | Tulis pesan ke berkas ini, bukan ke standard error. `eventlog` diterima dan berarti standard error |
| `--check` | Periksa konfigurasi, cetak jumlah target dan sumber datanya, lalu keluar. Tidak mengambil kunci |
| `--fhs` | Pakai path Filesystem Hierarchy Standard untuk cache, berkas PID, kunci, dan pesan (lihat [Lokasi berkas](install.md#lokasi-berkas)) |
| `--daemon` | Terus berjalan dan melakukan polling setiap `Interval` |
| `--pid-file FILE` | Berkas PID dalam mode daemon. Bawaan: path konfigurasi dengan ekstensi `.pid` |
| `--debug VALUE` | Diterima untuk kategori debug MRTG. Nilai apa pun menaikkan tingkat log ke debug |
| `--log-only` | Diterima dan tidak berefek: LinkScope tidak pernah menulis halaman HTML |

## Opsi yang ditambahkan LinkScope

| Opsi | Fungsi |
|---|---|
| `--listen ADDR:PORT` | Menyajikan antarmuka web, API JSON, dan `/metrics` di alamat ini. Menimpa `WebListen:` di konfigurasi |
| `--concurrency N` | Berapa agen yang di-poll bersamaan. Bawaan `256` |
| `--foreground` | Dengan `--daemon`: tetap terhubung ke terminal, untuk systemd dan supervisor lain. `NoDetach: yes` di konfigurasi berefek sama |
| `--allow-insecure-config` | Memuat berkas konfigurasi yang dapat diubah akun lain, dengan peringatan alih-alih kesalahan |
| `--help`, `--version` | Mencetak cara pakai atau versi |

Path yang diberikan ke opsi-opsi ini dijadikan absolut saat LinkScope mulai,
sehingga tidak ada yang bergantung pada direktori kerja.

## Lingkungan

| Variabel | Fungsi |
|---|---|
| `LINKSCOPE_LOG` | Tingkat log, misalnya `debug` atau `warn`. Menimpa bawaan `info` dan `--debug` |
| `LINKSCOPE_FOREGROUND` | Bila ditetapkan, daemon tidak melepaskan diri, seperti dengan `--foreground` |
| `SSL_CERT_FILE` | Berkas otoritas sertifikat untuk surel peringatan dan webhook, menggantikan milik sistem (lihat [Peringatan](alerts.md)) |

## Kode keluar

Kode keluar sama dengan milik MRTG.

| Kode | Arti |
|---|---|
| `0` | Setiap target berhasil di-poll. Dalam mode daemon: daemon berhenti secara normal |
| `91` | Sebagian target tidak dapat di-poll pada siklus ini |
| `92` | Tidak ada target yang dapat di-poll pada siklus ini |
| `1` | LinkScope tidak dapat mulai (misalnya kesalahan konfigurasi, kunci sudah dipegang, atau berkas yang tidak boleh dipakainya), atau satu siklus terputus oleh `SIGTERM` atau `SIGINT` |

Log yang tidak dapat ditulis dilaporkan sebagai kesalahan di antarmuka web dan
di pesan, tetapi tidak mengubah kode keluar. Seperti MRTG, kode keluar hanya
menunjukkan apakah nilai berhasil dikumpulkan.

## Contoh

Polling sekali dengan konfigurasi bawaan:

```sh
linkscope
```

Memeriksa konfigurasi selagi daemon berjalan:

```sh
linkscope --check /etc/mrtg/mrtg.cfg
```

Berjalan di bawah systemd dengan antarmuka web di loopback:

```sh
linkscope --daemon --foreground --user mrtg --listen 127.0.0.1:8080 /etc/mrtg/mrtg.cfg
```

Polling jaringan besar dengan lebih banyak agen sekaligus:

```sh
linkscope --daemon --concurrency 512 --logging /var/log/linkscope.log /etc/mrtg/mrtg.cfg
```
