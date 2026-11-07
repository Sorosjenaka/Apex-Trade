# Migrasi Rute WebSocket Binance USDⓈ-M Futures (`/public`, `/market`, `/private`)

> Dokumen rujukan resmi untuk migrasi endpoint WebSocket di `Apex-Trade` (WP-1).
> Seluruh informasi di bawah ini dirujuk dari dokumentasi resmi `developers.binance.com` untuk **USDⓈ-M Futures**.

---

## 1. Tanggal Efektif & Latar Belakang

Berdasarkan pengumuman resmi ***Important WebSocket Change Notice*** pada dokumentasi Binance Derivatives (USDⓈ-M Futures):

- **Tanggal efektif pemutusan rute lama:** **23 April 2026 (`2026-04-23`)**.
- Endpoint lama tanpa awalan kategori (`wss://fstream.binance.com/ws/...` dan `wss://fstream.binance.com/stream?streams=...`) telah dipensiunkan dan digantikan arsitektur tiga rute terpisah berdasarkan karakteristik beban dan otorisasi:
  1. `/public` — buku order frekuensi tinggi & *raw trade*
  2. `/market` — data pasar agregat, *kline*, harga wajar (*mark price*), ticker, dan likuidasi
  3. `/private` — *User Data Streams* milik akun (order, posisi, saldo, algo)
- Koneksi ke path lama `/ws` atau `/stream` tanpa kategori dapat menyelesaikan jabat tangan (*handshake*) atau mengembalikan ACK `SUBSCRIBE`, tetapi tidak mengalirkan *frame* data untuk stream yang telah dipindah ke `/market` atau `/public`.

**Sumber resmi:**
- *Important WebSocket Change Notice / WebSocket Market Streams*: https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams
- *Connect — WebSocket Market Streams*: https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Connect
- *Live Subscribing/Unsubscribing to streams*: https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Live-Subscribing-Unsubscribing-to-streams

---

## 2. Tabel Pemetaan Jenis Stream ke Rute Baru

Setiap koneksi WebSocket hanya boleh memuat stream yang berasal dari **satu rute yang sama**. Menggabungkan stream lintas rute (misalnya `@bookTicker` dan `@kline_15m`) pada satu koneksi tidak didukung dan wajib dipisah menjadi satu koneksi per rute.

| Jenis Stream | Pola Nama Stream | Rute Baru | Dipakai di Repo |
|---|---|---|---|
| Partial Book Depth | `<symbol>@depth<levels>` (`5`, `10`, `20`), `@100ms` / `@250ms` / `@500ms` | `/public` | `dashboard/micro_live.py` (`@depth20@100ms`) |
| Diff. Book Depth | `<symbol>@depth`, `<symbol>@depth@100ms` / `@500ms` | `/public` | `dashboard/wsclient.py` (dinamis) |
| RPI Diff. Book Depth | `<symbol>@rpiDepth` | `/public` | Tidak |
| Individual Symbol Book Ticker | `<symbol>@bookTicker` | `/public` | `dashboard/wsclient.py` (dinamis) |
| All Book Tickers | `!bookTicker` | `/public` | Tidak |
| Raw Trade Stream | `<symbol>@trade` | `/public` | Tidak |
| Kline / Candlestick | `<symbol>@kline_<interval>` (`1m`, `5m`, `15m`, `1h`, dst.) | `/market` | `crates/data/src/ws.rs`, `apps/apex/src/live.rs`, `dashboard` |
| Continuous Contract Kline | `<pair>_<contractType>@continuousKline_<interval>` | `/market` | Tidak |
| Aggregate Trade | `<symbol>@aggTrade` | `/market` | `dashboard/server.py`, `dashboard/wsclient.py` |
| Mark Price | `<symbol>@markPrice`, `<symbol>@markPrice@1s`, `!markPrice@arr`, `!markPrice@arr@1s` | `/market` | `dashboard/wsclient.py` |
| Individual / All Mini Ticker | `<symbol>@miniTicker`, `!miniTicker@arr` | `/market` | Tidak |
| Individual / All Rolling Ticker | `<symbol>@ticker`, `!ticker@arr` | `/market` | Tidak |
| Liquidation Order | `<symbol>@forceOrder`, `!forceOrder@arr` | `/market` | `crates/data/src/ws.rs` (MR-06) |
| Composite / Contract / Asset Index | `<symbol>@compositeIndex`, `!contractInfo`, `!assetIndex@arr` | `/market` | Tidak |
| User Data Stream | `ORDER_TRADE_UPDATE`, `ACCOUNT_UPDATE`, `ALGO_UPDATE`, `listenKeyExpired`, dst. | `/private` | `crates/oms/src/userstream.rs` |

> **Catatan pembeda penting:** `@trade` (*raw trade*) berada di `/public`, sedangkan `@aggTrade` (*aggregate trade*) berada di `/market`. Di kode Rust maupun Python, pencocokan akhiran `@trade` wajib eksak agar tidak salah menangkap `@aggTrade`.

---

## 3. Format URL Koneksi Baru

### 3.1 Stream Publik & Pasar (`/public` dan `/market`)

- **Raw stream tunggal:**
  `wss://<root>/<route>/ws/<streamName>`
  Contoh: `wss://fstream.binance.com/market/ws/btcusdt@kline_15m`
- **Combined streams (dipakai oleh `Apex-Trade`):**
  `wss://<root>/<route>/stream?streams=<stream1>/<stream2>/...`
  Contoh `/market`:
  `wss://fstream.binance.com/market/stream?streams=btcusdt@kline_15m/btcusdt@kline_1h/ethusdt@kline_15m`
  Contoh `/public`:
  `wss://fstream.binance.com/public/stream?streams=ethusdt@depth20@100ms/solusdt@depth20@100ms`

Pada combined stream, payload dibungkus `{"stream":"<nama>","data":{...}}` (identik dengan format sebelumnya).

### 3.2 Stream Privat / User Data (`/private`)

- **Tanpa filter event:**
  `wss://<root>/private/ws/<listenKey>`
- **Dengan parameter query `listenKey` dan filter `events` (opsional namun direkomendasikan agar eksplisit):**
  `wss://<root>/private/ws?listenKey=<listenKey>&events=ORDER_TRADE_UPDATE,ACCOUNT_UPDATE,ALGO_UPDATE`

Catatan parameter `events`:
- Parameter `events` bersifat opsional untuk memilih subset event user-data yang relevan; nilai dipisahkan koma tanpa spasi (`ORDER_TRADE_UPDATE,ACCOUNT_UPDATE,ALGO_UPDATE`).
- Event kontrol siklus hidup seperti `listenKeyExpired` tetap dikirim pada koneksi user-data tersebut.

**Sumber resmi:**
- *User Data Streams — Connect*: https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams/Connect
- *Event Order Update / Algo Order Update / Balance and Position Update*: https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams

---

## 4. Host Mainnet vs Demo Trading (Pengganti Testnet Lama)

Berdasarkan halaman resmi ***General Info*** USDⓈ-M Futures:

| Lingkungan | Base URL REST | Root URL WebSocket | Status |
|---|---|---|---|
| **Mainnet (Live)** | `https://fapi.binance.com` | `wss://fstream.binance.com` | Aktif |
| **Demo Trading (Testnet baru)** | `https://demo-fapi.binance.com` | `wss://demo-fstream.binance.com` | Aktif (resmi) |
| *Legacy Futures Testnet* | *`https://testnet.binancefuture.com`* | *`wss://fstream.binancefuture.com`* | **Usang — jangan dipakai** |

Di `config/engine.toml`, `live_ws_base` dan `testnet_ws_base` hanya boleh berisi **host akar** (`wss://fstream.binance.com` dan `wss://demo-fstream.binance.com`) tanpa `/ws`, `/stream`, `/public`, `/market`, atau `/private`.

**Sumber resmi:**
- *General Info (USDⓈ-M Futures)*: https://developers.binance.com/docs/derivatives/usds-margined-futures/general-info

---

## 5. Batas Koneksi & Siklus Hidup

1. **Umur maksimum koneksi = 24 jam:** satu koneksi WebSocket hanya valid selama paling lama 24 jam dan akan diputus otomatis oleh server tepat di batas 24 jam. Klien `Apex-Trade` melakukan *proactive reconnect* pada jam ke-23 (`23 * 3600` detik) ditambah *backfill* kline tertutup via REST saat `WsEvent::Reconnected` agar tidak kehilangan bar pada pergantian koneksi.
2. **Ping / Pong:** server mengirim *ping frame* setiap **3 menit**. Jika server tidak menerima *pong frame* dalam **10 menit**, koneksi diputus. Klien wajib membalas `Message::Ping(payload)` dengan `Message::Pong(payload)` identik.
3. **Batas pesan masuk dari klien:** maksimal **10 pesan kontrol per detik** per koneksi (`PING`, `PONG`, `SUBSCRIBE`, `UNSUBSCRIBE`); melampaui batas ini mengakibatkan pemutusan koneksi (`disconnect`).
4. **Batas jumlah stream per koneksi:** maksimal **200 stream** dalam satu koneksi *combined stream*. `run_public_stream` memecah daftar stream menjadi potongan ≤ 200 stream per koneksi rute.
5. **Masa berlaku `listenKey`:** aktif selama **60 menit**; perpanjangan (*keepalive*) dilakukan setiap **30 menit** (`1800` detik) melalui REST `PUT /fapi/v1/listenKey`.

---

## 6. Apa yang TIDAK Berubah

- **Endpoint REST manajemen `listenKey` tetap sama:**
  - Buat: `POST /fapi/v1/listenKey`
  - Perpanjang (+60 menit): `PUT /fapi/v1/listenKey`
  - Tutup: `DELETE /fapi/v1/listenKey`
- **Skema JSON payload tetap sama:** struktur field `kline` (`e`, `s`, `k.t`, `k.x`, `k.o/h/l/c/v/q/n/V`), `forceOrder` (`o.s`, `o.ap`, `o.z`, `o.T`), serta event privat `ORDER_TRADE_UPDATE`, `ALGO_UPDATE`, `ACCOUNT_UPDATE`, dan `listenKeyExpired` tidak berubah.

**Sumber resmi:**
- *Start / Keepalive / Close User Data Stream (REST)*: https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams/Start-User-Data-Stream
