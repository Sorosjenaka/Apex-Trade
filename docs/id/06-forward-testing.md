# 06 — Forward Testing: Demo dan Mainnet Shadow

Dokumen ini menjelaskan protokol forward testing sesuai `docs/plan/Rules.md` **GR-9** (baris 17, 86, 108) dan bagaimana subcommand `apex forward-gate` mengukur kesiapan operasional sebelum pertimbangan `ARM` mainnet.

---

## 1. Urutan Wajib (`Rules.md` GR-9)

1. **Replay & Validasi OOS (`apex validate`)**: strategi harus berstatus `GO` pada `config/strategies.toml` setelah lulus gerbang G-01..G-11. *(Saat ini seluruh strategi masih berstatus `NO-GO`.)*
2. **Forward Demo (`--stage demo`, ≥ 14 hari dan ≥ 30 trade)**: dijalankan pada endpoint Binance Futures Demo (`https://demo-fapi.binance.com` dan `wss://demo-fstream.binance.com`).
3. **Mainnet Shadow (`--stage shadow`, ≥ 7 hari)**: dijalankan pada mode `paper` memakai aliran data publik mainnet (`https://fapi.binance.com` dan `wss://fstream.binance.com`) tanpa mengirim order.
4. **Live Ukuran Terkecil ($1 risiko neto per trade)**: hanya setelah manusia memverifikasi checklist `docs/id/05-go-live-checklist.md`.

---

## 2. Kejujuran Metodologis: Apa yang Diukur dan TIDAK Diukur Tiap Tahap

### 2.1 Mainnet Shadow (`--stage shadow`)
- **Apa yang diukur:**
  - Kesinambungan koneksi WebSocket rute baru (`/market`, `/public`) pada beban data mainnet sesungguhnya.
  - Mekanisme deteksi celah bar dan backfill REST otomatis saat reconnect / rotasi 23 jam.
  - Paritas perhitungan fitur screener, rezim pasar, gate risiko, serta ketiadaan `RULE_VIOLATION` dan crash proses (`uptime`).
- **Apa yang TIDAK diukur:**
  - **Slippage eksekusi, antrean limit post-only (`GTX`), penolakan margin, atau latensi order bursa.** Pada mode `paper`, fill dihasilkan oleh `MockExchange` (`paper_slip_zero_by_construction = true`). Karena itu `apex forward-gate --stage shadow` melaporkan baris slippage sebagai `N/A (fill paper ter-model)` alih-alih mengklaim slippage nol sebagai bukti eksekusi.

### 2.2 Demo Trading (`--stage demo`)
- **Apa yang diukur:**
  - Semantik penandatanganan HMAC-SHA256, filter presisi `tickSize`/`stepSize`/`minNotional`, penolakan `GTX` (`-5022`), penempatan SL/TP kondisional (`/fapi/v1/algoOrder`), siklus user-data stream `/private`, penanganan `UnknownSubmit` (`-1007`/timeout), serta waktu konfirmasi SL ≤ 2 detik (`GR-8`).
  - Selisih `mean(realized_slip_R - model_slip_R) <= 0,05R` terhadap buku order demo.
- **Apa yang TIDAK diukur:**
  - **Dampak pasar nyata, kedalaman likuiditas riil saat rilis berita makro, dan adverse selection market maker mainnet.** Buku order demo memiliki likuiditas sintetis yang dapat berbeda jauh dari mainnet, baik lebih tipis maupun lebih ramah terhadap order limit.

### 2.3 Batas Kesimpulan
Lulus `demo` **dan** `shadow` membuktikan bahwa mesin eksekusi, state store, proteksi SL, dan rekonsiliasi bekerja sesuai spesifikasi tanpa melanggar aturan risiko. **Keduanya tidak menggantikan pengukuran eksekusi nyata pada live ukuran terkecil ($1 risiko per trade), dan bukan bukti bahwa strategi memiliki edge profitabel.**

---

## 3. Cara Menjalankan Evaluasi Gate Forward

Konfigurasi ambang dibaca dari `config/forward_gate.toml`. Perintah mengembalikan **exit code `0`** hanya bila seluruh baris berstatus `PASS` (bila ada satu saja `FAIL` atau `INSUFFICIENT`, proses keluar dengan kode `1`):

```powershell
# Evaluasi tahap Demo (minimal 14 hari & 30 trade)
.\cargo-msvc.cmd run -p apex -- forward-gate --stage demo --since 2026-11-10

# Evaluasi tahap Mainnet Shadow (minimal 7 hari)
.\cargo-msvc.cmd run -p apex -- forward-gate --stage shadow --since 2026-11-25
```

---

## 4. Invarian Runtime `R1`..`R8` (`RULE_VIOLATION`)

Setiap `boot()` dan `tick()`, `OrderManager` menjalankan `check_rule_invariants` dengan deduplikasi per `(rule, trade_id)`:

| Kode | Invarian yang Diperiksa |
|---|---|
| `R1` | `planned_risk` tidak melampaui `risk_usd × (1 + ε)` |
| `R2` | Jarak SL tetap di dalam pita `[sl_min_pct, sl_max_pct]` |
| `R3` | Posisi terbuka / order aktif non-yatim tidak melampaui `max_positions` |
| `R4` | Tidak ada entry baru yang diterima ketika kill switch, cooldown SL, atau daily stop sedang aktif |
| `R5` | Tidak ada posisi terbuka di bursa tanpa algo `STOP_MARKET` aktif lebih dari `2.000 ms` (`GR-8`) |
| `R6` | Tidak ada posisi yatim di bursa yang belum teradopsi melewati masa tenggang rekonsiliasi |
| `R7` | Konfigurasi simbol di bursa sesuai (`CROSSED` dan leverage konfigurasi) |
| `R8` | Pada mode non-paper, hanya strategi berstatus `GO` yang boleh diperdagangkan |
