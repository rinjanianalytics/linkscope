# Instalasi dan menjalankan

LinkScope adalah satu binary dengan antarmuka web yang tertanam di dalamnya. Jalankan dengan
Docker, atau bangun dari kode sumber dengan build Rust biasa.

## Menjalankan dengan Docker

Cara tercepat: Docker dengan Compose v2, tanpa toolchain Rust. Anda memerlukan `compose.yaml` dari
repositori dan konfigurasi Anda.

1. Letakkan konfigurasi Anda di samping `compose.yaml` sebagai `mrtg.cfg`, dengan
   `WorkDir: /var/lib/linkscope`. Konfigurasi MRTG yang sudah ada cukup diubah baris itu saja. Atau
   buat konfigurasi baru (cfgmaker mencetak ke standard output):

   ```sh
   docker compose run --rm -T linkscope cfgmaker --global 'WorkDir: /var/lib/linkscope' \
       --ifref=name public@192.0.2.1:::::2 > mrtg.cfg
   ```

2. Izinkan kontainer membacanya, dan tidak ada pihak lain. Kontainer berjalan sebagai uid 10001, dan
   LinkScope hanya memuat konfigurasi yang dimiliki root atau akunnya sendiri:

   ```sh
   sudo chown root:10001 mrtg.cfg
   sudo chmod 640 mrtg.cfg
   ```

3. Jalankan, lalu buka `http://127.0.0.1:8080/`:

   ```sh
   docker compose up -d
   docker compose logs -f
   ```

Compose menarik image yang sudah dipublikasikan (untuk server amd64 dan arm64), atau membangunnya
dari repositori bila belum ada (pembangunan pertama memakan beberapa menit). Untuk membangun sendiri:
`docker compose build`.

### Apa yang disimpan kontainer dan di mana

| Path di kontainer | Isi | Di host |
|---|---|---|
| `/etc/linkscope/mrtg.cfg` | Konfigurasi Anda, hanya-baca | `./mrtg.cfg` (atau `LINKSCOPE_CONFIG`) |
| `/var/lib/linkscope` | `WorkDir`: log, cache antarmuka, status peringatan, berkas lock dan PID | volume `linkscope-data` |

Kontainer berjalan sebagai akun tanpa hak istimewa dengan sistem berkas hanya-baca, tanpa
capability, dan dengan `no-new-privileges`. Cadangkan volume `linkscope-data` seperti direktori
kerja MRTG.

### Pengaturan Docker

Pengaturan Docker dapat ditaruh di berkas `.env` di samping `compose.yaml` (`.env.example`
mencantumkannya). Setiap pengaturan punya nilai bawaan, jadi berkas ini opsional. Pengaturan SNMP
tetap di `mrtg.cfg`.

| Variabel | Bawaan | Arti |
|---|---|---|
| `LINKSCOPE_LISTEN` | `127.0.0.1:8080` | Tempat antarmuka, API JSON, dan `/metrics` dipublikasikan di host |
| `LINKSCOPE_CONFIG` | `./mrtg.cfg` | Berkas konfigurasi di host |
| `LINKSCOPE_IMAGE` | `ghcr.io/rinjanianalytics/linkscope:0.3.0` | Image yang dijalankan |
| `CARGO_BUILD_JOBS` | `4` | Jumlah job Cargo saat image dibangun di sini |

Antarmuka web tidak memiliki login. Untuk membukanya dari jaringan manajemen, setel
`LINKSCOPE_LISTEN` ke alamat di jaringan itu (misalnya `192.0.2.10:8080`) dan cantumkan nama yang
Anda pakai untuk membukanya di `WebHosts:` (lihat [Antarmuka web](web-ui.md)).

### Menjangkau perangkat Anda

Kontainer melakukan polling melalui network address translation Docker, sehingga perangkat Anda
melihat alamat host Docker, sama seperti melihat MRTG yang berjalan di host itu: izinkan alamat
tersebut di daftar akses SNMP perangkat. Perangkat di jaringan Docker lain tidak terjangkau dari
kontainer kecuali kontainer bergabung ke jaringan itu.

### Memindahkan riwayat MRTG

Untuk mempertahankan grafik dari instalasi MRTG yang sudah ada, salin log-nya ke volume sekali saja,
sebelum mulai pertama kali (`/var/www/mrtg` hanya contoh; pakai `WorkDir` lama Anda):

```sh
docker compose run --rm --user root -v /var/www/mrtg:/old:ro --entrypoint sh linkscope \
    -c 'cp -r /old/. /var/lib/linkscope/ && chown -R 10001:10001 /var/lib/linkscope'
```

### Memperbarui

```sh
docker compose pull
docker compose up -d
```

## Membangun dari kode sumber

Anda memerlukan Rust 1.89 atau yang lebih baru.

```sh
git clone https://github.com/rinjanianalytics/linkscope.git
cd linkscope
cargo build --release
```

Binary-nya adalah `target/release/linkscope`. Pasang di salah satu direktori
dalam `PATH`:

```sh
sudo install -m 0755 target/release/linkscope /usr/local/bin/linkscope
```

Untuk menjalankan `cfgmaker` dengan namanya sendiri, tambahkan symlink:

```sh
sudo ln -s /usr/local/bin/linkscope /usr/local/bin/cfgmaker
```

Anda tidak memerlukan Node.js untuk membangun LinkScope. Node.js hanya
diperlukan bila Anda mengubah antarmuka webnya sendiri (kode sumbernya ada di
`crates/web/ui`).

## Menyiapkan konfigurasi

LinkScope membaca format konfigurasi MRTG. Jika Anda belum punya konfigurasi,
buat dengan [cfgmaker](cfgmaker.md):

```sh
linkscope cfgmaker --global 'WorkDir: /var/lib/mrtg' --ifref=name \
    --output /etc/mrtg/mrtg.cfg public@192.0.2.1:::::2
```

Konfigurasi harus menetapkan `WorkDir:` (atau `LogDir:`, `HtmlDir:`, dan
`ImageDir:`). Log disimpan di bawahnya.

Periksa konfigurasi sebelum menjalankannya pertama kali:

```sh
linkscope --check /etc/mrtg/mrtg.cfg
```

`--check` membaca dan memvalidasi konfigurasi, mencetak jumlah target dan
sumber data, lalu keluar. Opsi ini tidak melakukan polling apa pun.

### Izin berkas

Konfigurasi menyimpan kredensial SNMP, dan LinkScope menolak berkas
konfigurasi (atau berkas yang di-include) yang dapat diubah oleh akun lain:

- berkas harus dimiliki oleh root atau oleh user yang menjalankan LinkScope;
- berkas tidak boleh dapat ditulis oleh grup atau pengguna lain;
- direktorinya tidak boleh dapat ditulis oleh semua orang tanpa sticky bit.

Pengaturan yang baik untuk user layanan bernama `mrtg`:

```sh
sudo chown root:mrtg /etc/mrtg/mrtg.cfg
sudo chmod 640 /etc/mrtg/mrtg.cfg
```

Dengan mode 640 dan grup `mrtg`, daemon dapat membaca berkas itu lagi saat
reload setelah menurunkan hak aksesnya. `--allow-insecure-config` mengubah
penolakan itu menjadi peringatan; hindari opsi ini di luar pengujian.

Pastikan `WorkDir` hanya dapat ditulis oleh akun layanan.

## Menjalankan sekali, seperti MRTG dari cron

Tanpa `--daemon`, LinkScope melakukan polling ke setiap target sekali lalu
keluar, seperti MRTG dari cron:

```sh
linkscope /etc/mrtg/mrtg.cfg
```

Entri crontab untuk `Interval` 5 menit:

```text
*/5 * * * * mrtg /usr/local/bin/linkscope /etc/mrtg/mrtg.cfg
```

Kode keluar menunjukkan hasil siklus (lihat [Baris perintah](command-line.md#kode-keluar)).
Dalam mode ini antarmuka web tidak berjalan lagi setelah siklus selesai.

## Menjalankan sebagai daemon dengan antarmuka web

Dalam mode daemon, LinkScope terus berjalan, melakukan polling setiap
`Interval`, dan menyajikan antarmuka web serta `/metrics`:

```sh
linkscope --daemon --user mrtg --listen 127.0.0.1:8080 /etc/mrtg/mrtg.cfg
```

`--listen` menimpa kata kunci `WebListen:` di konfigurasi. Tanpa keduanya,
antarmuka web tidak berjalan. `RunAsDaemon: yes` di konfigurasi berefek sama
dengan `--daemon`.

Secara bawaan daemon melepaskan diri dari terminal, seperti MRTG. Setelah
melepaskan diri, pesannya tidak ditulis ke mana pun kecuali Anda menetapkan
`--logging FILE`. Di bawah systemd atau supervisor lain, gunakan `--foreground`
(atau `NoDetach: yes` di konfigurasi).

### Unit systemd

```ini
[Unit]
Description=LinkScope link utilization monitoring
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/linkscope --daemon --foreground --user mrtg --group mrtg /etc/mrtg/mrtg.cfg
ExecReload=/bin/kill -HUP $MAINPID
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Pesan ditulis ke standard error, sehingga journal menyimpannya
(`journalctl -u linkscope`). Tetapkan `WebListen:` di konfigurasi, atau
tambahkan `--listen 127.0.0.1:8080` ke `ExecStart`.

Di sini LinkScope mulai sebagai root hanya untuk membaca konfigurasi, mengambil
kuncinya, menulis berkas PID, dan membuka listener web. Setelah itu ia turun ke
user dan grup yang Anda tentukan, tanpa grup lain, dan memeriksa bahwa
perubahan itu berhasil.

### Sinyal

| Sinyal | Efek |
|---|---|
| `SIGHUP` | Memuat ulang konfigurasi; jadwal polling tetap |
| `SIGTERM`, `SIGINT` | Berhenti, membatalkan siklus yang sedang berjalan (log diganti secara atomik, jadi tidak ada yang tertulis setengah) |

Daemon juga memuat ulang sendiri saat berkas konfigurasi berubah. Jika
konfigurasi baru mengandung kesalahan, daemon tetap memakai konfigurasi
sebelumnya dan mencatat alasannya. Perubahan pada `WebListen`, `LoadMIBs`, dan
`RunAsDaemon` memerlukan restart.

## Lokasi berkas

| Berkas | Bawaan | Opsi |
|---|---|---|
| Log (`<target>.log`, `.old`) | `WorkDir` (atau `LogDir`), ditambah `Directory[]` target | |
| Cache interface | nama konfigurasi dengan `.ok` (misalnya `/etc/mrtg/mrtg.ok`) | `--confcache-file` |
| Status peringatan | di samping cache interface, dengan `.alerts` | mengikuti `--confcache-file` |
| Berkas kunci | nama konfigurasi dengan `_l` (misalnya `/etc/mrtg/mrtg.cfg_l`) | `--lock-file` |
| Berkas PID (mode daemon) | nama konfigurasi dengan `.pid` | `--pid-file` |
| Pesan | standard error | `--logging FILE` |

Dengan `--fhs`, LinkScope memakai path Filesystem Hierarchy Standard:

| Berkas | Path |
|---|---|
| Cache interface | `/var/lib/mrtg/mrtg.ok` |
| Berkas PID | `/run/mrtg/mrtg.pid` |
| Berkas kunci | `/var/lock/mrtg/mrtg.lck` |
| Pesan | `/var/log/mrtg/mrtg.log` |

Opsi yang Anda berikan secara eksplisit tetap diutamakan. Bila dijalankan
sebagai root dengan `--fhs`, LinkScope membuat direktori-direktori itu dan
menyerahkannya kepada user layanan.

## Langkah berikutnya

- [Pindah dari MRTG](migrating-from-mrtg.md) jika Anda mengganti instalasi MRTG
- [Antarmuka web](web-ui.md) dan [Metrik Prometheus](metrics.md)
- [Peringatan](alerts.md) untuk mendapat pemberitahuan saat sebuah link terlalu sibuk
