# Keamanan

MRTG memperlakukan konfigurasinya sebagai kode dan memercayai apa pun yang dikirim perangkat.
LinkScope mempertahankan format konfigurasi yang sama tetapi menutup celah tersebut. Halaman ini
menjelaskan caranya, dan cara menjalankan LinkScope dengan aman.

## Konfigurasi adalah data

Di MRTG, `Target[]` dijalankan sebagai Perl, `SnmpOptions` dan `ConversionCode` dievaluasi, dan
target backtick berjalan melalui `/bin/sh`. Siapa pun yang dapat menulis konfigurasi, atau berkas
apa pun yang di-include-nya, dapat menjalankan kode sebagai akun MRTG.

Di LinkScope:

- Ekspresi Target dikompilasi menjadi pohon aritmetika. Tidak ada yang dijalankan sebagai kode.
- `SnmpOptions` dibaca sebagai data.
- Kode konversi tidak didukung.
- Perintah eksternal berjalan tanpa shell, kecuali Anda menetapkan `ExecShell: yes`.
- Berkas konfigurasi yang dapat diubah akun lain ditolak (lihat di bawah).

`ExecShell: yes` hanya menentukan apakah LinkScope sendiri membaca sintaks shell. Ini bukan
sandbox. Siapa pun yang dapat menyunting konfigurasi tetap dapat menjalankan program sebagai akun
layanan (perintah eksternal, program ambang batas). Karena itulah berkasnya sendiri diperiksa.

### Berkas konfigurasi tepercaya

Setiap berkas konfigurasi, termasuk setiap berkas yang di-include, harus dimiliki oleh root atau
oleh akun yang menjalankan LinkScope, dan tidak boleh dapat ditulis oleh grup atau pihak lain. Setiap
direktori di jalurnya harus milik root atau akun itu dan tidak boleh mengizinkan pihak lain mengganti
nama isinya: tidak dapat ditulis pihak lain, maupun oleh grup selain milik root atau milik akun itu
sendiri, kecuali direktori sticky. LinkScope memeriksa berkas yang benar-benar dibukanya, bukan hanya
namanya. `--allow-insecure-config` tetap memuat berkas seperti itu dengan
peringatan.

## Kredensial tidak pernah keluar

Konfigurasi menyimpan community SNMP, kata sandi SNMPv3, URL webhook yang mungkin membawa token,
rahasia penanda tangan webhook, dan kata sandi mail. Semua itu tidak pernah keluar dari LinkScope:

- Setiap string yang dikembalikan API JSON (kesalahan, peringatan, teks Target) melewati satu tahap
  yang menghapus semua community, kata sandi SNMPv3, dan baris perintah eksternal yang dikonfigurasi.
  Kesalahan saat berjalan menyebut nama target, bukan teks Target-nya.
- Kredensial tidak diikutkan saat pengaturan diserialisasi.
- URL webhook, rahasianya, dan kata sandi mail tidak pernah muncul di antarmuka web, API,
  `/metrics`, webhook, atau pesan kesalahan.
- Path program ambang dan alamat email ambang juga tidak ada di API: API hanya menyebut apakah
  sebuah target memilikinya.
- Email ambang, argumen program ambang, dan `THRESH_DESC`-nya mengganti setiap rahasia yang
  dikonfigurasi dengan `•••`, seperti body webhook, bila judul, deskripsi, atau `PageTop[]`
  mengutipnya.
- Klien webhook mengabaikan pengaturan proxy di lingkungannya, sehingga tidak ada proxy yang melihat
  body bertanda tangan.
- LinkScope memberi peringatan saat mulai bila peringatan akan dikirim tanpa enkripsi: email ambang
  sebagai SMTP biasa ke host lain, atau webhook `http://` ke host lain.
- Sebuah pengujian menanam rahasia di konfigurasi lalu mencarinya di output setiap endpoint.

## Antarmuka web

- Antarmuka web menampilkan data perangkat (sysName, uptime, nama router) sebagai teks, sehingga
  perangkat tidak dapat menyisipkan HTML atau skrip.
- HTML dari konfigurasi (`PageTop`, `Title`, legenda) disaring hanya ke tag yang diizinkan, tanpa
  skrip, style, atau event handler.
- Setiap respons membawa Content-Security-Policy yang ketat (skrip hanya dari LinkScope sendiri,
  tanpa framing, tanpa skrip inline), `X-Content-Type-Options: nosniff`, `Referrer-Policy:
  no-referrer`, dan `X-Frame-Options: DENY`.
- Antarmuka web hanya-baca dan **tidak memiliki login**. Tidak ada listener web sampai Anda
  menetapkan `WebListen` atau `--listen`. Ikat ke loopback atau jaringan manajemen tepercaya, dan
  pasang reverse proxy dengan autentikasi di depannya sebelum membukanya lebih luas.
- Antarmuka hanya menjawab alamat IP, `localhost`, dan nama di `WebHosts:`, sehingga halaman web
  yang mengarahkan namanya sendiri ke alamat antarmuka (DNS rebinding) tidak dapat membacanya dari
  browser.
- Antarmuka melayani paling banyak 256 koneksi sekaligus (sisanya menunggu di antrean kernel dan
  tidak memakai file descriptor daemon, yang dibutuhkan poller), header permintaan harus tiba dalam
  10 detik, dan koneksi ditutup setelah 120 detik. Klien yang menahan koneksi tidak dapat
  menghentikan polling. Log diringkas untuk grafik beberapa sekaligus saja.

## Perangkat tidak dipercaya

### cfgmaker

cfgmaker milik MRTG menulis sysName ke `Title[]` apa adanya, dan baris baru atau backtick di string
perangkat ikut masuk ke konfigurasi yang dihasilkan. cfgmaker milik LinkScope:

- membuang karakter kontrol dari setiap string perangkat;
- kembali memakai ifIndex untuk rujukan antarmuka yang tidak dapat ditulis dengan aman;
- meng-escape nilai yang berakhir di HTML, termasuk sysName.

Sebuah pengujian memasukkan string berbahaya ke cfgmaker lalu mem-parsing hasilnya.

### Balasan SNMP

Balasan hanya diterima dari alamat yang di-poll (socket UDP yang terhubung), dan hanya dengan
request id dan community yang benar. SNMPv3 authPriv (SHA-1 atau SHA-2 dengan AES atau DES)
didukung dan dianjurkan.

### Agen yang berbahaya atau lambat

- Setiap agen memiliki jatah waktu per siklus (80 % dari `Interval`).
- Penelusuran tabel dibatasi oleh waktu, jumlah baris (100.000), dan jumlah byte, dengan setiap
  baris dihitung sesuai biayanya. Cache antarmuka menyimpan paling banyak 8 MB per perangkat,
  sehingga berkas `.ok` tetap kecil.
- Jawaban GET hanya dipakai untuk objek yang disebutnya, sehingga balasan nyasar atau basi tidak
  dapat memasang counter satu antarmuka pada antarmuka lain.
- Setelah permintaan yang tidak dijawab, permintaan berikutnya dikirim dari socket baru, sehingga
  balasan terlambat untuk permintaan lama tidak dapat dibaca sebagai jawaban yang baru.
- String perangkat (sysName, uptime) disimpan paling banyak 256 karakter.
- Angka yang lebih panjang dari 64 karakter ditolak, begitu pula nilai yang lebih lebar dari 190
  digit (batas MRTG), yang dinilai sebelum diuraikan: `1e999999999` bukan angka.
- Sesi SNMPv3 yang balasannya ditolak, atau yang autentikasinya gagal, dibuka lagi dengan socket
  baru dan discovery baru, sehingga balasan palsu tidak dapat merusaknya untuk seterusnya.
- Pengaturan SNMPv3 yang meminta lebih sedikit dari kesannya (kata sandi privasi tanpa kata sandi
  autentikasi, protokol tanpa kata sandinya, kata sandi kosong) adalah kesalahan, bukan tingkat
  keamanan yang diam-diam lebih rendah.
- Agen yang macet menunda siklus paling lama sebesar jatahnya dan tidak dapat menghalangi agen lain.
- Kegagalan pada satu target hanya berdampak pada target itu.

## Berkas

MRTG mengikuti symlink sebagai root (saat mengubah pemilik direktori dengan `--fhs`, dan untuk
berkas pid sebelum ada perbaikan). LinkScope:

- membuka direktori dengan `O_DIRECTORY|O_NOFOLLOW` dan mengubah pemiliknya dengan `fchown`;
- mengambil kuncinya sebelum membuat berkas pid dengan `O_EXCL|O_NOFOLLOW`, dan tidak pernah
  membaca berkas pid yang tertinggal (kunci yang membedakan instans yang sedang berjalan; berkas
  basi di-unlink);
- menulis setiap berkas output (log, cache, status ambang batas) ke berkas sementara baru dengan
  nama yang tidak dapat ditebak, dibuat dengan `O_EXCL|O_NOFOLLOW`, lalu mengganti namanya ke tempat
  tujuan;
- menolak subdirektori `Directory[]` yang berupa symlink, membuatnya satu tingkat demi satu
  tingkat, serta membaca dan menulis log target melalui direktori yang dibukanya (sehingga komponen
  yang ditukar dengan symlink setelah pemeriksaan tidak dapat mengalihkannya);
- membaca log dengan `O_NOFOLLOW|O_NONBLOCK`, hanya jika berupa berkas biasa, dan paling banyak
  4 MB, sehingga FIFO atau berkas perangkat di tempat log tidak dapat membuat daemon macet atau
  kehabisan sumber daya; paling banyak 16 kali jumlah baris log penuh yang disimpan, sehingga log
  berisi baris kosong tidak dapat memakan ratusan MB;
- membaca berkas konfigurasi tanpa memblokir, hanya berkas biasa, paling banyak 1.000 berkas dan
  64 MB untuk satu konfigurasi beserta include-nya.

Cache `.ok` dan status `.alerts` ditulis hanya untuk pemiliknya (mode 600), karena cache, seperti
milik MRTG, menyebut setiap perangkat dengan community SNMP-nya; berkas yang masih dapat dibaca pihak
lain ditulis ulang hanya untuk pemiliknya. Keduanya dibaca seperti log (tanpa symlink, tanpa FIFO,
dengan batas ukuran).

`WorkDir` sendiri dianggap tepercaya: pastikan hanya akun layanan yang dapat menulisinya.

### Path

Nama target tidak boleh mengandung pemisah path. `Directory[]` diubah menjadi bagian relatif biasa
(`/` di depan dibuang, sesuai cara MRTG menggabungkan string, dan `..` ditolak), sehingga setiap log
tetap berada di bawah direktori log.

## Hak akses dan penguncian

- Dengan `--user`, LinkScope memanggil `setgroups` hanya dengan grup tujuan, lalu `setgid` dan
  `setuid`, kemudian memeriksa kedua id dan daftar grup. MRTG mempertahankan grup tambahan dan hanya
  memeriksa id user.
- LinkScope memegang `flock` sepanjang umurnya, sehingga kunci tidak pernah hilang dalam mode daemon.
- Tidak ada proses per target: mesin perhitungan laju berjalan di dalam LinkScope, sehingga fork
  yang gagal tidak dapat menggantikan daemon.

## Keamanan memori

LinkScope ditulis dalam Rust, tanpa kode `unsafe` di parser konfigurasi, evaluator ekspresi, atau
mesin perhitungan laju (mesin ini melarangnya). Nilai yang terlalu panjang atau rusak ditolak
alih-alih meluap dari buffer. Mesin perhitungan laju diuji dengan 2.000 log yang dirusak secara acak.
Paket SNMP dienkode, didekode, dan diautentikasi oleh pustaka `async-snmp` (sejak 2026-10,
menggantikan `snmp2`, yang mengubah status sesi v3 dari sebuah balasan sebelum mengautentikasinya).
Pustaka ini memverifikasi balasan SNMPv3 sebelum mengubah apa pun, membandingkan tanda tangan dalam
waktu konstan, dan melarang kode `unsafe` di kliennya. LinkScope menyematkan versinya dan hanya
menerima balasan dari alamat yang di-poll.

## Menjalankan LinkScope dengan aman

- **Jalankan sebagai akun tanpa hak istimewa**, dengan `--user mrtg` atau `User=` di systemd.
  Dengan `--user`, LinkScope membaca konfigurasi, mengambil kunci, dan membuat berkas pid sebelum
  melepas hak istimewa, lalu membuat direktori log sebagai akun tersebut.
- **Jaga kerahasiaan `mrtg.cfg`.** Berkas ini menyimpan kredensial SNMP. Gunakan
  `chown root:mrtg` dan `chmod 640`, agar daemon dapat membacanya lagi saat reload setelah melepas
  hak istimewa. (`chmod 600` dengan pemilik root juga bisa, tetapi reload akan gagal dan Anda harus
  melakukan restart.)
- **Pastikan `WorkDir` hanya dapat ditulis oleh akun layanan.**
- **Utamakan SNMPv3 dengan authPriv**, dan izinkan agen hanya menjawab alamat poller.
- **Tempatkan listener web di jaringan tepercaya**, di belakang reverse proxy dengan autentikasi
  jika ada pihak lain yang dapat menjangkaunya.
