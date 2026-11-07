# Alur Kerja dan Struktur Workflow

Dokumen ini menguraikan struktur komponen dan alur kerja lengkap **Apex Trade** dari pembacaan pasar hingga siklus pembelajaran pasca-perdagangan.

Gambar diagram alur resolusi penuh (tema gelap):

- Pipeline Sinyal ke Order: [apex-pipeline-id.png](../diagrams/apex-pipeline-id.png)
- Kehidupan Satu Perdagangan: [apex-workflow-id.png](../diagrams/apex-workflow-id.png)

---

## 1. Struktur Hubungan Antarkomponen

```mermaid
flowchart LR
    subgraph SUMBER["Sumber Data Eksternal"]
        EX[(Binance USDT-M<br/>REST + WebSocket)]
        MK[(Feed Mikrostruktur<br/>Likuidasi & Makro)]
    end

    subgraph INTI["Mesin Inti Rust (SCALP Only)"]
        SC[Scanner Pasar 7-Gate<br/>& Asset Class Guard]
        AN[Analisis Multi-TF<br/>ZLEMA · CVD · VP48]
        SG[Mesin Sinyal<br/>S1_TFP · S-01 · S-02]
        GR[Gerbang Risiko $1<br/>& Volatility Shield]
        AI[Dewan 3-Agen & Arena<br/>Vega · Arka · Wardha]
        EK[Eksekutor OMS<br/>MockExchange / GTX]
        MP[Pengawas Posisi<br/>Anti-Orphan & Trend-Flip]
    end

    subgraph SIMPAN["Penyimpanan & Antarmuka"]
        ST[(StateStore JSON<br/>& Audit JSONL)]
        WS[Dashboard & Terminal Web<br/>http://127.0.0.1:8787]
        LR[Siklus Belajar<br/>5-Whys · Wilson LB · CPCV]
    end

    EX --> SC --> AN --> SG --> GR --> AI --> EK --> MP
    MK --> AN
    EK <--> EX
    MP <--> EX
    SC & AN & SG & GR & AI & EK & MP --> ST
    ST --> WS
    ST --> LR
    LR -. evaluasi & validasi .-> SG
```

---

## 2. Pipeline Perdagangan (Dari Sinyal ke Posisi)

Setiap kandidat wajib melewati seluruh gerbang secara berurutan. Kegagalan pada salah satu tahap langsung menghentikan kandidat dan mencatat alasan penolakannya di `audit.jsonl`.

![Pipeline Sinyal ke Order](../diagrams/apex-pipeline-id.png)

```mermaid
flowchart TD
    A[Penutupan Bar Baru 5m / 15m] --> B[Asset Class Guard & Volume Gate]
    B --> C{Rezim Pasar & Makro<br/>Mengizinkan?}
    C -->|CHOP_NO_TRADE / SHOCK_HALT| X1[Lewati — tercatat di audit]
    C -->|Lolos| D[Evaluasi Sinyal Scalping<br/>S1_TFP · S01_SWEEP · S02_SNIPER]
    D --> E{Setup Valid &<br/>Momentum 5m Selaras?}
    E -->|Tidak| X2[Lewati — tercatat di audit]
    E -->|Ya| F{Gerbang Risiko $1<br/>SL 2–4% & Volatility Shield}
    F -->|Gagal / stepSize tidak muat| X3[Tolak — tercatat di audit]
    F -->|Lolos| G{Sidang Dewan 3-Agen<br/>Vega · Arka · Wardha}
    G -->|Veto / Konsensus Kurang| X4[Tolak — ARTATRADE_COUNCIL_REJECT]
    G -->|EXECUTE| H[Kirim Order via OMS<br/>Simulasi PAPER / Nyata GTX]
    H --> I{Order Terisi<br/>Sebelum TTL Habis?}
    I -->|Kedaluwarsa| X5[Batalkan Order — tercatat]
    I -->|Terisi| J[Pasang SL/TP Algo &<br/>Masuk Pengawasan Posisi]
```

---

## 3. Alur Sidang Dewan 3-Agen (*Fail-Closed*)

```mermaid
flowchart TD
    A[Kandidat Lolos Gerbang Risiko $1] --> B{Vega — Macro Scout<br/>Matriks 6-State BTC.D × BTC}
    B -->|Hard Veto Conf > 70| TOLAK([Tolak Eksekusi])
    B -->|Lanjut| C{Arka — Setup Judge<br/>Skor Komposit · k-NN · CVD}
    C -->|Hard Veto Conf > 70| TOLAK
    C -->|Lanjut| D{Wardha — Risk Officer<br/>Rasio SL/ATR & Menu Proteksi}
    D -->|Suara GO < 2 atau AvgConf < 60| TOLAK
    D -->|Konsensus >= 2 GO & Tanpa Veto| E([EXECUTE — Teruskan ke OMS])

    style TOLAK fill:#3b1219,stroke:#ef4444,color:#fecaca
    style E fill:#0c2e24,stroke:#10b981,color:#bbf7d0
```

---

## 4. Manajemen Posisi Terbuka & Proteksi

```mermaid
flowchart TD
    A[Posisi Terbuka] --> B[Pasang Stop-Loss & Take-Profit<br/>Conditional Algo MARK_PRICE]
    B --> C[Pemantauan Setiap Siklus<br/>OMS Reconciler + Trend-Flip Guard]
    C --> D{Kondisi Keluar Terpicu?}
    D -->|Take-Profit Tersentuh| E[Tutup Profit & Batalkan Order Sisa]
    D -->|Stop-Loss Tersentuh| F[Tutup Rugi Terukur ~1R & Aktifkan Cooldown]
    D -->|Tren Berbalik Terkonfirmasi| G[Trend-Flip Early Exit]
    D -->|Posisi Yatim Terdeteksi| H[Anti-Orphan: Pasang SL Darurat / Flatten]
    E & F & G & H --> I[Catat Realisasi PnL ke OMS State & Audit]
    I --> J[Jalankan Forensik 5-Whys]
```

---

## 5. Siklus Belajar & Validasi Statistik

![Kehidupan Satu Perdagangan](../diagrams/apex-workflow-id.png)

```mermaid
flowchart LR
    A[Perdagangan Selesai] --> B[Forensik 5-Whys<br/>Klasifikasi Akar Masalah]
    B --> C[Statistik Probabilitas<br/>Wilson 95% Lower Bound]
    C --> D[Usulan Parameter Terbatas<br/>Bounded Tuner <= 10%]
    D --> E[Uji Backtest & CPCV<br/>Anti-Overfitting + Stress Fee]
    E -.->|Hanya promosi bila lolos gate| F[Konfigurasi Strategi]
```

Untuk panduan instalasi dan pengoperasian harian, lanjutkan ke [03-tutorial-pasang.md](03-tutorial-pasang.md).
