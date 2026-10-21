# cfgmaker

`cfgmaker` menanyakan daftar antarmuka ke sebuah perangkat lalu menulis konfigurasi untuknya,
seperti cfgmaker milik MRTG. Fitur ini sudah ada di dalam LinkScope, jadi tidak ada program
terpisah yang perlu dipasang.

## Menjalankannya

```sh
linkscope cfgmaker --global 'WorkDir: /var/lib/mrtg' --ifref=name \
    --output /etc/mrtg/mrtg.cfg public@192.0.2.1:::::2
```

Symlink bernama `cfgmaker` yang menunjuk ke `linkscope` juga dapat dipakai:

```sh
ln -s /usr/local/bin/linkscope /usr/local/bin/cfgmaker
cfgmaker public@192.0.2.1:::::2
```

Tanpa `--output`, konfigurasi ditulis ke standard output. Dengan opsi itu, LinkScope menulis
berkas dengan mode 0640, sehingga kredensial SNMP di dalamnya tidak dapat dibaca oleh semua akun.

## Perangkat

Setiap perangkat adalah satu argumen:

```text
[community@]router[:[port]:[timeout]:[retries]:[backoff]:[version]]
```

Kolom setelah nama sama dengan di `Target[]` (lihat
[Konfigurasi](configuration.md#pengaturan-koneksi)). Tanpa community, cfgmaker memakai
`--community`, atau `public`. Alamat IPv6 ditulis di dalam kurung siku: `public@[2001:db8::1]:::::2`.

### Gunakan SNMP versi 2 atau 3

Tanpa versi, cfgmaker menulis target SNMPv1, sama seperti milik MRTG. Penghitung versi 1
berukuran 32 bit dan kembali ke nol setiap 34 detik pada 1 Gbit/s, sehingga grafik dan
[metrik Prometheus](metrics.md) untuk link yang sibuk menjadi salah. Tuliskan versi sebagai
kolom kelima, dengan lima titik dua setelah nama:

```sh
linkscope cfgmaker public@192.0.2.1:::::2
```

### SNMPv3

Berikan user dan kunci sebagai opsi, dan versi 3 untuk perangkatnya:

```sh
linkscope cfgmaker --username linkscope --authprotocol sha256 \
    --authpassword 'example-auth-pass' --privprotocol aes \
    --privpassword 'example-priv-pass' 192.0.2.1:::::3
```

Opsi tersebut ditulis ke `SnmpOptions[]` setiap target. Opsi hanya berlaku untuk perangkat dengan
versi 3; untuk perangkat lain opsi itu dihilangkan. Baris perintah di header berkas menampilkan
kata sandi, kunci, dan community (opsi maupun `community@router`) sebagai `XXXXX`.

cfgmaker menulis paling banyak 50.000 antarmuka untuk satu perangkat; perangkat yang melaporkan
lebih banyak adalah kesalahan.

## Opsi

Opsi berlaku untuk perangkat yang menyusul sesudahnya di baris perintah, sehingga perangkat yang
berbeda dapat memiliki pengaturan yang berbeda.

| Opsi | Arti |
|---|---|
| `--global 'Key: value'` | Tambahkan baris global ke output. Ulangi untuk beberapa baris. |
| `--output FILE` | Tulis konfigurasi ke `FILE`, bukan ke standard output |
| `--ifref=TYPE[,TYPE...]` | Cara target merujuk antarmuka: `nr` (ifIndex), `ip`, `eth` (MAC), `descr`, `name`, atau `type`. Yang pertama dan unik untuk antarmuka tersebut yang dipakai. |
| `--ifdesc=TYPE[,TYPE...]` | Cara judul mendeskripsikan antarmuka: `nr`, `ip`, `eth`, `descr`, `name`, `alias`, atau `type` |
| `--if-filter=EXPR` | Hanya sertakan antarmuka yang membuat `EXPR` bernilai benar (lihat di bawah) |
| `--community=STRING` | Community untuk perangkat yang ditulis tanpa community (default `public`) |
| `--snmp-options=:[port]:[timeout]:[retries]:[backoff]:[version]` | Kolom koneksi untuk perangkat yang ditulis tanpa kolom tersebut |
| `--dns-domain=DOMAIN` | Tambahkan `DOMAIN` ke nama perangkat |
| `--no-down` | Jangan lewati antarmuka yang sedang down |
| `--show-op-down` | Sertakan antarmuka yang down secara operasional |
| `--noreversedns` | Jangan cari nama untuk alamat antarmuka |
| `--subdirs=FORMAT` | Taruh log setiap perangkat di subdirektori; `HOSTNAME` dan `SNMPNAME` di `FORMAT` diganti |
| `--nointerfaces` | Tulis header perangkat saja, tanpa target antarmuka |
| `--zero-speed=BPS` | Kecepatan yang dianggap untuk antarmuka yang tidak melaporkan kecepatan |
| `--enable-ipv6` | Izinkan SNMP melalui IPv6 (juga menulis `EnableIPv6: yes`) |
| `--username`, `--authprotocol`, `--authpassword`, `--privprotocol`, `--privpassword`, `--contextname` | Pengaturan SNMPv3 (lihat di atas) |
| `--help`, `--version` | Tampilkan cara pakai atau versi |

Tanpa `--ifref`, antarmuka dirujuk dengan ifName bila perangkat melaporkannya, dan dengan ifIndex
bila tidak. `--ifref=name` membuat pilihan ini eksplisit. Rujukan antarmuka yang tidak dapat
ditulis dengan aman (misalnya nama yang mengandung backtick) kembali memakai ifIndex.

Inilah yang juga dilakukan cfgmaker MRTG 2.17.10, sesuai dengan `--help`-nya
(`--ifref=name ... (default)`). Halaman manual MRTG yang lebih panjang masih menyebut bawaannya
`nr`; kalimat itu sudah usang. Untuk tetap memakai rujukan ifIndex, gunakan `--ifref=nr`.

Antarmuka yang dilewati cfgmaker (down, jenis yang tidak digrafikkan, atau ditolak filter) tetap
ditulis dalam bentuk komentar, dengan alasannya di atasnya. Hapus `#` untuk tetap memakainya.

## Filter antarmuka

`--if-filter` menerima bahasa ekspresi kecil yang mencakup filter yang didokumentasikan MRTG:

- variabel seperti `$default`, `$if_type`, `$if_is_ethernet`, `$if_admin`, `$if_oper`,
  `$if_speed`, `$if_snmp_descr`, `$if_snmp_name`, `$if_snmp_alias`, `$if_ip`, dan `$sysname`;
- angka dan string dalam tanda kutip;
- `!`, `not`, `&&`, `and`, `||`, `or`, dan kurung;
- perbandingan angka `== != < <= > >=`, perbandingan string `eq ne lt gt le ge`;
- pencocokan regular expression `=~ /re/i` dan `!~ /re/`.

```sh
# Ethernet interfaces cfgmaker would include anyway, faster than 100 Mbit/s
linkscope cfgmaker --if-filter='$default && $if_is_ethernet && $if_speed > 12500000' \
    public@192.0.2.1:::::2

# Only interfaces whose alias mentions an uplink
linkscope cfgmaker --if-filter='$if_snmp_alias =~ /uplink/i' public@192.0.2.1:::::2
```

`$if_speed` dalam byte per detik. MRTG menjalankan filter sebagai Perl; LinkScope hanya menerima
bahasa ini. `--if-template` dan `--host-template` menjalankan program Perl dan ditolak: gunakan
`--if-filter` dan `--global`, atau sunting berkas yang dihasilkan.

## Output yang aman

String perangkat (sysName, sysDescr, ifDescr, ifName, ifAlias, dan lainnya) berasal dari perangkat,
jadi cfgmaker memperlakukannya sebagai tidak tepercaya. cfgmaker membuang karakter kontrol, sehingga
perangkat tidak dapat menambahkan baris ke konfigurasi Anda, dan meng-escape nilai yang berakhir di
HTML. Lihat [Keamanan](security.md).

## Tidak didukung

- `--if-template` dan `--host-template` (program Perl).
- Kolom VLAN Cisco: `$if_vlan_id` dan `$if_cisco_trunk` tidak tersedia di filter, dan deskripsi
  `catname` serta `ppname` tidak didukung.
- Kunci SNMPv3 terlokalisasi (`--authkey`, `--privkey`): gunakan kata sandinya.
