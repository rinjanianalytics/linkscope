# Perangkat gNMI

LinkScope membaca penghitung interface melalui SNMP, seperti MRTG, dan juga melalui gNMI (gRPC
Network Management Interface). Banyak router dan switch saat ini menerbitkan penghitungnya melalui
gNMI, dan sebagian menjadikannya cara utama untuk membacanya. Target gNMI bekerja seperti target
lain: file `.log`, grafik, ambang, peringatan, `/metrics`, dan API yang sama.

MRTG tidak mengenal gNMI, jadi konfigurasi yang memakainya adalah konfigurasi LinkScope.
Konfigurasi tanpa target gNMI dibaca persis seperti sebelumnya.

## Sebuah target

```
Target[srl1_e1-1]: gnmi:ethernet-1/1@srl1.example.net:57400
GnmiOptions[srl1_e1-1]: user=linkscope password-file=/etc/linkscope/srl1.pass ca=/etc/linkscope/ca.pem
MaxBytes[srl1_e1-1]: 1250000000
Title[srl1_e1-1]: srl1 ethernet-1/1
```

`gnmi:<interface>@<host>[:port]` membaca satu interface: penghitung oktet masuk dan keluarnya, yang
menjadi nilai masuk dan keluar target, seperti `ifHCInOctets` dan `ifHCOutOctets` pada SNMP. Nama
interface ditulis seperti perangkat menamainya (`ethernet-1/1`, `Ethernet1`, `xe-0/0/0`,
`GigabitEthernet0/0/0/1`). Alamat IPv6 ditulis dalam kurung siku
(`gnmi:Ethernet1@[2001:db8::1]:6030`). Port bawaannya 9339, port resmi gNMI; banyak perangkat
memakai port lain:

| Platform | Port gNMI yang umum |
|---|---|
| Nokia SR Linux | 57400 |
| Arista EOS | 6030 |
| Cisco IOS XR | 57400 (diatur di konfigurasinya) |
| Cisco IOS XE, NX-OS | 9339 atau 50051 |
| Juniper Junos | 32767 |

Sumber gNMI dapat digabung dengan sumber lain dalam ekspresi, seperti MRTG menggabungkan sumber
SNMP: `Target[both]: gnmi:ethernet-1/1@srl1:57400 + gnmi:ethernet-1/2@srl1:57400`. Beri spasi di
sekitar setiap operator.

## GnmiOptions

`GnmiOptions[x]` menyatakan cara menghubungi perangkat: pasangan `kunci=nilai` yang dipisah spasi.
Atur sekali untuk semua target berikutnya dengan `GnmiOptions[_]:`, seperti `SnmpOptions[]`.

| Kunci | Arti |
|---|---|
| `user=` | Nama pengguna, dikirim bersama setiap permintaan. |
| `password-file=` | File yang baris pertamanya berisi kata sandi. Biarkan hanya akun LinkScope yang dapat membacanya (`chmod 600`); jika tidak, LinkScope memberi peringatan. |
| `password-env=` | Variabel lingkungan yang berisi kata sandi, sebagai ganti file. |
| `tls=` | `verify` (bawaan): TLS, dan sertifikat perangkat harus tepercaya dan memuat namanya. `insecure`: TLS tanpa memeriksa sertifikat. `off`: tanpa TLS. |
| `ca=` | Sertifikat CA (PEM) yang dipercaya untuk perangkat ini, sebagai ganti CA sistem. |
| `tls-name=` | Nama yang dicocokkan dengan sertifikat, bila target memakai alamat. |
| `cert=`, `key=` | Sertifikat dan kunci klien (PEM), untuk perangkat yang mewajibkan TLS dua arah. |
| `model=` | Jalur yang dibaca: `openconfig` (bawaan) atau `srl`. Lihat di bawah. |
| `target=` | Nama target gNMI, untuk perangkat di belakang gateway gNMI. |

Kata sandi tidak pernah ditulis di konfigurasi: `password=` ditolak. LinkScope membaca file atau
variabel itu di setiap siklus, jadi kata sandi yang diganti tidak perlu restart. Kata sandi dan
jalur file tidak pernah muncul di antarmuka, API, `/metrics`, peringatan, maupun log.

`tls=insecure` dan `tls=off` berfungsi, tetapi LinkScope memberi peringatan saat mulai dan halaman
target menampilkannya: tanpa sertifikat yang diperiksa, siapa pun di jalur dapat menyamar sebagai
perangkat, dan dengan `tls=off` kata sandi dikirim tanpa enkripsi. Pakai keduanya di lab, bukan di
jaringan produksi.

## Model

Model adalah kumpulan jalur yang dijawab perangkat. LinkScope mengenal dua:

| `model=` | Penghitung masuk dan keluar | Perangkat |
|---|---|---|
| `openconfig` | `/interfaces/interface[name=…]/state/counters/in-octets`, `out-octets` | Arista EOS, Cisco IOS XR, IOS XE dan NX-OS, Juniper Junos, Nokia SR OS, SR Linux dengan OpenConfig aktif, dan perangkat lain yang menerbitkan OpenConfig |
| `srl` | `/interface[name=…]/statistics/in-octets`, `out-octets` | Nokia SR Linux, dengan atau tanpa OpenConfig |

LinkScope juga membaca nama dan waktu boot setiap perangkat (ditampilkan sebagai System dan
Uptime) serta ifIndex setiap interface (di `/metrics`), bila perangkat menjawabnya; perangkat yang
tidak menjawab tetap dibaca penghitungnya.

Platform mana yang sudah diuji tercantum di rencana proyek (`docs/PLAN.md`, "Router models").
Platform bertanda "by spec" menerbitkan penghitung OpenConfig menurut dokumentasinya, tetapi belum
dijalankan dengan LinkScope.

## Cara LinkScope membaca perangkat

Sekali per siklus, LinkScope mengirim satu `Get` gNMI ke setiap perangkat untuk penghitung semua
interface targetnya, dan satu lagi untuk nama, waktu boot, dan ifIndex. Perangkat yang tidak
mendukung `Get` dibaca dengan `Subscribe` mode `ONCE`. Koneksi tetap terbuka di antara siklus.
Perangkat gNMI berbagi batas yang sama dengan SNMP: berapa perangkat dibaca bersamaan, dan berapa
lama satu perangkat boleh memakan waktu per siklus.

Bila perangkat menolak seluruh permintaan (kebanyakan begitu jika satu interface tidak ada),
LinkScope bertanya lagi satu interface demi satu interface, sehingga hanya target dengan nama yang
salah yang gagal.

## Menulis target dengan cfgmaker

`linkscope cfgmaker --gnmi` menulis target untuk setiap interface perangkat gNMI, seperti cfgmaker
untuk SNMP:

```
linkscope cfgmaker --gnmi \
  --gnmi-options 'user=linkscope password-file=/etc/linkscope/srl1.pass ca=/etc/linkscope/ca.pem' \
  --global 'WorkDir: /var/lib/linkscope' --output /etc/linkscope/mrtg.cfg \
  srl1.example.net:57400 srl2.example.net:57400
```

cfgmaker mencoba OpenConfig lebih dulu, lalu model SR Linux (atau hanya model yang diberikan dengan
`model=`), dan menulis model yang ditemukan ke `GnmiOptions[]` setiap target. `MaxBytes[]` diambil
dari kecepatan port. Interface yang mati, atau yang kecepatannya tidak dilaporkan perangkat, ditulis
sebagai komentar; `--show-op-down` juga menulis yang mati. Semua yang dikirim perangkat (nama,
deskripsi) dibersihkan menjadi satu baris sebelum masuk ke konfigurasi.

## Bila target gagal

Halaman target dan API menyebutkan alasannya:

| Pesan | Yang diperiksa |
|---|---|
| `login refused (wrong user or password)` | `user=` dan file kata sandi. |
| `TLS: invalid peer certificate: UnknownIssuer` | Sertifikat perangkat tidak ditandatangani CA yang dipercaya LinkScope: atur `ca=`. |
| `TLS: ... NotValidForName` | Sertifikat tidak memuat nama atau alamat di target: atur `tls-name=`. |
| `unreachable: ...`, `connection refused` | Alamat dan port, dan apakah gNMI aktif di perangkat. |
| `the device does not answer these openconfig paths` | Perangkat tidak punya OpenConfig, atau tidak untuk interface ini: coba `model=srl`, atau periksa nama interface. |
| `no ... counter in the answer` | Perangkat menjawab tanpa penghitung: periksa `model=` dan nama. |

## Mencoba tanpa router

`linkscope-mockagent gnmi` (ikut dibangun bersama proyek) menjawab gNMI seperti router, dengan
penghitung yang terus bertambah, untuk mencoba konfigurasi:

```
linkscope-mockagent gnmi --listen 127.0.0.1:9339 --model srl --interfaces ethernet-1/1,ethernet-1/2
```

Tanpa `--cert` dan `--key`, mock berbicara gRPC biasa, jadi targetnya perlu
`GnmiOptions[x]: tls=off`.
