# Konfigurasi

LinkScope membaca `mrtg.cfg` milik MRTG apa adanya. Anda tidak perlu mengubah atau menulis ulang
berkas tersebut. Halaman ini menjelaskan cara berkas dibaca, sintaks `Target[]`, dan setiap kata
kunci yang diterima LinkScope. Kata kunci tambahan dari LinkScope ditandai **Baru**; MRTG sendiri
menolaknya, jadi jangan masukkan kata kunci tersebut ke berkas yang masih Anda pakai bersama MRTG.

## Cara berkas dibaca

Berkas konfigurasi berisi dua jenis baris:

- **Kata kunci global** berlaku untuk seluruh proses: `Keyword: value`.
- **Kata kunci per target** berlaku untuk satu target: `Keyword[name]: value`.

```ini
WorkDir: /var/lib/mrtg

Target[core-uplink]: 2:public@192.0.2.1:::::2
Title[core-uplink]: Core uplink
MaxBytes[core-uplink]: 125000000
```

Aturannya sama persis dengan MRTG:

- Kata kunci tidak membedakan huruf besar dan kecil. Nama target dibaca dalam huruf kecil.
- Baris yang diawali `#` adalah komentar. Baris kosong diabaikan.
- Baris yang diawali spasi atau tab melanjutkan kata kunci per target sebelumnya. Lanjutan
  setelah kata kunci **global** diabaikan, sama seperti di MRTG, dan LinkScope memberi peringatan.
  Tulis nilai global dalam satu baris.
- Mendefinisikan `Keyword[name]` yang sama dua kali adalah kesalahan, dan pesannya menyebut kedua
  baris.
- Kata kunci yang tidak dikenal adalah kesalahan. Kata kunci milik alat lain yang ditulis sebagai
  `tool*Keyword[name]` diterima tanpa pemeriksaan.
- `Colors` dibaca sebagai `Colours`, dan ejaan lama `Supress` sebagai `Suppress`.

### Default, prefiks, dan sufiks

Tiga target semu menetapkan nilai untuk target yang muncul sesudahnya:

| Bentuk | Arti |
|---|---|
| `Keyword[_]: value` | Default untuk setiap target berikutnya yang tidak menetapkan `Keyword` |
| `Keyword[^]: value` | Prefiks yang ditaruh di depan nilai setiap target berikutnya |
| `Keyword[$]: value` | Sufiks yang ditaruh di belakang nilai setiap target berikutnya |

Nilai kosong (`Options[_]:`) menghapus default, prefiks, atau sufiks tersebut. Prefiks digabung
dengan nilai memakai spasi, begitu juga sufiks. Jika karakter global `NoSpaceChar` mengakhiri
prefiks atau sufiks, karakter itu dibuang dan tidak ada spasi yang ditambahkan.

```ini
Options[_]: growright, bits
Directory[_]: routers
```

### Include

`Include: file` membaca berkas lain di titik tersebut. Path relatif dicoba apa adanya, lalu relatif
terhadap direktori berkas konfigurasi utama. Path dengan `*` adalah glob; LinkScope membaca berkas
yang cocok dalam urutan terurut (MRTG membacanya dalam urutan terbalik). Include boleh bersarang
hingga 16 tingkat, dan berkas yang meng-include dirinya sendiri adalah kesalahan.

### Berkas tepercaya

Berkas konfigurasi dapat menjalankan program (target perintah eksternal dan program ambang batas),
jadi LinkScope hanya memuat berkas yang tidak dapat diubah oleh akun lain. Setiap berkas, termasuk
setiap berkas yang di-include, harus dimiliki oleh root atau oleh akun yang menjalankan LinkScope,
dan baik berkas maupun direktorinya tidak boleh dapat ditulis oleh grup atau pihak lain (direktori
sticky tidak masalah). `--allow-insecure-config` mengubah penolakan ini menjadi peringatan. Lihat
[Keamanan](security.md).

## Pengaturan wajib

| Kata kunci | Aturan |
|---|---|
| `WorkDir` | Tempat berkas `.log` disimpan. Sebagai gantinya, Anda dapat menetapkan ketiga kata kunci `LogDir`, `HtmlDir`, dan `ImageDir`. |
| `Target[name]` | Sumber data untuk target. |
| `Title[name]` | Judul target. |
| `MaxBytes[name]` | Nilai valid tertinggi. Sebagai gantinya, Anda dapat menetapkan `MaxBytes1` dan `MaxBytes2` untuk input dan output. |

LinkScope membuat direktori yang belum ada dengan mode 0755.

## Sintaks Target

Nilai `Target[]` menyebut satu atau beberapa sumber data dan dapat menggabungkannya dengan aritmetika.

### Target antarmuka

Bentuk yang umum membaca penghitung oktet input dan output sebuah antarmuka:

```text
selector:community@host[:port:timeout:retries:backoff:version]
```

Selektor memilih antarmuka:

| Selektor | Contoh | Antarmuka dipilih berdasarkan |
|---|---|---|
| angka | `2:public@192.0.2.1` | ifIndex |
| `/alamat` | `/192.0.2.17:public@192.0.2.1` | alamat IPv4 yang dipasang padanya |
| `\descr` | `\ether1:public@192.0.2.1` | ifDescr |
| `#name` | `#Gi0/1:public@192.0.2.1` | ifName |
| `!mac` | `!00-00-5e-00-53-01:public@192.0.2.1` | alamat MAC-nya |
| `%type` | `%6:public@192.0.2.1` | ifType |

Di dalam deskripsi atau nama, awali spasi, `:`, `&`, atau `@` dengan garis miring terbalik
(`\GigabitEthernet\ 0/1`). Tanda `-` di depan (`-2:public@192.0.2.1`) menukar input dan output.
LinkScope mencari selektor di perangkat dan menyimpan hasilnya di cache antarmuka (`<config>.ok`).

Host berupa nama, alamat IPv4, atau alamat IPv6 di dalam kurung siku (`[2001:db8::1]`).

### Pengaturan koneksi

Bagian setelah host menentukan cara LinkScope berbicara dengan agen. Setiap kolom bersifat
opsional; biarkan kosong untuk memakai nilai default-nya.

| Kolom | Default | Catatan |
|---|---|---|
| port | 161 | |
| timeout | 2 dtk | Antara 0,05 dan 300 dtk |
| retries | 5 | Paling banyak 50 |
| backoff | 1.0 | Timeout bertambah dengan faktor ini pada setiap percobaan ulang, antara 1 dan 10 |
| version | 1 | `1`, `2` (atau `2c`), atau `3` |

Versi adalah kolom kelima, jadi diperlukan lima titik dua setelah host:

```ini
Target[edge]: 2:public@192.0.2.1:::::2
Target[edge-v3]: 2:v3@192.0.2.1:::::3
```

Dengan versi 3, user dan kunci diambil dari `SnmpOptions` (di bawah). Target tetap memerlukan satu
kata sebelum `@`; kata apa pun boleh.

Gunakan versi 2 atau 3 untuk target antarmuka. Hanya versi tersebut yang membaca penghitung 64-bit
(ifHCInOctets dan ifHCOutOctets). Penghitung versi 1 berukuran 32 bit dan kembali ke nol setiap
34 detik pada 1 Gbit/s. `NoHC[name]: yes` membuat target tetap memakai penghitung 32-bit.

### OID eksplisit

Dua OID yang digabung dengan `&` membaca pasangan nilai apa pun:

```ini
Target[cpu]: 1.3.6.1.4.1.9.2.1.58.0&1.3.6.1.4.1.9.2.1.58.0:public@192.0.2.1:::::2
Target[pkts]: ifInUcastPkts.2&ifOutUcastPkts.2:public@192.0.2.1:::::2
```

OID berupa angka bertitik atau nama yang dikenal LinkScope (nama MIB-2, dan nama dari berkas di
`LoadMIBs`). Selektor antarmuka dapat menggantikan indeks: `ifInErrors#Gi0/1&ifOutErrors#Gi0/1:...`.

### Aritmetika

Target dapat menggabungkan sumber dengan angka:

```ini
Target[both-uplinks]: 2:public@192.0.2.1:::::2 + 3:public@192.0.2.1:::::2
Target[in-bits]: 2:public@192.0.2.1:::::2 * 8
```

LinkScope mengompilasi ekspresi saat memuat berkas; ekspresi tidak pernah dijalankan sebagai kode.
Yang diterima: sumber data, angka (desimal, heksadesimal, oktal, dengan pemisah `_`),
`+ - * / % **`, kurung, minus unary, `int()`, dan `abs()`, dengan urutan prioritas Perl dan
presisi tak terbatas. Selain itu adalah kesalahan saat berkas dimuat.

### Perintah eksternal

Perintah di dalam backtick menyediakan nilai:

```ini
Target[mail-queue]: `/usr/local/bin/mailq-count`
```

Perintah mencetak hingga empat baris: nilai input, nilai output, uptime, dan nama. LinkScope
menjalankannya secara langsung, tanpa shell, dengan timeout 120 dtk. Perintah yang memerlukan shell
(pipe, pengalihan, variabel, atau glob) adalah kesalahan kecuali Anda menetapkan `ExecShell: yes`.
`PathAdd` menambahkan direktori ke `PATH` perintah tersebut.

### SNMPv3 dan SnmpOptions

Pengaturan SNMPv3 ditulis di `SnmpOptions`, secara global atau per target, sebagai pasangan
`key => value`. Nilainya berupa angka atau string dalam tanda kutip; LinkScope membacanya sebagai
data, tidak pernah sebagai Perl.

```ini
SnmpOptions[edge-v3]: username => 'linkscope', authprotocol => 'sha256',
  authpassword => 'example-auth-pass', privprotocol => 'aes',
  privpassword => 'example-priv-pass'
```

| Kunci | Nilai |
|---|---|
| `username` | Wajib untuk versi 3 |
| `authprotocol` | `md5` (default), `sha` (atau `sha1`), `sha224`, `sha256`, `sha384`, `sha512` |
| `authpassword` | Tanpa ini, target memakai noAuthNoPriv |
| `privprotocol` | `des` (default), `aes` (atau `aes128`), `aes192`, `aes256` |
| `privpassword` | Bersama `authpassword`, target memakai authPriv; tanpanya, authNoPriv |
| `contextname` | Konteks SNMP |

Utamakan authPriv dengan SHA-2 dan AES. Kunci terlokalisasi (`authkey`, `privkey`) tidak didukung;
gunakan kata sandinya.

`SnmpOptions` juga menerima `port`, `timeout`, `retries`, dan `backoff` (dipakai bila Target
mengosongkannya), `maxrepetitions` (jumlah baris per permintaan GETBULK, default 25), dan
`use_getbulk` (`0` untuk menelusuri tabel dengan GETNEXT).

Subrutin konversi (`|name` setelah host) memerlukan Perl dan tidak didukung; Target yang memakainya
dilaporkan saat berkas dimuat.

## Kata kunci global

| Kata kunci | Default | Arti |
|---|---|---|
| `WorkDir` | tidak ada | Direktori untuk berkas `.log` (dan halaman MRTG) |
| `LogDir`, `HtmlDir`, `ImageDir` | tidak ada | Direktori terpisah, dipakai bersama sebagai pengganti `WorkDir` |
| `Interval` | 5 | Menit di antara polling, atau `M:SS`. Kurang dari 5 menit adalah kesalahan. |
| `RunAsDaemon` | no | Terus berjalan dan melakukan polling setiap `Interval` (seperti `--daemon`) |
| `NoDetach` | no | Dalam mode daemon, tetap terhubung ke terminal (seperti `--foreground`) |
| `MaxAge` | tidak ada | Detik; log yang tidak ditulis lebih lama dari ini tidak diperbarui, dan targetnya gagal |
| `EnableIPv6` | no | Izinkan SNMP melalui IPv6. Per target, `IPv4Only[name]: yes` mematikannya lagi. |
| `EnableSnmpV3` | no | Diterima untuk konfigurasi MRTG (`yes` atau `no`) |
| `SingleRequest` | no | Minta satu OID per permintaan, untuk agen yang tidak bisa menjawab beberapa sekaligus |
| `NoMib2` | no | Jangan membaca sysUptime dan sysName dari agen |
| `LoadMIBs` | tidak ada | Berkas MIB yang nama objeknya boleh dipakai Target |
| `PathAdd` | tidak ada | Direktori yang ditambahkan ke `PATH` perintah eksternal |
| `SnmpOptions` | tidak ada | Pengaturan SNMP untuk setiap target (lihat di atas) |
| `NoSpaceChar` | tidak ada | Satu karakter yang menggabungkan prefiks atau sufiks tanpa spasi |
| `ThreshDir` | tidak ada | Direktori untuk berkas status ambang batas (lihat [Peringatan](alerts.md)) |
| `ThreshHyst` | 0.1 | Histeresis untuk ambang batas, sebagai pecahan |
| `ThreshMailServer` | tidak ada | Server mail untuk email ambang batas. **Baru:** juga `host:port`. Memerlukan `ThreshMailSender`. |
| `ThreshMailSender` | tidak ada | Alamat pengirim email ambang batas. Memerlukan `ThreshMailServer`. |
| `LogFormat` | rateup | Hanya `rateup` (berkas `.log` MRTG); `rrdtool` adalah kesalahan |
| `Forks`, `Refresh`, `Language`, `WriteExpires`, `IconDir` | | Diterima untuk konfigurasi MRTG; LinkScope tidak memakainya |
| `ConversionCode`, `LibAdd`, `SendToGraphite`, `RRDCached` | | Diterima dengan peringatan lalu diabaikan (memerlukan Perl atau RRDtool) |

### Kata kunci global baru

| Kata kunci | Default | Arti |
|---|---|---|
| `WebListen` | tidak ada | **Baru.** Alamat dan port untuk antarmuka web, API JSON, dan `/metrics`, misalnya `127.0.0.1:8080` atau `[::1]:8080`. `--listen` mengesampingkannya. Tanpa keduanya, tidak ada listener web. |
| `ExecShell` | no | **Baru.** `yes` mengizinkan perintah eksternal berjalan melalui `/bin/sh`. |
| `AlertWebhook` | tidak ada | **Baru.** URL `http://` atau `https://` yang menerima POST bertanda tangan saat peringatan aktif atau pulih. Memerlukan `AlertWebhookSecret`. |
| `AlertWebhookSecret` | tidak ada | **Baru.** Kunci penanda tangan, minimal 16 karakter. Memerlukan `AlertWebhook`. |
| `AlertMailUser` | tidak ada | **Baru.** Login ke `ThreshMailServer` melalui STARTTLS dengan user ini. Memerlukan `AlertMailPassword`. |
| `AlertMailPassword` | tidak ada | **Baru.** Kata sandi untuk `AlertMailUser`. |

[Peringatan](alerts.md) menjelaskan cara kerjanya. URL webhook, rahasianya, dan kata sandi mail tidak
pernah muncul di antarmuka web, API, `/metrics`, atau pesan kesalahan.

## Kata kunci per target

### Data dan polling

| Kata kunci | Arti |
|---|---|
| `Target[name]` | Sumber data (wajib) |
| `MaxBytes[name]` | Nilai valid tertinggi per detik (wajib, atau `MaxBytes1` dan `MaxBytes2`) |
| `MaxBytes1[name]`, `MaxBytes2[name]` | Nilai maksimum terpisah untuk input dan output |
| `AbsMax[name]` | Nilai di atas `MaxBytes` hingga nilai ini tetap disimpan; tidak boleh lebih kecil dari `MaxBytes` |
| `Options[name]` | Flag, lihat di bawah |
| `SnmpOptions[name]` | Pengaturan SNMP untuk sumber target ini |
| `NoHC[name]` | `yes`: memakai penghitung 32-bit |
| `IPv4Only[name]` | `yes`: tanpa IPv6 untuk target ini |
| `RouterUptime[name]` | Baca uptime dan nama dari agen lain, `community@host` |
| `RouterName[name]` | Baca nama dari OID lain |

### Pencatatan log

| Kata kunci | Arti |
|---|---|
| `Directory[name]` | Subdirektori `WorkDir` untuk log target ini. Tetap berada di dalam `WorkDir`: `/` di depan dibuang dan `..` adalah kesalahan. |

### Tampilan

Kata kunci ini membentuk tampilan target di [Antarmuka web](web-ui.md):

| Kata kunci | Arti |
|---|---|
| `Title[name]` | Judul (wajib) |
| `PageTop[name]`, `PageFoot[name]` | Teks di atas dan di bawah grafik; HTML disanitasi |
| `YLegend[name]`, `ShortLegend[name]` | Legenda sumbu dan satuan |
| `Legend1[name]` sampai `Legend5[name]`, `LegendI[name]`, `LegendO[name]` | Teks legenda |
| `kMG[name]` | Prefiks satuan, misalnya `k,M,G,T` |
| `Kilo[name]` | 1000 (default) atau 1024 |
| `Factor[name]` | Kalikan nilai yang ditampilkan dengan angka ini |
| `Unscaled[name]`, `WithPeak[name]`, `Suppress[name]` | Huruf `d w m y` (hari, minggu, bulan, tahun): grafik tanpa penskalaan, dengan puncak, atau tidak ditampilkan |
| `Colours[name]` | `Name#rrggbb` untuk in, out, max in, dan max out |

Kata kunci halaman dan grafik MRTG lainnya (`XSize`, `YSize`, `Background`, `PNGTitle`,
`TimeStrPos`, `AddHead`, `BodyTag`, dan seterusnya) diterima dan diperiksa seperti di MRTG.
LinkScope tidak menulis berkas HTML atau PNG, jadi kata kunci tersebut tidak mengubah antarmuka web.

### Options

`Options[name]` menerima daftar yang dipisahkan koma. Yang mengubah data:

| Opsi | Arti |
|---|---|
| `gauge` | Nilai berupa level, bukan penghitung |
| `absolute` | Nilai berupa jumlah sejak polling terakhir |
| `perminute`, `perhour` | Tampilkan laju per menit atau per jam |
| `unknaszero` | Catat nilai yang tidak diketahui sebagai nol |
| `bits` | Tampilkan bit, bukan byte |

Opsi lainnya (`growright`, `logscale`, `nopercent`, `noi`, `noo`, dan sisa daftar MRTG) diterima.
`derive` memerlukan RRDtool dan merupakan kesalahan. Opsi yang tidak dikenal adalah kesalahan.

### Ambang batas

| Kata kunci | Arti |
|---|---|
| `ThreshMinI[name]`, `ThreshMaxI[name]`, `ThreshMinO[name]`, `ThreshMaxO[name]` | Batas untuk input dan output, berupa angka atau persentase dari `MaxBytes` (`80%`) |
| `ThreshDesc[name]` | Teks yang diberikan ke program ambang batas |
| `ThreshProgI[name]`, `ThreshProgO[name]` | Program yang dijalankan saat batas dilanggar |
| `ThreshProgOKI[name]`, `ThreshProgOKO[name]` | Program yang dijalankan saat kembali dalam batas |
| `ThreshMailAddress[name]` | Tujuan email ambang batas (memerlukan `ThreshMailServer`) |
| `SetEnv[name]` | Variabel lingkungan untuk program ambang batas, `NAME="value"` |
| `ThreshFor[name]` | **Baru.** Jumlah polling sebuah batas harus tetap dilanggar sebelum peringatan aktif |

Kata kunci `HWThresh*` memerlukan data Holt-Winters dari RRDtool; kata kunci ini diterima dengan
peringatan lalu diabaikan. [Peringatan](alerts.md) menjelaskan kapan ambang batas aktif dan pulih.

## Memeriksa konfigurasi

`linkscope --check /etc/mrtg/mrtg.cfg` membaca dan memeriksa berkas tanpa melakukan polling.
Perintah ini tidak mengambil kunci (lock), jadi dapat dipakai saat daemon berjalan. Lihat
[Baris perintah](command-line.md).
