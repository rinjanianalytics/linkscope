# Instalasi dan menjalankan

LinkScope adalah satu binary. Antarmuka web sudah dibangun sebelumnya dan
tertanam di dalamnya, sehingga Anda cukup melakukan build Rust biasa.

## Membangun dari kode sumber

Anda memerlukan Rust 1.88 atau yang lebih baru.

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
