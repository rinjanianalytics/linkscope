# Kompatibilitas dengan MRTG

LinkScope membaca berkas konfigurasi MRTG 2.17.10 serta membaca dan menulis berkas `.log` MRTG.
LinkScope dapat menggantikan `mrtg` di tempat: konfigurasi yang sama, direktori log yang sama,
riwayat yang sama. Halaman ini menjelaskan apa yang identik, apa yang sengaja berbeda, dan apa yang
tidak didukung.

## Yang identik

| Area | Cara pemeriksaannya |
|---|---|
| Tata bahasa konfigurasi: default `[_]`, prefiks `[^]`, sufiks `[$]`, `NoSpaceChar`, baris lanjutan, `Include:` | Dibandingkan dengan Perl milik MRTG pada sekumpulan berkas tulisan tangan, 1.500 konfigurasi acak, dan 26 konfigurasi di `contrib/` MRTG |
| Validasi dan default (`cfgcheck` MRTG) serta parsing Target (`targparser`) | Perbandingan yang sama: peta kata kunci, sumber data, dan ekspresi Target hasil tulis ulang yang identik, serta konfigurasi yang ditolak juga sama |
| Perhitungan laju dan format log (`rateup`) | Berkas `.log` dan `.old` identik per byte pada 50.000 pembaruan acak terhadap program C, ditambah 2.000 log yang rusak |
| Seluruh alur: polling, pencarian antarmuka berdasarkan nama, deskripsi, IP, MAC, dan jenis, aritmetika, gauge, perminute, AbsMax, nilai yang tidak diketahui, wrap 32-bit, penghitung 64-bit, perintah eksternal | MRTG dan LinkScope dijalankan terhadap agen yang sama pada waktu (palsu) yang sama: 18 target, 6 siklus, setiap `.log` identik per byte (`testenv/parity.sh`) |
| Output cfgmaker | Identik dengan cfgmaker MRTG untuk perangkat versi 2c, kecuali header |

Kebiasaan unik MRTG dipertahankan bila memengaruhi data. Contohnya:

- baris lanjutan setelah kata kunci global diabaikan (LinkScope memberi peringatan);
- prefiks `[^]` multi-baris yang diakhiri `NoSpaceChar` hanya menyimpan baris terakhirnya;
- `ThreshHyst` tanpa `ThreshDir` hanya aktif di dalam pita histeresis.

## Yang sengaja berbeda

### Fitur yang memerlukan Perl

- **Ekspresi Target** dikompilasi, tidak dijalankan sebagai Perl. Ekspresi mendukung sumber data,
  angka (desimal, heksadesimal, oktal, pemisah `_`), `+ - * / % **`, kurung, minus unary, `int()`,
  dan `abs()`, dengan urutan prioritas Perl dan presisi tak terbatas (seperti `Math::BigFloat`).
  Selain itu adalah kesalahan saat berkas dimuat, bukan kegagalan saat polling.
- **`SnmpOptions`** harus berupa pasangan `key => value` sederhana (angka atau string dalam tanda
  kutip). MRTG menjalankannya sebagai Perl.
- **Subrutin konversi** (`|name` di Target, `ConversionCode:`) tidak didukung. Target yang
  memakainya dilaporkan saat berkas dimuat.
- **`LibAdd:`** diabaikan, dengan peringatan.
- **`--if-filter` milik cfgmaker** menerima bahasa ekspresi yang aman (variabel, perbandingan,
  `=~ /re/i`, `&& || !`) yang mencakup contoh yang didokumentasikan MRTG. `--if-template` dan
  `--host-template` menjalankan Perl dan ditolak. Lihat [cfgmaker](cfgmaker.md#filter-antarmuka).

### Perintah eksternal (target backtick)

- Perintah dijalankan **secara langsung, tanpa shell**. Perintah yang memerlukan shell (pipe,
  pengalihan, `$VAR`, glob) adalah kesalahan saat berkas dimuat, kecuali Anda menetapkan
  `ExecShell: yes`.
- Perintah dihentikan setelah 120 dtk, dan outputnya dibatasi 64 KB.

### Output

- **Tidak ada halaman HTML atau berkas PNG yang ditulis.** [Antarmuka web](web-ui.md) bawaan
  menggantikan halaman per target dan `indexmaker`. `HtmlDir`, `ImageDir`, dan kata kunci halaman
  tetap diterima; HTML di `PageTop`, `PageFoot`, `Title`, dan legenda ditampilkan setelah
  disanitasi. `AddHead` dan `BodyTag` tidak dipakai.
- `LogFormat: rrdtool`, `RRDCached`, `SendToGraphite`, dan ambang batas Holt-Winters
  (`HWThresh*`) tidak didukung.

### Keamanan

- Berkas konfigurasi, beserta berkas yang di-include-nya, harus dimiliki oleh root atau oleh akun
  yang menjalankan LinkScope dan tidak dapat ditulis oleh grup atau pihak lain.
  `--allow-insecure-config` mengubah hal ini menjadi peringatan.
- Balasan SNMP hanya diterima dari alamat yang di-poll. Secara default, MRTG mencocokkan balasan
  dengan longgar.
- Nama target dan `Directory[]` tidak boleh mengandung `..` atau pemisah path yang keluar dari
  direktori kerja.
- Direktori dibuat dengan mode 0755 (MRTG memakai 0777 dikurangi umask).

Lihat [Keamanan](security.md) untuk model lengkapnya.

### Bug yang diperbaiki

- **Daemon tetap memegang kuncinya.** Proses induk MRTG yang menjadi daemon menghapus berkas kunci
  yang diandalkan proses anaknya. LinkScope memegang `flock` sepanjang umur proses.
- **Reload tidak lagi menggandakan target.** MRTG terus menambah daftar targetnya pada setiap reload.
- **Cache antarmuka (`.ok`) menyimpan setiap host.** MRTG membuang host yang tidak dipindai ulang
  dalam satu proses setiap kali ada host yang dipindai, dan daemon MRTG tidak pernah menulis cache.
- **`Include:` dengan glob membaca berkas dalam urutan terurut** (MRTG memakai urutan terbalik),
  dan include yang berputar terdeteksi.
- **Nilai TimeTicks yang dipakai sebagai data adalah angka.** MRTG mengubahnya menjadi teks seperti
  "3 days, 1:02:03" lalu menolaknya.
- **cfgmaker:** kesalahan lama dari penelusuran khusus vendor tidak lagi mematikan antarmuka pertama
  pada perangkat SNMPv1 mirip Cisco.
- **cfgmaker:** string perangkat (sysName, ifAlias, dan lainnya) tidak dapat lagi menambahkan baris
  konfigurasi atau perintah `Target[]`.

### Batas yang menjaga LinkScope tetap responsif

- Setiap agen boleh memakai paling banyak 80 % dari `Interval` per siklus. Setelah itu, targetnya
  tidak diketahui untuk siklus tersebut dan target lain tetap berjalan.
- Perintah eksternal: hanya empat baris pertama yang dibaca (seperti di MRTG), lalu grup proses
  perintah dihentikan; timeout-nya 120 dtk.
- Program dan email ambang batas berjalan di latar belakang, delapan sekaligus.
- Nilai lebih dari 64 karakter dari agen atau perintah ditolak.
- Log yang tidak dapat ditulis dilaporkan sebagai kesalahan (di antarmuka web dan log). Seperti di
  MRTG, hal ini tidak mengubah kode keluar, yang hanya menyatakan apakah nilai berhasil dikumpulkan.

### Perbedaan lain

- `--daemon` melepaskan diri seperti MRTG. `--foreground` (atau `NoDetach: yes`) membuatnya tetap
  terhubung, untuk systemd.
- Opsi baru: `--listen`, `--concurrency`, `--foreground`, `--allow-insecure-config`. Kata kunci
  global baru: `WebListen`, `ExecShell`, `AlertWebhook`, `AlertWebhookSecret`, `AlertMailUser`,
  `AlertMailPassword`. Kata kunci per target baru: `ThreshFor[]`. MRTG menolak kata kunci ini, jadi
  jangan masukkan ke konfigurasi yang masih Anda pakai bersama MRTG.
- `ThreshMailServer` juga menerima `host:port`.
- Ambang batas pada target tanpa `ThreshFor[]` menjalankan program dan email persis seperti MRTG.
  Dengan `ThreshFor[]`, keduanya berjalan sekali saat peringatan aktif dan sekali saat pulih.
  Dalam kedua cara, status peringatan juga disimpan di `<config>.alerts`, untuk antarmuka web dan
  `AlertWebhook`. Lihat [Peringatan](alerts.md).
- `--log-only` diterima dan tidak melakukan apa pun (LinkScope hanya menulis log).
- `--check` tidak mengambil kunci, jadi dapat dipakai saat daemon berjalan.
- `Directory[x]: /routers/` berarti `WorkDir/routers/`, sesuai cara MRTG menggabungkan string.
- Setelah `SIGHUP`, daemon memuat ulang konfigurasi dan mempertahankan jadwal polling-nya.
  `SIGTERM` menghentikan siklus yang sedang berjalan. Reload tidak menerapkan perubahan pada
  `WebListen`, `LoadMIBs`, atau `RunAsDaemon` (lakukan restart untuk itu), dan memerlukan
  konfigurasi yang dapat dibaca oleh akun layanan.
- Target cache Squid (`1.3.6.1.4.1.3495.1`): MRTG membaca `cacheUptime` dan `cacheSoftware` untuk
  baris uptime; LinkScope membaca `sysUptime` dan `sysName`. Data grafik tetap sama.
- Jika pembacaan `RouterUptime[]` gagal, uptime dan nama dari sumber data tetap dipakai (MRTG
  mengosongkannya).

## Tidak didukung

- Fitur Perl: subrutin konversi, `ConversionCode`, `--if-template` dan `--host-template` milik
  cfgmaker.
- RRDtool: `LogFormat: rrdtool`, `RRDCached`, opsi `derive`, `HWThresh*`.
- `SendToGraphite`.
- Output HTML dan PNG, serta `indexmaker`.
- Di cfgmaker: kolom VLAN Cisco (`$if_vlan_id`, `$if_cisco_trunk` di filter) serta deskripsi
  `catname` dan `ppname`.
- Kunci SNMPv3 terlokalisasi (`authkey`, `privkey`). Gunakan `authpassword` dan `privpassword`.
