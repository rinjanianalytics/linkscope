# API JSON

Antarmuka web membaca semua yang ditampilkannya dari API JSON kecil pada listener yang sama. Anda
juga dapat membacanya, untuk skrip, dasbor, atau alat lain. API ini hanya untuk dibaca: tidak ada
yang mengubah LinkScope atau konfigurasinya.

| Metode dan path | Hasil |
|---|---|
| `GET /api/summary` | Status daemon dan jumlah target |
| `GET /api/targets` | Setiap target dengan status terkininya |
| `GET /api/targets/{name}` | Detail dan pengaturan tampilan satu target |
| `GET /api/targets/{name}/series` | Titik grafik satu periode serta Max, Average, dan Current dari MRTG |
| `GET /healthz` | `ok` sebagai teks biasa selama server berjalan |

Laju dinyatakan dalam satuan log: byte per detik, atau per menit dengan `Options[]: perminute`,
atau per jam dengan `perhour`. `format` setiap target menjelaskan cara MRTG mencetaknya. Untuk laju
yang selalu per detik, gunakan [`/metrics`](metrics.md).

## Galat

Galat berupa status HTTP dengan pesan teks biasa:

| Status | Kapan |
|---|---|
| 400 | `period` yang tidak dikenal untuk sebuah seri. |
| 404 | Tidak ada target dengan nama itu, atau belum ada data yang dikumpulkan untuk sebuah seri. |
| 503 | Konfigurasi belum dimuat (sesaat setelah mulai). |
| 500 | Log target tidak dapat dibaca. |

## Rahasia

API tidak pernah mengembalikan rahasia. Community SNMP di `Target[]` disamarkan
(`2:•••@192.0.2.1`), dan kata sandi serta kunci SNMPv3, baris perintah eksternal, URL webhook,
rahasianya, dan kata sandi email diganti dengan `•••` di mana pun muncul, termasuk di pesan galat.
Path program ambang dan alamat email ambang juga tidak diberikan, hanya apakah keduanya diatur.
HTML dari konfigurasi (`Title[]`, `PageTop[]`, `PageFoot[]`, legenda) dibersihkan menjadi subset
yang aman: struktur dan format teks, serta tautan hanya ke `http` dan `https`.

## GET /api/summary

```json
{
  "version": "0.2.0",
  "started": 1760940000,
  "now": 1760943600,
  "cycles": 12,
  "last_cycle": { "started": 1760943300, "duration_ms": 412, "targets": 24, "ok": 23, "failed": 1, "write_failures": 0 },
  "interval_secs": 300,
  "config_name": "mrtg.cfg",
  "warnings": [],
  "targets_total": 24,
  "targets_ok": 22,
  "targets_error": 2,
  "targets_warn": 1,
  "targets_failed": 1,
  "targets_stale": 0,
  "targets_pending": 0
}
```

| Field | Tipe | Arti |
|---|---|---|
| `version` | string | Versi LinkScope. |
| `started` | number | Kapan daemon dimulai, waktu Unix. |
| `now` | number | Waktu server, waktu Unix. |
| `cycles` | number | Siklus polling yang selesai. |
| `last_cycle` | object atau null | Siklus terakhir: `started`, `duration_ms`, `targets`, `ok`, `failed`, `write_failures` (log yang tidak terbaca atau tertulis). Null sebelum siklus pertama. |
| `interval_secs` | number | Detik antar siklus. |
| `config_name` | string | Nama berkas konfigurasi (bukan path-nya, yang tidak ditampilkan antarmuka). |
| `warnings` | array of string | Peringatan saat memuat konfigurasi. |
| `targets_total` | number | Jumlah target yang dikonfigurasi. |
| `targets_ok` | number | Target dengan status `ok`. |
| `targets_warn`, `targets_failed`, `targets_stale`, `targets_pending` | number | Target dengan status `warn`, `error`, `stale`, dan `pending`. |
| `targets_error` | number | `warn`, `error`, dan `stale` sekaligus (dipertahankan untuk klien lama). |

## GET /api/targets

Array dengan satu objek per target, sesuai urutan konfigurasi:

```json
[
  {
    "name": "core-uplink",
    "title_html": "Core uplink",
    "directory": "core",
    "status": "ok",
    "error": null,
    "last_poll": 1760943300,
    "sys_name": "edge1.example.com",
    "uptime": "41 days, 3:12:08",
    "max_bytes1": 125000000,
    "max_bytes2": 125000000,
    "format": { "mul": 8.0, "factor": 1.0, "kilo": 1000.0, "units": ["b/s", "kb/s", "Mb/s", "Gb/s"], "integer": false, "nopercent": false },
    "legend_i": "In",
    "legend_o": "Out",
    "colours": [{ "name": "GREEN", "rgb": "#00cc00" }, { "name": "BLUE", "rgb": "#0000ff" }],
    "no_i": false,
    "no_o": false,
    "growright": true,
    "logscale": false,
    "cur_in": 6514625.0,
    "cur_out": 1203750.0,
    "cur_in_text": "52.1 Mb/s (5.2%)",
    "cur_out_text": "9.6 Mb/s (1.0%)",
    "alerts": []
  }
]
```

| Field | Tipe | Arti |
|---|---|---|
| `name` | string | Nama target. |
| `title_html` | string | `Title[]`, HTML yang sudah dibersihkan. |
| `directory` | string | `Directory[]`, tanpa garis miring di akhir. |
| `status` | string | `ok`, `warn`, `error`, `stale`, atau `pending`. Lihat [Antarmuka web](web-ui.md#status). |
| `error` | string atau null | Alasan data hilang pada poll terakhir. |
| `last_poll` | number atau null | Waktu Unix sampel terakhir. |
| `sys_name`, `uptime` | string atau null | Nama dan uptime perangkat seperti yang dilaporkannya. |
| `max_bytes1`, `max_bytes2` | number | `MaxBytes` untuk masuk dan keluar. |
| `format` | object | Cara MRTG mencetak laju: `mul` (8 dengan `bits`), `factor` (`Factor[]`), `kilo` (`Kilo[]`), `units` (label per pangkat `kilo`), `integer`, `nopercent`. |
| `legend_i`, `legend_o` | string | `LegendI[]` dan `LegendO[]`, HTML yang sudah dibersihkan. |
| `colours` | array | `Colours[]`: `name` dan `rgb`. |
| `no_i`, `no_o`, `growright`, `logscale` | boolean | `Options[]` yang sesuai. |
| `cur_in`, `cur_out` | number atau null | Laju terbaru, dalam satuan log. |
| `cur_in_text`, `cur_out_text` | string atau null | Sama, diformat seperti cetakan MRTG. |
| `alerts` | array | Batas ambang yang terlampaui saat ini, aktif atau menunggu. |

Setiap item `alerts`:

| Field | Tipe | Arti |
|---|---|---|
| `direction` | string | `in` atau `out`. |
| `bound` | string | `min` atau `max`. |
| `threshold` | string | Batas seperti dikonfigurasi, misalnya `80%`. |
| `threshold_rate`, `rate` | number | Batas dan laju saat ini, dalam satuan log. |
| `percent` | number atau null | Laju dalam persen terhadap `MaxBytes`, untuk batas persentase. |
| `firing` | boolean | True setelah terlampaui selama `polls_needed` poll. |
| `polls`, `polls_needed` | number | Poll terlampaui berturut-turut, dan nilai `ThreshFor[]` saat peringatan aktif. |
| `since` | number | Waktu Unix poll pertama yang terlampaui. |
| `fired_at` | number atau null | Waktu Unix poll yang mengaktifkannya. |

## GET /api/targets/{name}

Semua isi item `/api/targets`, ditambah pengaturan tampilan target:

| Field | Tipe | Arti |
|---|---|---|
| `target_text` | string | Teks `Target[]`, dengan community disamarkan. |
| `page_top_html`, `page_foot_html` | string | `PageTop[]` dan `PageFoot[]`, HTML yang sudah dibersihkan. |
| `y_legend` | string | `YLegend[]`. |
| `legends` | array berisi 5 string | `Legend1[]` sampai `Legend5[]`, HTML yang sudah dibersihkan. |
| `periods` | array of string | Grafik yang ditampilkan: `day`, `week`, `month`, `year`, dikurangi `Suppress[]`. |
| `with_peak`, `unscaled` | array of string | Periode yang disebut di `WithPeak[]` dan `Unscaled[]`. |
| `growright`, `logscale`, `expscale`, `secondmean`, `dorelpercent`, `avgpeak`, `noinfo`, `nolegend` | boolean | `Options[]` yang sesuai. |
| `xsize`, `ysize`, `ytics`, `yticsfactor` | number | `XSize[]`, `YSize[]`, `YTics[]`, `YTicsFactor[]`. |
| `abs_max` | number | `AbsMax[]`. |
| `timezone` | string atau null | `Timezone[]`. |
| `thresholds` | object | Pengaturan ambang target: `min_i`, `min_o`, `max_i`, `max_o`, `desc`, dan `for_polls` (`ThreshFor[]`), bernilai null bila tidak diatur; `programs` (boolean: ada `ThreshProg*[]` yang diatur) dan `mail` (boolean: `ThreshMailAddress[]` diatur). Program mana dan alamat mana tidak diberikan. |

## GET /api/targets/{name}/series

Parameter kueri:

| Parameter | Default | Arti |
|---|---|---|
| `period` | `day` | `day`, `week`, `month`, atau `year`. |
| `maxx` | `XSize[]` target | Jumlah titik yang diminta, dari 8 sampai 2000. |

```json
{
  "period": "day",
  "step": 300,
  "now": 1760943300,
  "points": [
    { "t": 1760857200, "in": 6013500.0, "out": 1100250.0, "in_max": 6201000.0, "out_max": 1180500.0 }
  ],
  "peak": false,
  "fixed_max": null,
  "stats": {
    "max_in": 9120000, "max_out": 2040000, "avg_in": 5210000, "avg_out": 1150000,
    "cur_in": 6514625, "cur_out": 1203750, "avg_max_in": 0, "avg_max_out": 0,
    "max_pct": 7, "avg_pct": 4, "cur_pct": 5,
    "text": {
      "max_in": "73.0 Mb/s (7.3%)", "max_out": "16.3 Mb/s (1.6%)",
      "avg_in": "41.7 Mb/s (4.2%)", "avg_out": "9.2 Mb/s (0.9%)",
      "cur_in": "52.1 Mb/s (5.2%)", "cur_out": "9.6 Mb/s (1.0%)",
      "avg_max_in": "0.0 b/s (0.0%)", "avg_max_out": "0.0 b/s (0.0%)"
    }
  }
}
```

| Field | Tipe | Arti |
|---|---|---|
| `period` | string | Periode yang dikembalikan. |
| `step` | number | Detik per titik. |
| `now` | number | Waktu pembaruan terakhir log, waktu Unix. |
| `points` | array | Terlama lebih dulu: `t` (waktu Unix), `in`, `out`, dan puncak `in_max`, `out_max`, dalam satuan log. |
| `peak` | boolean | True bila `WithPeak[]` mencakup periode ini. |
| `fixed_max` | number atau null | Maksimum sumbu y yang tetap bila `Unscaled[]` mencakup periode ini. |
| `stats` | object | Max, Average, Current, dan Average max dari MRTG untuk masuk dan keluar (satuan log), persentase terhadap `MaxBytes` (`max_pct`, `avg_pct`, `cur_pct`, masing-masing number atau null), dan `text`, angka yang sama diformat seperti cetakan MRTG. |

## Contoh

```sh
curl -s http://127.0.0.1:8080/api/targets \
  | jq -r '.[] | select(.status != "ok") | "\(.name) \(.status) \(.error // "")"'
```
