# Pemecahan masalah

Mulailah dengan dua pemeriksaan. Keduanya menjawab sebagian besar pertanyaan.

```sh
# Is the configuration valid?
linkscope --check /etc/mrtg/mrtg.cfg
# Is the daemon up? (with the web UI on 127.0.0.1:8080)
curl http://127.0.0.1:8080/healthz
```

`--check` mencetak jumlah target dan sumber data serta `OK`, atau kesalahan
yang ditemukannya. Opsi ini tidak mengambil kunci, jadi Anda dapat
menjalankannya selagi daemon berjalan. `/healthz` menjawab `ok` selama listener
web aktif.

## Tempat melihat pesan

LinkScope menulis pesannya ke standard error, atau ke berkas yang disebut oleh
`--logging` (`/var/log/mrtg/mrtg.log` dengan `--fhs`). Di bawah systemd, baca
dengan `journalctl -u linkscope`. Daemon yang melepaskan diri tanpa
`--logging` tidak menulis pesannya ke mana pun, jadi tetapkan `--logging FILE`
atau jalankan dengan `--foreground`.

Untuk detail lebih banyak, naikkan tingkat log dengan `--debug all` atau
`LINKSCOPE_LOG=debug`.

Setelah setiap siklus, daemon mencatat satu baris:

```text
cycle: 120 targets in 1830 ms (2 failed, 0 log write failures)
```

## Kontainer tidak dapat membaca konfigurasinya

`cannot read /etc/linkscope/mrtg.cfg: Permission denied` di `docker compose logs` berarti akun
kontainer (uid 10001) tidak dapat membaca berkas yang Anda pasang. Serahkan berkas itu ke root,
dapat dibaca oleh grup kontainer:

```sh
sudo chown root:10001 mrtg.cfg
sudo chmod 640 mrtg.cfg
docker compose restart
```

Berkas milik akun Anda sendiri ditolak meskipun dapat dibaca, karena LinkScope hanya memuat
konfigurasi milik root atau akunnya sendiri (lihat
[Menjalankan dengan Docker](install.md#menjalankan-dengan-docker)).

## Konfigurasi ditolak karena izinnya

LinkScope menolak berkas konfigurasi, atau berkas yang di-include-nya, yang
dapat diubah oleh akun lain, karena konfigurasi menentukan apa yang dijalankan
LinkScope. Pesannya menyebutkan alasannya, misalnya:

```text
writable by group or others (mode 664); run chmod go-w on it
```

Perbaiki berkasnya, bukan pemeriksaannya:

```sh
sudo chown root:mrtg /etc/mrtg/mrtg.cfg
sudo chmod 640 /etc/mrtg/mrtg.cfg
```

Berkas harus dimiliki oleh root atau oleh user yang menjalankan LinkScope.
Setiap direktori di jalurnya harus milik root atau user itu dan tidak boleh
dapat ditulis pihak lain, atau oleh grup selain milik root atau milik user itu
sendiri, kecuali direktori sticky. Pesannya menyebut direktori tersebut. Lihat
[Izin berkas](install.md#izin-berkas). `--allow-insecure-config` mengubah
kesalahan itu menjadi peringatan, hanya untuk pengujian.

## Laju terlihat terlalu rendah, atau melompat-lompat, pada link cepat

SNMPv1 hanya memiliki counter 32-bit. Pada 1 Gbit/s, counter oktet 32-bit
berputar ulang setiap 34 detik, sehingga polling 5 menit tidak dapat melihat
berapa kali counter itu berputar. Lakukan polling target interface dengan
SNMPv2c atau v3, yang memakai counter 64-bit. Versi adalah kolom kelima setelah
host:

```text
Target[core-uplink]: #ether1:public@192.0.2.1:::::2
```

Buat konfigurasi dengan cara yang sama:

```sh
linkscope cfgmaker --ifref=name public@192.0.2.1:::::2
```

`cfgmaker` tanpa versi menulis target SNMPv1, seperti cfgmaker milik MRTG.

## Target berstatus Error, Stale, atau Pending

[Antarmuka web](web-ui.md) memberi setiap target sebuah status:

| Status | Arti | Yang perlu diperiksa |
|---|---|---|
| Pending | Belum di-poll sejak LinkScope mulai | Tunggu satu `Interval` |
| Error | Polling terakhir tidak menghasilkan data | Agen dapat dijangkau dari poller, community atau user SNMPv3 benar, interface masih ada |
| Stale | Polling terakhir yang berhasil lebih lama dari dua interval | Daemon masih berjalan dan siklusnya selesai (lihat di bawah) |
| Warn | Salah satu arah tidak memiliki nilai, atau ada peringatan ambang batas yang aktif | Halaman target menunjukkan batas mana yang terlampaui |

LinkScope hanya menerima balasan SNMP dari alamat yang di-poll-nya. Agen yang
menjawab dari alamat lain (router dengan beberapa alamat, NAT di tengah jalan)
terlihat seperti timeout. Lakukan polling ke alamat yang dipakai agen untuk
menjawab.

Target interface yang dicari berdasarkan nama (`#ether1`), deskripsi
(`\eth0`), alamat IP (`/192.0.2.1`), atau alamat MAC (`!0a-1b-2c-3d-4e-5f`)
dipetakan melalui cache interface (berkas `.ok`). Bila pemetaan di cache tidak
lagi cocok (router menomori ulang interface-nya), LinkScope melupakannya dan
memindai perangkat sekali lagi. Jika interface tetap tidak ditemukan, target
melaporkan `could not match ...`; periksa bahwa nama atau alamat itu masih ada
di perangkat. Pemindaian yang gagal dicatat sebagai
`interface scan of ... failed`.

## Siklus lebih lama dari Interval

Pesan seperti ini berarti siklus tidak selesai tepat waktu:

```text
polling took longer than Interval (300 s); consider raising --concurrency
```

LinkScope melakukan polling ke paling banyak `--concurrency` agen sekaligus
(bawaan 256). Setiap agen boleh memakai paling banyak 80% dari `Interval` per
siklus; setelah itu targetnya tidak diketahui untuk siklus tersebut dan yang
lain tetap berjalan:

```text
192.0.2.1 did not finish within 240 s; its targets are unknown this cycle
```

Naikkan `--concurrency` untuk agen yang banyak, dan periksa agen yang lambat
atau tidak dapat dijangkau.

## LinkScope lain sedang berjalan

```text
another LinkScope instance is running with this configuration (lock /etc/mrtg/mrtg.cfg_l is held)
```

Hanya satu LinkScope (atau MRTG) yang boleh melakukan polling untuk satu
konfigurasi pada satu waktu. Periksa daemon, job cron, atau MRTG yang masih
berjalan dengan konfigurasi yang sama. Kunci ini adalah `flock` yang dipegang
selama proses berjalan, jadi kunci hilang saat proses itu keluar; tidak ada
kunci basi yang perlu dihapus.

Dalam mode daemon, berkas PID yang menyebut proses yang masih berjalan
mencegah daemon kedua dengan cara yang sama
(`LinkScope is already running with pid ...`). Berkas PID yang ditinggalkan
proses yang sudah keluar akan diganti.

## Log disisihkan sebagai `.future`

`the log's last update ... is more than a day ahead of now; set aside as <name>.future` berarti log
mencatat pembaruan lebih dari sehari di masa depan, biasanya setelah jam sistem salah. MRTG akan
menolak memperbaruinya sampai waktu itu tiba. LinkScope menyimpan log lama sebagai `<name>.future`
dan memulai log baru. Jika riwayatnya penting, perbaiki jam, hentikan LinkScope, lalu pindahkan
berkas itu kembali.

## Log tidak dapat ditulis

```text
3 of 120 logs could not be read or written this cycle; check permissions of /var/lib/mrtg/
```

User layanan harus dapat menulis ke `WorkDir` (atau `LogDir`) dan
`Directory[]` di bawahnya. LinkScope juga menolak log yang berupa symlink, dan
`Directory[]` yang berupa symlink. Log yang tidak dapat ditulis ditampilkan
sebagai kesalahan di antarmuka web, tetapi kode keluar tetap hanya menunjukkan
apakah nilai berhasil dikumpulkan.

## Perubahan konfigurasi tidak terbaca

Daemon memuat ulang saat menerima `SIGHUP` dan saat berkas konfigurasi
berubah. Jika konfigurasi baru mengandung kesalahan, daemon tetap memakai yang
sebelumnya dan mencatat:

```text
keeping the previous configuration; reload failed: ...
```

Perbaiki kesalahannya dan simpan lagi. User layanan harus dapat membaca
konfigurasi untuk memuat ulangnya setelah menurunkan hak akses (mode 640
dengan grup layanan). Perubahan pada `WebListen`, `LoadMIBs`, dan `RunAsDaemon`
memerlukan restart.

## Antarmuka web tidak menjawab

- Pastikan `WebListen:` atau `--listen` ditetapkan; tanpa itu tidak ada
  listener yang berjalan.
- Dari cron (tanpa `--daemon`), antarmuka web hanya berjalan selama siklus.
- `cannot listen on ...` saat mulai berarti alamatnya salah atau port-nya
  sudah dipakai.
- Antarmuka web tidak memiliki login. Jika Anda mengaksesnya melalui reverse
  proxy, periksa autentikasi proxy itu dan alamat upstream-nya.
- `421` dan "does not answer to this host name" berarti Anda membukanya dengan
  nama yang tidak ada di `WebHosts:`. Tambahkan nama itu (seperti yang
  ditampilkan browser, tanpa port) lalu muat ulang konfigurasi.

## Kode keluar dari cron

| Kode | Arti |
|---|---|
| `0` | Setiap target berhasil di-poll |
| `91` | Sebagian target gagal |
| `92` | Semua target gagal |
| `1` | LinkScope tidak dapat mulai, atau siklus terputus |

Lihat [Baris perintah](command-line.md#kode-keluar).
