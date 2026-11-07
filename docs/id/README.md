# Panduan Apex Trade (Bahasa Indonesia)

**Apex Trade** adalah sistem perdagangan scalping otomatis berbasis **Rust** untuk pasar **Binance USDT-M Futures**. Dokumentasi ini menjelaskan arsitektur sistem, daftar fitur, alur kerja kandidat hingga penutupan posisi, serta langkah pemasangan yang aman — tanpa menyertakan kredensial atau kunci rahasia apa pun.

## Isi Panduan

| Dokumen | Isi |
|---|---|
| [01-ikhtisar-dan-fitur.md](01-ikhtisar-dan-fitur.md) | Ikhtisar sistem, fokus murni *Scalping*, dan rincian fitur utama |
| [02-alur-kerja-workflow.md](02-alur-kerja-workflow.md) | Struktur sistem dan alur kerja lengkap dari pemindaian hingga pembelajaran |
| [03-tutorial-pasang.md](03-tutorial-pasang.md) | Panduan kompilasi Rust, menjalankan simulasi `PAPER`, dashboard, dan watchdog |
| [04-arsitektur-c4.md](04-arsitektur-c4.md) | Arsitektur sistem dengan model C4 (*Konteks → Wadah → Komponen*) |

## Diagram Arsitektur (Versi Gelap)

- Konteks Sistem (C4 Level 1): [docs/diagrams/apex-c4-konteks-id.png](../diagrams/apex-c4-konteks-id.png)
- Wadah / Container (C4 Level 2): [docs/diagrams/apex-c4-wadah-id.png](../diagrams/apex-c4-wadah-id.png)
- Komponen Mesin Inti (C4 Level 3): [docs/diagrams/apex-c4-komponen-id.png](../diagrams/apex-c4-komponen-id.png)
- Pipeline Sinyal ke Order: [docs/diagrams/apex-pipeline-id.png](../diagrams/apex-pipeline-id.png)
- Kehidupan Satu Perdagangan: [docs/diagrams/apex-workflow-id.png](../diagrams/apex-workflow-id.png)

Dokumentasi teknis berbahasa Inggris tersedia di [docs/public/](../public/README.md).
