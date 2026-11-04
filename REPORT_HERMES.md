# REPORT_HERMES — apex-wt-hermes (branch `hermes/validation`)

## 1. Sudah jadi

- **Target folder disatukan ke `target-msvc`** (`target/` & `target-msvc-rel/` dihapus, `CARGO_TARGET_DIR` dipin di `cargo-msvc.cmd` + `target-dir` di `.cargo/config.toml`, di-`gitignore`).
- **Test per crate lulus**: `apex-backtest` 11/11, `apex-market-reader` 11/11, `apex` 3/3 (termasuk 2 test baru: split IS/VAL per `exit_ms`, status INCONCLUSIVE saat sampel kurang).
- **Commit** (belum push, sesuai aturan): `451ca0d` target folder; `e1cfca8` gates/stats/engine + `ChopThresholds`; `d7dff24` runner validate + REPORT; `f05ad5e` gitignore; `6e312ca` catatan metodologi; + commit fix gate Rules.md (bt.rs).
- **Gate runner kini patuh Rules.md §4.3** (tindak lanjut review statis Codex — temuan: label PASS gate lama menyesatkan):
  - **G-02**: `IS >= 100` **dan** `VAL >= 60` terpisah (bukan >=60 jendela gabungan); kurang -> **INCONCLUSIVE**, bukan PASS/FAIL. G-11 wajib `HOLDOUT >= 40` trade.
  - **G-03/G-04/G-07**: dihitung pada **segmen VAL** (exit >= 2026-01-01), bukan jendela gabungan. MC bootstrap **trade VAL**.
  - **G-09**: pakai **E_VAL** (`oos_e_by`), bukan E_FIT.
  - Ringkasan JSON memuat `is_trades`, `val_trades`, `val_e`, `val_pf` per strategi (auditable).
- `apex validate --strategies S1,S5` jalan penuh: sweep CHOP, MC 10.000 path, WF anchored 6 fold, stress G-05, DSR — **holdout tidak disentuh** (G-11 LOCKED).

## 2. Hasil validate S1/S5 (tanpa --holdout) — gate Rules.md

Window 2025-02-01 -> 2026-05-01. IS = exit trade s/d 2026-01-01; VAL = exit 2026-01-01 -> 2026-05-01.

| | S1_TFP | S5_RMP |
|---|---|---|
| trades total / IS / VAL | 48 / 36 / 12 | 23 / 16 / 7 |
| E jendela penuh | +0,269R | +0,005R |
| **E VAL (G-03)** | **−0,276R** | +0,243R |
| PF jendela penuh | 1,48 | 1,02 |
| **PF VAL (G-04)** | **0,63** | 1,43 |
| stress G-05 (jendela penuh) | +0,205R / 1,34 | −0,059R / 0,92 |
| WF 6 fold (G-06) | 4/6, terburuk −1,003R | 2/6, terburuk −1,000R |
| MC 10k pada VAL (G-07) | p95DD **24,2R**, P(<$40) **11,2%** | p95DD 9,4R, P(<$40) 0,0% |
| DSR (G-08, N=208) | 0,000 | 0,000 |
| G-09 plateau (E_VAL) | **FAIL** (median −0,088, 50% positif) | PASS (median +1,056, 100%) |
| G-10 konsentrasi | FAIL (simbol 59% / bulan 52%) | FAIL (1381% / 1792% — net tipis) |
| G-11 holdout | LOCKED | LOCKED |

- **G-02 = INCONCLUSIVE di keduanya** (S1: IS 36 < 100, VAL 12 < 60; S5: IS 16, VAL 7). Dengan aturan Rules.md, **hasil gate lain tidak sah sebagai validasi aturan** — output lama yang menampilkan PASS G-03/G-04 dari jendela gabungan tidak boleh dipakai.
- **Temuan penting**: E jendela penuh S1 (+0,269R) menutupi **E VAL negatif (−0,276R)** — edge S1 tidak bertahan di segmen VAL (Jan–Mei 2026), konsisten dengan WF fold 6 yang −1,003R. MC pada VAL juga lewat batas (p95DD 24,2R > 15R; P(<$40) 11,2% > 5%).
- S5: angka VAL tampak positif (+0,243R / PF 1,43) tapi dari **7 trade** — tidak bermakna statistik.
- WF folds S1: +1,539 / +0,293 / −0,044 / +0,455 / +0,087 / −1,003. S5: −0,996 / +0,800 / −0,996 / +0,000 / +1,175 / −1,000.
- **==> NO-GO** kedua strategi. Status: **INCONCLUSIVE (sampel kurang)**, dengan indikasi kuat S1 gagal di VAL.

## 3. Tuning CHOP (T-H2) — hasil sweep

Sekali jalan `apex validate --strategies S1,S5` (FIT 2025-01->2026-01, OOS 2026-01->2026-05, holdout tidak disentuh). Angka dari `reports/validation_summary.json`.

| knob regime | grid | run admissible | adopted (median run) | efek ke blok FIT |
|---|---|---|---|---|
| `trend_share_min` | 0..30 | 7/7 | **15** | 64,5% -> 82,3% (knob efektif) |
| `adx_max` | 6..30 | 7/7 | **15** | 80,2% -> 83,0% (nyaris flat) |
| `er_max` | 0,10..0,40 | 7/7 | **0,25** (= baseline) | 80,4% -> 80,7% (nyaris flat) |
| screener `chop_range` | 45..75 | 0/7 | tidak ada | 89,8–94,4% semua >= 87% |

- **Kombinasi (15 / 15 / 0,25)**: blok FIT **78,5%**, blok OOS **84,0%** (dua < 87% ✓), PF OOS **1,38 vs baseline 1,14 — tidak turun** ✓. T-H2 tercapai pada definisi itu.
- Plateau dilaporkan median run, bukan titik PF tertinggi (th=30, PF 1,62 tidak dipakai).
- Temuan: pemblokiran hampir seluruhnya dari `trend_share`; klausa ADX/ER jarang bind (sweep nyaris flat).
- Rekomendasi adopsi (menunggu review Grok Bot, config belum diubah): `trend_share_min=15`, `adx_max=15`, `er_max=0.25`. **Catatan**: penyaring kandidat plateau memakai PF OOS — lihat §4.

## 4. Belum / catatan

- **G-11 holdout 2026-05-01..2026-10-31 tetap LOCKED** — butuh `--unlock-holdout` (sekali per param_hash) + review user. Dengan aturan baru, G-11 juga wajib >= 40 trade holdout.
- **G-02 INCONCLUSIVE di semua strategi** — sampel jauh di bawah minimum Rules.md (IS>=100, VAL>=60). Validasi apa pun atas S1/S5 saat ini **belum sah**; butuh lebih banyak trade (strategi tambahan yang lolos gate, periode lebih panjang, atau threshold yang menaikkan jumlah trade).
- S1 edge tidak bertahan di VAL (E −0,276R) — perbaikan sinyal dulu (T-C1: kNN grader, magnet zone, OFCC — di Codex) sebelum re-validasi bermakna.
- **DSR 0,000 di kedua strategi** dengan N=208: `var_sr` antar run sweep sangat besar (S5 ±85rb) -> E[max SR] meledak. Definisi trial SR perlu direview (apakah run sweep beda threshold satu populasi).
- **Catatan metodologi (review statis Codex, wajib dibaca bersama angka OOS)**: (1) pemilihan plateau memakai PF OOS untuk menyaring kandidat — angka OOS **bukan evaluasi independen**, butuh outer loop/holdout terpisah untuk klaim final; (2) walk-forward 6 fold parameter beku (tidak refit per fold) — bukan WF re-fit standar. Keduanya memperkuat status NO-GO.
- Adopsi threshold CHOP ke `config` + status GO/NO-GO = wewenang review Grok Bot (jangan push/merge sendiri).
- Keluarga screener `chop_range` belum punya plateau (blok >= 87% di seluruh grid) — grid harus digeser ke bawah 45 bila mau dikejar.
