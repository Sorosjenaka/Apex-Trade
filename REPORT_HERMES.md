# REPORT_HERMES — apex-wt-hermes (branch `hermes/validation`)

## 1. Sudah jadi

- **Target folder disatukan ke `target-msvc`**: `target/` (106 MB) & `target-msvc-rel/` (229 MB) dihapus. `CARGO_TARGET_DIR` dipin di `cargo-msvc.cmd` + `target-dir` di `.cargo/config.toml` — tidak akan ada folder target baru lagi.
- **Test per crate lulus** (`cargo test -p apex-backtest -p apex-market-reader`, debug, EXIT=0):
  - `apex-backtest` 11/11 (termasuk 4 test `gates::*`: anchored folds, walk-forward/plateau, plateau median, honest trials)
  - `apex-market-reader` 11/11 (termasuk `chop_threshold_overrides_default`)
- **Commit** (belum push, sesuai aturan): `451ca0d` build: satu target folder; `e1cfca8` feat(backtest): gates G-02..G-11 murni, plateau CHOP, stats sharpe/var, fill extra_slip, `ChopThresholds` di market_reader.
- **Perbaikan ikut ke-commit**: match `param_hash` ledger per-field JSON (bukan substring), fill limit kena `extra_slip` (baseline 0; stress G-05 menggeser berlawanan arah).
- `apex validate --strategies S1,S5` jalan penuh: sweep CHOP 14 titik (regime + screener), MC 10.000 path, WF anchored 6 fold, stress G-05, DSR — **holdout tidak disentuh** (data hanya sampai 2026-05-01, window default 2025-02-01→2026-05-01, G-11 status LOCKED).

## 2. Hasil validate S1/S5 (tanpa --holdout)

Window: 2025-02-01 → 2026-05-01. FIT sweep 2025-01→2026-01, OOS 2026-01→2026-05.

### S1_TFP

| metric | base | stress (fee×2, slip+0,05%, funding×2) |
|---|---|---|
| trades | 48 | 48 |
| WR | 43,8% | 43,8% |
| PF | 1,48 | 1,34 |
| E[R] | +0,269R | +0,205R |
| net | +$12,76 | +$9,74 |
| maxDD | 7,1R | 7,8R |

- PASS: G-03 (+0,269R), G-04 (PF 1,48), G-05 (stress +0,205R / 1,34), G-07 (MC p95DD 11,6R, P(<$40)=0,1%), G-09 (tetangga 2/2 positif, median +0,525)
- FAIL: G-02 (48 < 60 trade), G-06 (4/6 fold positif tapi terburuk −1,003R di fold 6 = Feb–Mei 2026), G-08 (DSR 0,000 — varSR antar run 1730), G-10 (simbol 59% / bulan 52% dari net)
- WF folds: +1,539 / +0,293 / −0,044 / +0,455 / +0,087 / −1,003
- **==> NO-GO**

### S5_RMP

| metric | base | stress |
|---|---|---|
| trades | 23 | 23 |
| WR | 34,8% | 34,8% |
| PF | 1,02 | 0,92 |
| E[R] | +0,005R | −0,059R |
| net | +$0,26 | −$1,20 |
| maxDD | 5,0R | 5,3R |

- PASS: hanya G-07
- FAIL: G-02 (23), G-03, G-04, G-05, G-06 (2/6), G-08 (DSR 0,000, varSR 155.623), G-09 (1/2), G-10 (konsentrasi 1381%/1792% — net tipis, rasio meledak)
- **==> NO-GO** (edge tipis, hampir breakeven)

### Plateau CHOP sweep (G-09 input)

- **Regime `trend_share_min`**: seluruh grid 0..30 admissible, run = 0..30, **adopted = 15** (median run, bukan PF tertinggi). Di 15: blok FIT 78,8% (< 87%), blok OOS 84,4% (< 87%), PF OOS 1,38 vs baseline 1,14 (tidak turun).
- **Screener `chop_range` 45..75**: **tidak ada run admissible** (blok FIT 89,8–94,4% semua ≥ 87%) — keluarga ini belum punya plateau.
- G-09 gate per strategi memakai keluarga screener (blok baseline lebih tinggi: 93,1% vs 80,6%).

## 3. Tuning CHOP (T-H2) — hasil sweep

Sekali jalan `apex validate --strategies S1,S5` (FIT 2025-01→2026-01, OOS 2026-01→2026-05, holdout tidak disentuh). Semua angka dari run nyata (`reports/validation_summary.json`, `validate_chop.log`).

| knob regime | grid | run admissible | adopted (median run) | efek ke blok FIT |
|---|---|---|---|---|
| `trend_share_min` | 0..30 | 7/7, run=0..30 | **15** | 64,5% → 82,3% (knob efektif) |
| `adx_max` | 6..30 | 7/7, run=6..30 | **15** | 80,2% → 83,0% (nyaris flat) |
| `er_max` | 0,10..0,40 | 7/7, run=0,10..0,40 | **0,25** (= baseline) | 80,4% → 80,7% (nyaris flat) |
| screener `chop_range` | 45..75 | 0/7 | tidak ada | 89,8–94,4% semua ≥ 87% |

- **Kombinasi (15 / 15 / 0,25)**: blok FIT **78,5%**, blok OOS **84,0%** (keduanya < 87% ✓), E OOS +0,221R, **PF OOS 1,38 vs baseline 1,14 — tidak turun** (✓).
- T-H2 tercapai: ada threshold CHOP yang tidak memblok 87% bar tanpa menurunkan PF OOS. Plateau dilaporkan apa adanya — titik PF OOS tertinggi (th=30, PF 1,62) **tidak** dipakai; yang dilaporkan median run.
- Temuan: hampir semua pemblokiran datang dari klausa `trend_share < trend_share_min`; klausa `(ADX<18 && ER<0,25)` jarang bind (AND-nya jarang keduanya rendah) — sweep ADX/ER nyaris flat.
- Rekomendasi adopsi (menunggu review Grok Bot, belum diubah di config): `trend_share_min=15`, `adx_max=15`, `er_max=0.25`.

## 4. Belum

- **G-11 holdout 2026-05-01..2026-10-31 tetap LOCKED** — belum pernah dibuka, butuh `--unlock-holdout` (sekali per param_hash) + review user.
- S1: kurang sampel (48 < 60), fold 6 (Feb–Mei 2026) E −1,003R, konsentrasi simbol/bulan lewat batas, DSR 0.
- S5: edge hampir nol (PF 1,02) — perlu perbaikan sinyal (T-C1: kNN grader, magnet zone, OFCC — di Codex).
- **DSR 0,000 di kedua strategi**: `var_sr` antar run sweep sangat besar (S1 ±944, S5 ±155rb) → E[max SR] mendekati tak hingga. Definisi trial SR perlu direview (apakah run sweep beda threshold boleh dihitung satu populasi).
- Adopsi threshold CHOP ke `config` + status GO/NO-GO = wewenang review Grok Bot (rule: jangan push/merge sendiri).
- Keluarga screener `chop_range` belum punya plateau (blok ≥ 87% di seluruh grid) — kalau mau diturunkan, grid-nya harus digeser ke bawah 45.
