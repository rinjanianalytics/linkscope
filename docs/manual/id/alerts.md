# Peringatan

LinkScope memeriksa ambang seperti MRTG, dengan kata kunci, program, email, dan file status yang
sama. LinkScope menambahkan empat hal, semuanya nonaktif sampai Anda mengaturnya: batas yang harus
tetap terlampaui selama beberapa poll sebelum aktif, webhook bertanda tangan, email melalui
STARTTLS dengan login, serta status peringatan di antarmuka dan API.

## Ambang MRTG

Kata kunci berikut bekerja persis seperti di MRTG 2.17.10:

| Kata kunci | Arti |
|---|---|
| `ThreshMinI[x]`, `ThreshMinO[x]` | Terlampaui bila laju masuk (atau keluar) turun di bawah nilai ini. |
| `ThreshMaxI[x]`, `ThreshMaxO[x]` | Terlampaui bila laju masuk (atau keluar) naik di atas nilai ini. |
| `ThreshDesc[x]` | Deskripsi yang diteruskan ke program (sebagai `THRESH_DESC`) dan email. |
| `ThreshProgI[x]`, `ThreshProgO[x]` | Program yang dijalankan saat batas masuk (atau keluar) terlampaui. |
| `ThreshProgOKI[x]`, `ThreshProgOKO[x]` | Program yang dijalankan saat batas tidak lagi terlampaui. |
| `ThreshMailAddress[x]` | Kirim pemberitahuan ambang ke alamat ini. |
| `ThreshMailServer` | Server email, `host` atau `host:port`. |
| `ThreshMailSender` | Alamat pengirim. Wajib bersama `ThreshMailServer`. |
| `ThreshDir` | Direktori untuk file status, agar program dan email berjalan sekali per perubahan. |
| `ThreshHyst` | Aturan histeresis MRTG, dipakai tanpa `ThreshDir` (hanya bila diatur). |

Batas adalah laju dalam byte per detik, atau persentase terhadap `MaxBytes` bila diakhiri `%`
(`ThreshMaxI[x]: 80%`). LinkScope membandingkannya dengan laju terkini pada grafik harian, seperti
MRTG.

Program ambang dijalankan langsung, tanpa shell, dengan `SetEnv[]` target dan `THRESH_DESC` di
lingkungannya. Argumennya sama dengan MRTG: nama target, batas seperti dikonfigurasi, nilai saat
ini, dan, saat terlampaui, deskripsinya (`No Description` bila tidak ada). Program yang berjalan
lebih dari 60 detik dihentikan, bersama semua yang dijalankannya (program berjalan dalam process
group sendiri).

Programnya adalah file yang ditemukan pemeriksaan konfigurasi: path relatif diambil dari direktori
tempat LinkScope dijalankan, dan file itulah yang dijalankan. MRTG memeriksa path tersebut tetapi
menjalankan nama tanpa path melalui `PATH`, yang bisa menjalankan program lain dari yang diperiksa.
File harus dapat dieksekusi.

Rahasia yang dikonfigurasi (community SNMP, kata sandi, URL webhook) yang dikutip oleh `Title[]`,
`ThreshDesc[]`, atau `PageTop[]` diganti dengan `•••` di email, di argumen program, dan di
`THRESH_DESC`, seperti di body webhook. Teks lain, termasuk alamat email, tetap seperti yang ditulis
MRTG.

Tanpa `ThreshDir`, MRTG menjalankan program dan mengirim email pada setiap poll yang terlampaui.
Dengan `ThreshDir`, satu file status per batas (`<target>.threshmaxI` dan seterusnya) mencatat
bahwa batas terlampaui, sehingga program dan email berjalan sekali saat terlampaui dan sekali saat
pulih. LinkScope mempertahankan kedua perilaku ini.

File status yang tidak dapat dipakai (`ThreshDir` hilang atau tidak dapat ditulis, atau ada hal lain
di tempatnya: direktori, link, file milik akun lain) dicatat sebagai galat di log. LinkScope lalu
mengingat perubahan itu di memori, sehingga program dan email tetap berjalan sekali per perubahan,
bukan pada setiap poll.

## Tambahan dari LinkScope

```text
ThreshMaxI[core-uplink]: 80%
ThreshFor[core-uplink]: 3

AlertWebhook: https://alerts.example.com/linkscope
AlertWebhookSecret: 3f7c9a1e5b2d4c6f8a0e

ThreshMailServer: smtp.example.com:587
ThreshMailSender: linkscope@example.com
AlertMailUser: linkscope@example.com
AlertMailPassword: app-password
```

| Kata kunci | Arti |
|---|---|
| `ThreshFor[x]: N` | Batas aktif setelah N poll berturut-turut terlampaui, dan selesai pada poll pertama yang tidak terlampaui. |
| `AlertWebhook` | Kirim setiap peringatan yang aktif atau selesai ke URL ini. Memerlukan `AlertWebhookSecret`. |
| `AlertWebhookSecret` | Kunci untuk menandatangani setiap kiriman. Minimal 16 karakter. |
| `AlertMailUser`, `AlertMailPassword` | Login ke `ThreshMailServer` melalui STARTTLS. Keduanya atau tidak sama sekali. |

### ThreshFor

Tanpa `ThreshFor[]`, batas aktif pada poll pertama yang terlampaui, dan program serta email target
berjalan persis seperti MRTG menjalankannya.

Dengan `ThreshFor[x]: N`, batas harus tetap terlampaui selama N poll berturut-turut sebelum aktif.
Satu poll yang kembali dalam batas memulai hitungan dari awal. `ThreshProg*` dan
`ThreshMailAddress[]` target kemudian berjalan sekali saat peringatan aktif dan sekali saat
selesai, dengan argumen dan format pesan MRTG.

Poll tanpa laju untuk suatu arah (tidak ada jawaban dari perangkat) tidak mengubah apa pun: data
yang hilang tidak mengaktifkan maupun menyelesaikan peringatan.

### Email dengan login

Tanpa `AlertMailUser`, email dikirim seperti MRTG mengirimnya: SMTP biasa, ke port 25 kecuali
`ThreshMailServer` menyebut port. Ke server selain host ini, LinkScope memberi peringatan saat mulai
bahwa email dikirim tanpa TLS.

Dengan `AlertMailUser` dan `AlertMailPassword`, LinkScope terhubung ke port 587 kecuali
`ThreshMailServer` menyebut port lain, mewajibkan STARTTLS, memeriksa sertifikat server, lalu
login. Bila STARTTLS gagal, email tidak dikirim. `AlertMailUser` memerlukan `ThreshMailServer`.

Subjek dan isi mengikuti MRTG: subjek diawali `[MRTG-TH] BROKEN` saat batas terlampaui dan
`[MRTG-TH] UN-BROKEN` saat pulih, diikuti target, batas, dan nilainya.

### Sertifikat

Email dan webhook memercayai otoritas sertifikat sistem. Untuk memakai milik Anda sendiri, atur
`SSL_CERT_FILE` ke file PEM di lingkungan LinkScope. File ini menggantikan otoritas sistem.

## Webhook

Dengan `AlertWebhook` diatur, LinkScope mengirim satu body JSON untuk setiap peringatan yang aktif
atau selesai, pada setiap target yang memiliki ambang. Target tanpa `ThreshFor[]` aktif pada poll
pertama yang terlampaui.

Setiap permintaan membawa header berikut:

| Header | Nilai |
|---|---|
| `Content-Type` | `application/json` |
| `User-Agent` | `LinkScope/<version>` |
| `X-LinkScope-Event` | `threshold` |
| `X-LinkScope-Delivery` | Id pemberitahuan ini, sama pada setiap percobaan |
| `X-LinkScope-Timestamp` | Waktu Unix percobaan ini |
| `X-LinkScope-Signature` | `sha256=` dan HMAC-SHA256 heksadesimal dari timestamp, sebuah titik, dan body |

Body-nya:

```json
{
  "event": "threshold", "state": "firing", "reason": null,
  "target": "core-uplink", "title": "Core uplink", "directory": "core",
  "direction": "in", "bound": "max", "threshold": "80%",
  "threshold_rate": 100000.0, "rate": 107750.0, "percent": 86.2,
  "polls": 3, "since": 1760940000, "time": 1760940600,
  "description": null, "linkscope": "0.3.0"
}
```

| Field | Tipe | Arti |
|---|---|---|
| `event` | string | Selalu `threshold`. |
| `state` | string | `firing` atau `resolved`. |
| `reason` | string atau null | Untuk `resolved`: `cleared` (kembali dalam batas) atau `removed` (batas dihapus dari konfigurasi). Null saat aktif. |
| `target` | string | Nama target. |
| `title` | string | `Title[]` sebagai teks biasa. |
| `directory` | string | `Directory[]`, tanpa garis miring di akhir. |
| `direction` | string | `in` atau `out`. |
| `bound` | string | `min` atau `max`. |
| `threshold` | string | Batas seperti dikonfigurasi, misalnya `80%` atau `37500`. |
| `threshold_rate` | number | Batas sebagai laju per detik. |
| `rate` | number | Laju saat ini per detik (byte untuk counter interface). |
| `percent` | number atau null | Laju dalam persen terhadap `MaxBytes`, untuk batas persentase. |
| `polls` | number | Poll terlampaui berturut-turut: saat aktif, atau sampai poll yang menyelesaikannya. |
| `since` | number | Waktu Unix poll pertama yang terlampaui. |
| `time` | number | Waktu Unix poll ini. |
| `description` | string atau null | `ThreshDesc[]`. |
| `linkscope` | string | Versi LinkScope. |

### Percobaan ulang

LinkScope menunggu jawaban hingga 10 detik (5 detik untuk terhubung). LinkScope mencoba tiga kali
(segera, setelah 2 detik, setelah 10 detik) bila terjadi galat jaringan, 5xx, atau 429. Jawaban
lain bersifat final. LinkScope tidak pernah mengikuti redirect, sehingga body bertanda tangan tidak
dapat terkirim ke tempat yang tidak Anda konfigurasi, dan mengabaikan pengaturan proxy di
lingkungannya (`HTTP_PROXY`, `ALL_PROXY`). Kegagalan dicatat di log tanpa URL.

Pengiriman bersifat paling sedikit sekali: percobaan yang habis waktunya mungkin tetap sampai, dan
percobaan berikutnya menjadi salinan kedua. Setiap percobaan dari satu pemberitahuan membawa
`X-LinkScope-Delivery` yang sama, sehingga penerima membuang id pengiriman yang sudah pernah
diterimanya.

URL `http://` ke host lain diizinkan, tetapi LinkScope memberi peringatan saat mulai: body, dan token
apa pun di URL, dikirim tanpa enkripsi. Gunakan `https://`.

### Memeriksa tanda tangan

Hitung HMAC-SHA256 dari header timestamp, sebuah titik, dan body mentah, dengan kunci
`AlertWebhookSecret`, lalu bandingkan dengan header tanda tangan. Tolak timestamp yang sudah lama
untuk mencegah pengiriman ulang:

```python
import hashlib, hmac, time

def verify(secret: bytes, timestamp: str, body: bytes, signature: str) -> bool:
    """True for a signed, recent request; False for anything else, malformed headers included."""
    try:
        sent = int(timestamp)
        want = hmac.new(secret, timestamp.encode() + b"." + body, hashlib.sha256).hexdigest()
        ok = hmac.compare_digest(("sha256=" + want).encode(), signature.encode())
    except (TypeError, ValueError, AttributeError, UnicodeError):
        return False
    return ok and abs(time.time() - sent) < 300
```

URL webhook, rahasianya, dan kata sandi email tidak pernah muncul di antarmuka, API, `/metrics`,
maupun log.

## Status peringatan

LinkScope menyimpan peringatan dalam file status di samping cache `.ok`: `mrtg.alerts` untuk
`mrtg.cfg`, atau path `--confcache-file` dengan ekstensi diganti menjadi `.alerts`. Memuat ulang konfigurasi atau memulai
ulang tidak mengulang maupun menghilangkan pemberitahuan. Batas yang dihapus dari konfigurasi saat
aktif mengirim satu pemberitahuan `resolved` dengan alasan `removed`.

## Di antarmuka dan API

- Target dengan peringatan aktif berstatus `warn`, dan kartunya menampilkan batas yang terlampaui,
  misalnya `In above 80%, now 86.2%`.
- Batas yang terlampaui kurang dari jumlah poll `ThreshFor[]` ditampilkan sebagai menunggu,
  misalnya `1 of 3 polls before it fires`, dan target tetap `ok`.
- [API JSON](api.md) mencantumkan batas yang terlampaui untuk setiap target pada field `alerts`.
