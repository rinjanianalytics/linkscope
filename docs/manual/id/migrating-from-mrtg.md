# Pindah dari MRTG

LinkScope membaca konfigurasi yang sama dan melanjutkan berkas `.log` yang sama
dengan MRTG 2.17.10, sehingga Anda dapat mengganti `mrtg` di tempat:
konfigurasi yang sama, direktori log yang sama, riwayat yang sama. Halaman ini
menjelaskan langkah perpindahannya.

## Sebelum memulai

Baca [Kompatibilitas dengan MRTG](compatibility.md) dan periksa konfigurasi
Anda untuk beberapa hal yang tidak didukung LinkScope:

- subrutin konversi (`|name` di dalam `Target[]`, `ConversionCode:`);
- `LogFormat: rrdtool`, `RRDCached`, `SendToGraphite`, dan ambang batas
  Holt-Winters (`HWThresh*`);
- perintah eksternal (target backtick) yang memerlukan shell, kecuali Anda
  menetapkan `ExecShell: yes`;
- `SnmpOptions` yang ditulis sebagai kode Perl, bukan pasangan
  `key => value` biasa;
- kunci lokal SNMPv3 (`authkey`, `privkey`): gunakan `authpassword` dan
  `privpassword` sebagai gantinya.

LinkScope melaporkan setiap hal ini saat memuat konfigurasi, sehingga
pemeriksaan di bawah menemukannya untuk Anda.

## Langkah demi langkah

### 1. Pasang LinkScope

Bangun dan pasang binary seperti dijelaskan di
[Instalasi dan menjalankan](install.md). Jangan hapus MRTG dulu.

### 2. Periksa konfigurasi

Arahkan LinkScope ke konfigurasi Anda yang ada dengan `--check`:

```sh
linkscope --check /etc/mrtg/mrtg.cfg
```

`--check` tidak mengambil kunci, jadi aman dijalankan selagi MRTG masih
berjalan. Perbaiki apa pun yang dilaporkannya. Jika berkas ditolak karena
izinnya, lihat [Izin berkas](install.md#izin-berkas).

### 3. Hentikan MRTG

Hanya satu program yang boleh menulis log. Hentikan MRTG sebelum LinkScope
berjalan:

- jika MRTG berjalan dari cron, jadikan baris crontab-nya komentar;
- jika MRTG berjalan sebagai daemon, hentikan (misalnya `systemctl stop mrtg`)
  dan nonaktifkan.

Tunggu sampai siklus yang sedang berjalan selesai.

### 4. Jalankan LinkScope

Dari cron, ganti `mrtg` dengan `linkscope` pada baris yang sama:

```text
*/5 * * * * mrtg /usr/local/bin/linkscope /etc/mrtg/mrtg.cfg
```

Atau jalankan sebagai daemon dengan antarmuka web (lihat
[Menjalankan sebagai daemon](install.md#menjalankan-sebagai-daemon-dengan-antarmuka-web)):

```sh
linkscope --daemon --user mrtg --listen 127.0.0.1:8080 /etc/mrtg/mrtg.cfg
```

LinkScope melanjutkan berkas `.log` setiap target dari titik terakhir MRTG.
Cache interface (berkas `.ok`) memakai format MRTG, sehingga LinkScope memakai
cache MRTG apa adanya.

### 5. Ganti halaman HTML

LinkScope tidak menulis halaman HTML maupun grafik PNG. Antarmuka web bawaan
menggantikan halaman per target dan `indexmaker`. Tetapkan alamat yang
didengarkannya, dengan `--listen` atau di konfigurasi:

```text
WebListen: 127.0.0.1:8080
```

Antarmuka web tidak memiliki login. Biarkan di loopback, atau pasang reverse
proxy dengan autentikasi di depannya. `HtmlDir`, `ImageDir`, dan kata kunci
halaman tetap diterima, jadi konfigurasi tidak perlu diubah; berkas yang dulu
dibuat MRTG di sana hanya tidak diperbarui lagi.

### 6. Periksa hasilnya

- Buka antarmuka web dan pastikan setiap target sudah di-poll dan berstatus
  `ok` (lihat [Antarmuka web](web-ui.md)).
- Setelah satu siklus, berkas `.log` memiliki baris baru di bagian atas,
  seperti sebelumnya.
- Dari cron, kode keluarnya `0` bila setiap target berhasil di-poll
  (lihat [Kode keluar](command-line.md#kode-keluar)).

## Kembali ke MRTG

Log tetap dalam format MRTG, jadi Anda dapat kembali ke MRTG: hentikan
LinkScope dan jalankan MRTG lagi dengan konfigurasi yang sama.

Dua hal yang perlu diingat:

- MRTG menolak kata kunci yang ditambahkan LinkScope (`WebListen`,
  `ExecShell`, `AlertWebhook`, `AlertWebhookSecret`, `AlertMailUser`,
  `AlertMailPassword`, `ThreshFor[]`). Hapus dulu, atau jangan masukkan ke
  konfigurasi yang Anda pakai bersama MRTG.
- `ThreshMailServer` dengan port (`host:port`) adalah tambahan LinkScope.

## Apa yang berubah bagi Anda

| Dengan MRTG | Dengan LinkScope |
|---|---|
| Berkas HTML dan PNG statis, `indexmaker` | [Antarmuka web](web-ui.md) bawaan |
| Kode Perl di dalam konfigurasi | Ekspresi yang dikompilasi; kode Perl ditolak saat dimuat |
| `rateup` dijalankan untuk setiap target | Mesin perhitungan laju berjalan di dalam proses |
| Ambang batas menjalankan program dan mengirim surel | Sama, ditambah webhook dan surel dengan autentikasi ([Peringatan](alerts.md)) |
| Tidak ada endpoint metrik | [`/metrics`](metrics.md) untuk Prometheus |
| Kunci hilang saat daemon melepaskan diri | Kunci dipegang selama proses berjalan |
