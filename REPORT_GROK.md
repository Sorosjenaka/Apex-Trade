# Laporan Grok — tutup sisa T-C3

Branch: `codex/sample` di `C:\Users\FATHUR\Downloads\apex-wt-codex`. Tidak push, tidak merge. Tidak ada order, akses `.env`, atau run holdout. `arm` dan `mode` tidak diubah. Semua strategi tetap NO-GO.

Sisa yang terbuka adalah kode entry 5m yang belum di-commit dan tes workspace yang mati di tengah (`exit 0xffffffff` pada `deterministic_and_risk_invariants`). Replay opsi a dan b sudah ada. Opsi c dan d tidak ada di catatan tugas, jadi tidak dibuat.

`cargo test --workspace --offline` lulus:

| Crate | Lulus | Gagal | Durasi |
|---|---:|---:|---:|
| apex | 7 | 0 | 0,01 d |
| apex-backtest | 21 | 0 | 306,93 d |
| apex-core | 23 | 0 | 0,25 d |
| apex-data | 6 | 0 | 0,02 d |
| apex-indicators | 6 | 0 | 0,01 d |
| apex-market-reader | 18 | 0 | 0,01 d |
| apex-oms | 7 | 0 | 0,00 d |
| state_machine | 20 | 0 | 0,19 d |
| apex-screener | 8 | 0 | 0,43 d |
| apex-strategies | 17 | 0 | 81,97 d |

Jumlah 133 lulus, 0 gagal. Doc-test 0.

Angka replay yang sudah tersimpan, bukan hasil tes ulang hari ini:

- Baseline S1: 29 trade, VAL 17, E VAL +0,195R, PF 1,776.
- Universe 40, S1: 64 trade, VAL 35, E VAL +0,073R, PF 1,468. Di bawah G-02 dan G-03. Mei 2025 melebihi batas bulan G-10.
- Entry 5m, S1: 44 trade, VAL 21, E VAL −0,316R, PF 1,197. Tidak diadopsi.

`research_entry_5m` di `config/strategies.toml` tetap `false`.
