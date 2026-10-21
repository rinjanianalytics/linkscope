# Metrik Prometheus

LinkScope melayani `GET /metrics` pada listener web (`WebListen:` atau `--listen`) dalam format
teks Prometheus (versi 0.0.4). Isinya laju terkini setiap target, counter seperti terakhir dibaca,
`MaxBytes`, status dan waktu poll terakhir, serta hasil siklus polling terakhir.

## Mengambil metrik

```yaml
scrape_configs:
  - job_name: linkscope
    static_configs:
      - targets: ['127.0.0.1:8080']
```

Ambil metrik kira-kira sesuai interval polling atau lebih sering. Nilainya berubah sekali per
siklus (`Interval`, lima menit secara default). Selama konfigurasi belum dimuat, `/metrics`
menjawab 503.

## Label

Setiap seri per target membawa label berikut:

| Label | Arti |
|---|---|
| `target` | Nama target, seperti pada `Target[name]`. |
| `directory` | `Directory[]`-nya, tanpa garis miring di akhir (kosong bila tidak ada). |
| `source` | `snmp` untuk satu sumber SNMP, `exec` untuk program eksternal, `expression` untuk aritmetika atas beberapa sumber. |
| `host` | Host SNMP, untuk target `snmp`. Kosong untuk yang lain. |
| `interface` | Pemilih interface seperti ditulis, dalam bentuk `method:key`, misalnya `Name:ether1`, `Descr:uplink`, atau `If:3`. Kosong bila target langsung menyebut OID atau indeks. |
| `ifindex` | ifIndex hasil pemilih pada poll terakhir. Kosong sampai saat itu. |
| `direction` | `in` atau `out`, pada seri yang memiliki arah. |

Label tidak pernah berisi community, kata sandi SNMPv3, atau baris perintah. Setiap nilai label
melewati penyamaran yang sama dengan [API JSON](api.md).

## Metrik

| Metrik | Tipe | Label | Arti |
|---|---|---|---|
| `linkscope_info` | gauge | `version` | Selalu 1. Label `version` adalah versi LinkScope. |
| `linkscope_start_time_seconds` | gauge | tidak ada | Kapan daemon dimulai, waktu Unix. |
| `linkscope_interval_seconds` | gauge | tidak ada | Detik antar siklus polling (`Interval`). |
| `linkscope_cycles_total` | counter | tidak ada | Siklus polling yang selesai sejak daemon dimulai. |
| `linkscope_last_cycle_start_time_seconds` | gauge | tidak ada | Kapan siklus terakhir dimulai, waktu Unix. |
| `linkscope_last_cycle_duration_seconds` | gauge | tidak ada | Lama siklus terakhir, dalam detik. |
| `linkscope_last_cycle_targets` | gauge | `outcome` (`ok`, `failed`) | Target pada siklus terakhir, menurut hasilnya. |
| `linkscope_last_cycle_log_write_failures` | gauge | tidak ada | Log yang tidak dapat dibaca atau ditulis pada siklus terakhir. |
| `linkscope_target_status` | gauge | label target, `status` | 1 untuk status target saat ini (`ok`, `warn`, `error`, `stale`, `pending`), 0 untuk empat lainnya. |
| `linkscope_target_last_poll_time_seconds` | gauge | label target | Kapan sampel terakhir target diambil, waktu Unix. |
| `linkscope_target_rate` | gauge | label target, `direction` | Laju terbaru per detik, seperti ditampilkan antarmuka sebelum diformat. Byte per detik untuk counter interface. |
| `linkscope_target_max` | gauge | label target, `direction` | `MaxBytes` untuk setiap arah, per detik dalam satuan target. |
| `linkscope_target_counter_total` | counter | label target, `direction` | Counter seperti terakhir dibaca: oktet untuk target interface. Tidak ada untuk target dengan `gauge` atau `absolute`. |
| `linkscope_target_value` | gauge | label target, `direction` | Nilai seperti terakhir dibaca, untuk target dengan `Options[]: gauge` atau `absolute`. |

Empat seri `linkscope_last_cycle_*` muncul setelah siklus pertama. Laju, counter, dan poll
terakhir sebuah target muncul setelah poll pertamanya.

Laju selalu per detik, bahkan untuk target dengan `perminute` atau `perhour`: LinkScope membaginya
dengan 60 atau 3600 untuk Anda. Opsi `bits` hanya mengubah cara antarmuka mencetak laju;
`linkscope_target_rate` tetap dalam byte, jadi kalikan 8 untuk bit per detik.

## Contoh kueri

Trafik masuk satu target dalam bit per detik, dari laju LinkScope sendiri:

```promql
linkscope_target_rate{target="core-uplink", direction="in"} * 8
```

Hal yang sama dari counter, dirata-rata selama sepuluh menit (jendela harus mencakup setidaknya
dua poll):

```promql
rate(linkscope_target_counter_total{target="core-uplink", direction="in"}[10m]) * 8
```

Utilisasi dalam persen terhadap `MaxBytes`, untuk setiap target dan arah (kedua metrik membawa
label yang sama, sehingga cocok satu lawan satu):

```promql
100 * linkscope_target_rate / linkscope_target_max
```

Target yang tidak `ok`:

```promql
linkscope_target_status{status!="ok"} == 1
```

Target yang tidak di-poll dalam 15 menit terakhir:

```promql
time() - linkscope_target_last_poll_time_seconds > 900
```

## Counter 32-bit

`cfgmaker` menulis target SNMPv1 kecuali Anda meminta versi lain, sama seperti milik MRTG. Counter
SNMPv1 berukuran 32 bit: pada 1 Gbit/s counter ini berputar ulang setiap 34 detik, sehingga poll
lima menit tidak dapat mengetahui berapa kali interface yang sibuk berputar ulang. Gunakan SNMPv2c
atau v3 untuk target interface, yang membaca counter 64-bit:

```text
Target[core-uplink]: #ether1:public@192.0.2.1:::::2
```

atau buat konfigurasi dengan `cfgmaker public@192.0.2.1:::::2`. Lihat [cfgmaker](cfgmaker.md).
