# Master Plan & Inventaris Penuh Fitur Scalping `Yahudi-Bot` → `apex-scalper-rs` (Rust)

Dokumen ini memuat **inventaris menyeluruh dari seluruh subsistem** yang ada di repositori private `Yahudi-Bot` (khusus **Mode Scalping `SCALP`**, **tanpa `RSW`/Swing** sesuai instruksi operator) beserta peta migrasinya ke crate Rust di `apex-scalper-rs`.

---

## Daftar Lengkap Seluruh Fitur di `Yahudi-Bot` (7 Modul Utama + 5 Modul Mikrostruktur)

### Modul 1 — Universe, Eligibility & Top-Mover Scanner (`universe/` & `scanner/`)
1. **Asset Class Guard (`assetClassGuard.ts`)**:
   - Hanya mengizinkan kontrak **Crypto USDT-M Perpetual** murni; memblokir indeks/komoditas/saham/fiat (`XAU`, `XAG`, `EUR`, `GBP`, `BTCDOM`, kontrak quarterly) dan koin blacklist manual.
2. **Filter Eligibility & Anti-Sideways (`eligibility.ts` & `marketSelectionPolicy.ts`)**:
   - Volume 24 jam wajib `> $7.000.000` (`VOLUME_BELOW_MIN`).
   - Filter anti-sideways wajib `Choppiness(14) < 58.3` (`CHOP_HIGH`) dan `ADX(14) >= 20.0` (`ADX_LOW`).
   - Minimal `structureAtrPct` (`ATR(14)/Close * 100 >= 0.15%`).
3. **Interleaved Top Gainers & Top Losers + Loser Drop Cap (`autonomousTradingBot.ts` & `marketSelectionPolicy.ts`)**:
   - Mengurutkan kandidat berdasarkan `performanceScore(|chg24h|, vol24hUsd)` dengan menyusun selang-seling (*interleaved*) antara **Top Gainer** dan **Top Loser**.
   - **Loser Drop Cap (`MAX_LOSER_DROP_PCT = 15%`)**: Melarang entry pada koin yang sudah jatuh lebih dalam dari `-15%` dalam 24 jam (`LOSER_DROP_TOO_DEEP`).
4. **Scanner Pool & Rotating MTF Window (`scannerPool.ts` & `slotFillPolicy.ts`)**:
   - Memindai hingga `mtfTopN = 48` simbol per siklus (`limit = 90` bar = weight REST 1) + rotasi simbol.

### Modul 2 — 5 Modul Mikrostruktur & Multi-Venue Order Flow Hub (`marketdata/`)
1. **Modul 1: Multi-Venue Order Flow & CVD Hub (`multiVenueOrderFlowHub.ts`)**:
   - 10 koneksi WebSocket publik: Binance Perp, Binance Spot, Hyperliquid, Coinbase, Bybit Linear, Bybit Spot, OKX, Gate, Bitget, KuCoin.
   - Menghitung delta taker USD (`+usd` = Buy, `-usd` = Sell), memisahkan **Spot CVD vs Perp CVD**, `aggression` (`1m/5m/15m/1h`), dan **Large Tape** (`>= $10k`).
2. **Modul 2: Wall Lifelines L2 (`wallLifelines.ts`)**:
   - Melacak siklus hidup dinding order book besar (`>= $100k` dalam `+-3%` mid price): `APPEARED`, `MOVED`, **`PULLED` (Spoofing)** vs **`EATEN` (Absorpsi Nyata)**.
3. **Modul 3: Liquidation Map +-15% (`liquidationMapHub.ts` & `liquidationHub.ts`)**:
   - Peta histogram 30 bin (`+-15%` sekitar mark price) dari Vike Data API & Hyperliquid On-Chain Leaderboard Sampler + stream `forceOrder` Binance.
4. **Modul 4: Volume Profile Hub (`volumeProfileHub.ts` & `volumeProfile.ts`)**:
   - Menghitung **POC**, **VAH**, **VAL** (70% value area) + `pocDistanceBins`.
5. **Modul 5: Size-Tier Imbalance & Cross-Exchange Divergence (`tierFlowHub.ts`)**:
   - 5 tier ukuran trade: `MEGA` (`>= $1M`), `WHALE` (`>= $250k`), `LARGE` (`>= $100k`), `MEDIUM` (`>= $10k`), `SMALL` (`< $10k`) + `whaleRetailBias` + Divergence Alert (`>= $50k`).

### Modul 3 — MTF Analyzer, ZLEMA Shield, Zone/Squeeze & k-NN (`engines/`)
1. **Regime Engine Smoothed 5-Bar (`regimeTrendEngine.ts`)**:
   - Voting mayoritas 5 bar terakhir; entry scalping **HANYA diizinkan saat `TRENDING`**.
2. **Zero-Lag EMA (`ZLEMA 21`) + Volatility Shield (`regimeTrendEngine.ts`)**:
   - `lag = 10`, `adjClose = 2*C - C[i-10]`, `upperShield = ZLEMA + 1.0*ATR(14)`, `lowerShield = ZLEMA - 1.0*ATR(14)`.
   - Volatility Shield memblokir bila `ATR(14)/SMA72(ATR) > 2.0` atau spread `> 0.15%`.
3. **5-Factor Zone Engine & Squeeze Momentum Trigger (`zoneTriggerEngine.ts`)**:
   - Deteksi zona `OB`, `FVG`, `DEMAND`, `SUPPLY` dengan skor 5 faktor (`impulse`, `freshness`, `structureBreak`, `fvgConfluence`, `trendAlign`, `minZoneScore = 60`).
   - Squeeze Momentum (BB 20,2.0 di dalam KC 20,1.5 ATR) mendeteksi `squeezeOn` & `squeezeRelease`.
4. **Lorentzian k-NN Analog Engine (`knnAnalogEngine.ts`)**:
   - Vektor 5D `[zoneScore, triggerScore, trendScore, adx, volRatio]`, jarak Lorentzian `K=5` (`n >= 50` non-netral, `minKnnProb = 0.55`).
5. **Persepsi Big Brain B4 (`perceptionB4.ts` & `trendQuality.ts`)**:
   - Perubahan Open Interest 30m %, Beta terhadap BTC (200 bar 5m), Kaufman ER(30), dan Session VWAP UTC 00:00.

### Modul 4 — Mesin Strategi Scalping (`S-01` & `S-02`) + Menu SL/TP (`strategies/`)
1. **`S-01` — Liquidity Sweep Reclaim (`setupStrategies.ts`)**:
   - `detectRangeSweepReclaim` (30 bar / sesi `ASIA` & `PREV_DAY`), `minDepthAtr = 0.05`, `maxDepthAtr = 1.60`, `reclaimWindowBars = 6`, `minDisplacementAtr = 0.25`, veto cermin tren kuat (`trendScore >= 65` atau `ADX >= 30`).
2. **`S-02` — Liquidity Sniper (`setupStrategies.ts`)**:
   - Tier likuidasi `100x/50x/25x/10x` + stop pools pivot 8 -> cluster merge `0.5*ATR` -> decay half-life 50 -> purge wick touch -> `shortMagnet` & `longMagnet` -> `pullBias` `1.6x` searah ZLEMA.
3. **Menu SL/TP Deterministik (`buildSlTpMenu`)**:
   - Varian SL A (struktural) & B (struktural + buffer `0.46*ATR`), target tunggal `1.7R`.

### Modul 5 — Gate Makro, Sentimen, CVD, Momentum 5m & Kurikulum Progresif (`engines/` & `config/`)
1. **Macro Gate 1h (`macroGate.ts`)**: `BTCUSDT`, `ETHUSDT`, `BTCDOMUSDT` (spike `> +0.5%` blokir altcoin) + Altcoin Breadth Matrix.
2. **Sentiment Reader (`sentimentReader.ts`)**: Fear & Greed Index + Binance Global & Top-Trader Long/Short Crowding (`60/40`).
3. **Hard Gate CVD (`isCvdAligned`) & 5m Closed-Bar Momentum (`confirmEntryMomentum`)** (`body/range >= 0.20`).
4. **Progressive Exploration Policy `L0..L5` (`progressivePolicy.ts`)**.

### Modul 6 — Risk Engine, Pre-Entry Bracket, Zone-Edge Limit & Depth Guard (`risk/` & `exec/`)
1. **Structural Risk Sizing & Fixed `$1.00 / $1.70` Geometry (`riskEngine.ts` & `plannedEntry.ts`)**:
   - Band SL `2.0%–4.0%` (Clamp `< 2%` ke `max(2%, 0.805*ATR)`, Reject `> 4%`), Council floor `SL/ATR >= 0.50x`, leverage otomatis s.d. `25x`, plafon margin `$3` Live / `$4.8` Paper.
2. **Pre-Entry Bracket Verification (`preEntryBracket.ts`)**: Pasang & verifikasi echo SL/TP `reduceOnly+qty` sebelum kirim entry limit.
3. **Zone-Edge Limit (`zoneEntryPrice`) & Post-Only `GTX` 1-Tick Re-Anchor (`anchorPostOnlyEntry`)**.
4. **Order Book Depth & Slippage Guard (`depthGuard.ts`)**.

### Modul 7 — Position Supervisor, Cooldown Berlapis, Watchdog & Self-Learning (`exec/`, `risk/`, `learning/`)
1. **Position Supervisor (`positionSupervisor.ts`)**: Single-target `1.7R` exit, verifikasi gap SL kline 1m (`SL_OBSERVATION_GAP_FILL`), dan **Exit Flip (`TREND_FLIP` / `FLIP_EXCESS`)**.
2. **4 Lapis Cooldown & Anti-Churn**: 15m global post-loss, 60m global post-SL, 10m per-symbol re-entry, 45m symbol probation setelah 3x loss beruntun, dan 2x Live Loss Tripwire.
3. **SMB Self-Learning (`playbookRules.ts`, `tradeForensics.ts`)**: Klasifikasi kesalahan eksekusi, auto-propose/promote/rollback aturan If-Then via Replay Gate.
