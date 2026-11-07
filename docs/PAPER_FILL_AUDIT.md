# Audit realisme fill paper (MockExchange) vs backtest

Tanggal audit: 2026-11-06. Hanya membaca kode; **tidak ada perubahan** pada engine/eksekusi/risk. `arm=false`.

Sumber: [`crates/oms/src/mock.rs`](../crates/oms/src/mock.rs), [`apps/apex/src/live.rs`](../apps/apex/src/live.rs) (`Feed for MockExchange`),
[`crates/backtest/src/engine.rs`](../crates/backtest/src/engine.rs), [`config/engine.toml`](../config/engine.toml).

## Cara paper mengisi order

| Aspek | Paper (MockExchange) | Backtest | Dampak ke perbandingan |
|---|---|---|---|
| Sumber harga | Mark price publik, di-poll tiap `poll_interval_secs` (15 dtk) lalu `set_price` | High/low candle (5m) | **Paper bisa melewatkan wick** di antara dua poll → SL/TP terdeteksi lebih lambat atau tidak sama sekali. Condong optimistis untuk SL yang hanya tersentuh sesaat. |
| Entry limit GTX | Terisi bila mark **menembus** limit (BUY: `px < limit`, SELL: `px > limit`), harga isi = limit, fee maker 0,02% | Terisi bila low/high menembus limit **1 tick** (`penetrated`), harga isi = limit | Keduanya konservatif (butuh tembus). Selisih kecil, tetapi paper hanya melihat sampel 15 dtk → fill bisa lebih jarang. |
| SL / TP | Algo trigger saat mark menyentuh level; isi **MARKET di harga poll** (bukan di level trigger), fee taker 0,05% | Isi di level SL/TP × (1 − slip_sl) | Paper memasukkan slippage alami dari jeda poll; backtest memakai slip tetap. Bisa lebih buruk **atau** lebih baik dari backtest per trade. |
| Funding | **Tidak dimodelkan** | Dimodelkan (±$0,41 dari net $45 pada 18 bulan) | Net paper sedikit lebih tinggi dari backtest. Pengaruh kecil, tetapi bias optimistis. |
| Partial fill / antrean / latensi / kedalaman | Tidak ada (isi penuh) | Tidak ada | Realisme eksekusi live tidak teruji oleh paper. |
| Post-only | Ditolak bila limit sudah melintasi harga (`-5022`) | Tidak ada | Entry yang di backtest "terisi" bisa ditolak di paper. Ini realistis. |
| Fee | maker 0,02% / taker 0,05% | Model fee yang sama (stress = 2×) | Setara. |

## Kesimpulan

1. Paper **lebih baik dari backtest di dua hal** (tanpa funding, wick di antara poll terlewat) dan **lebih buruk di satu hal** (SL/TP terisi di harga poll, bukan di level). Tanda selisih per trade tidak pasti; jangan menyimpulkan edge dari < 20 trade.
2. Paper **tidak membuktikan** kualitas eksekusi live (antrean maker, partial fill, latensi). Itu hanya bisa diuji di testnet/live kecil, yang saat ini dikunci (G-01..G-11 belum lolos, `arm=false`).
3. Penyempurnaan yang **belum dilakukan** karena menyentuh logika eksekusi (butuh persetujuan eksplisit):
   - Isi `set_price` dengan high/low bar 1m terbaru (bukan hanya mark sesaat) agar wick tidak terlewat.
   - Kurangi `poll_interval_secs` untuk simbol yang punya posisi/pending.
   - Tambah biaya funding ke `pnl_net` paper.

## Rencana kalibrasi (setelah ada trade)

Setelah ≥ 20 trade paper tertutup: jalankan backtest pada jendela tanggal yang sama (data harian dari `data.binance.vision`, 0 bobot API) dan bandingkan per trade:
selisih harga entry, selisih harga exit, selisih R. Median selisih R > 0,1R berarti ada bias eksekusi yang perlu diperbaiki sebelum memercayai angka paper.
Progres paper dan kriteria sukses ada di `/api/paper` (target 60 trade, E ≥ +0,10R, PF ≥ 1,25).
