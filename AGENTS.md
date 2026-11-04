# AGENTS.md - aturan wajib semua agen (Grok Bot, Codex, Hermes)

1. DILARANG mengirim order ke Binance (live maupun testnet). Pakai MockExchange / paper saja.
2. arm=false dan mode=paper JANGAN diubah. Jangan sentuh kode safety gate live (APEX_LIVE_CONFIRM, host check).
3. DILARANG membaca, mencetak, atau membuat .env / API key. Data publik saja.
4. Kerja hanya di worktree & branch masing-masing. Jangan push, jangan merge ke master. Grok Bot yang review & merge.
5. Holdout 2026-05-01..2026-10-31 terkunci. Jangan jalankan --holdout. Jangan tuning pakai data holdout.
6. Semua klaim harus ada angka dari run nyata. Jangan klaim profit tanpa validate GO.
7. Wajib lulus: cargo test --workspace sebelum commit. Commit kecil dan jelas.
8. Tulis laporan akhir ke REPORT_<agen>.md di root worktree.
