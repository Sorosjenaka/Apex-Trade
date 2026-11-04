# BLACKLIST.md — Hanya Crypto-Native (Apex Scalper RS)

> Versi 0.1 · data: `https://fapi.binance.com/fapi/v1/exchangeInfo (snapshot fapi_exchangeInfo_20261104T190437Z.json, fetch 20261104T190437Z UTC, serverTime 1793754061893)` (= 2026-11-05 02:04 WIB).
> SSoT untuk: aturan blacklist & allowlist. Daftar mesin: `blacklist.json` (dibangun ulang otomatis, BL-R8).

## 1. Hasil dari data nyata

| Item | Jumlah |
|---|---:|
| Kontrak USDT status TRADING (PERPETUAL + TRADIFI_PERPETUAL) | 756 |
| **Crypto-native diizinkan (allowlist)** | **516** |
| **Diblacklist (quote USDT)** | **240** |
| Diblacklist semua quote (termasuk 1 kontrak USD1) | 241 |

Per kategori (USDT): INDEKS 2, STABLECOIN 1, KOMODITAS_TOKENIZED 2, KOMODITAS 8, SAHAM 185, ETF/ETP 36, PRE-IPO 4, FOREX 1, PRE-MARKET 1

Catatan: pemisahan SAHAM vs ETF/ETP hanya label (daftar ETF manual di `blacklist_build.py`); keduanya `underlyingType=*EQUITY` dan sama-sama diblacklist oleh BL-R1. Di luar scope (otomatis tidak diperdagangkan, BL-R0): quote USDC/USD1/U/BTC, kontrak kuartalan, status SETTLING/PENDING_TRADING.

## 2. Aturan (dievaluasi berurutan, biner)

| ID | Aturan | Field Binance |
|---|---|---|
| BL-R0 | izinkan HANYA jika contractType=='PERPETUAL' AND quoteAsset=='USDT' AND status=='TRADING' AND underlyingType=='COIN'; selain itu default DENY (termasuk listing baru dengan tipe yang belum dikenal) | exchangeInfo |
| BL-R1 | contractType=='TRADIFI_PERPETUAL' -> blacklist (saham, ETF, komoditas, forex, pre-IPO) | exchangeInfo |
| BL-R2 | 'TradFi' in underlyingSubType -> blacklist (jaring pengaman bila Binance memakai contractType PERPETUAL) | exchangeInfo |
| BL-R3 | underlyingType=='INDEX' -> blacklist (BTCDOM, ALL, indeks komposit) | exchangeInfo |
| BL-R4 | underlyingType=='PREMARKET' OR 'Pre-Market' in underlyingSubType -> blacklist | exchangeInfo |
| BL-R5 | underlyingType not in {'COIN'} -> blacklist (default deny tipe baru: EQUITY, HK_EQUITY, KR_EQUITY, CN_EQUITY, COMMODITY, FX, dll) | exchangeInfo |
| BL-R6 | baseAsset in COMMODITY_TOKENS -> blacklist (token yang melacak harga komoditas walau underlyingType=COIN) | exchangeInfo |
| BL-R7 | baseAsset in STABLECOINS -> blacklist (pegged, tidak ada volatilitas scalping) | exchangeInfo |
| BL-R8 | re-evaluasi saat start + tiap 1 jam dari /fapi/v1/exchangeInfo; symbol yang hilang/status!=TRADING otomatis keluar dari universe; hasil berubah -> log + alert | exchangeInfo |

Prinsip: **allowlist, default-deny**. Listing baru otomatis masuk blacklist kalau `contractType`/`underlyingType` bukan `PERPETUAL`/`COIN` atau ada tag `TradFi` — jadi saham/komoditas/ETF baru tidak perlu ditambah manual.

## 3. Daftar per kategori (field pembukti)

### KOMODITAS (8) — `contractType=TRADIFI_PERPETUAL, underlyingType=COMMODITY`
BZUSDT, CLUSDT, COPPERUSDT, NATGASUSDT, XAGUSDT, XAUUSDT, XPDUSDT, XPTUSDT

### KOMODITAS_TOKENIZED (2) — `underlyingType=COIN, subType=RWA`, BL-R6 (token emas)
PAXGUSDT, XAUTUSDT

### FOREX (1) — `underlyingType=FX`
USDBRLUSDT

### PRE-IPO (4) — `underlyingType=PREMARKET, subType=Pre-IPO,TradFi`
ANTHROPICUSDT, MOONSHOTUSDT, OPENAIUSDT, OURAUSDT

### INDEKS (2) — `underlyingType=INDEX` (tetap DIBACA oleh reader sebagai proksi [→ MARKET_READER.md §2 S-02/S-03], tidak diperdagangkan)
ALLUSDT, BTCDOMUSDT

### PRE-MARKET crypto (1) — `underlyingType=PREMARKET, subType=Pre-Market`
PREDICTUSDT

### STABLECOIN (1) — BL-R7
USDCUSDT

### ETF/ETP (36) — `underlyingType=EQUITY, subType=TradFi`
AGPUUSDT, BITOUSDT, CRMLUSDT, CSOPSAMSUNG2LUSDT, CSOPSKHYNIX2LUSDT, DRAMUSDT, EIDOUSDT, EWJUSDT, EWTUSDT, EWYUSDT, EWZSUSDT, EWZUSDT, GDXUSDT, IWMUSDT, KODEX200USDT, KORUUSDT, MSTUUSDT, MUUUSDT, NVDLUSDT, QQQUSDT, SKDDUSDT, SKUUUSDT, SMHUSDT, SOXLUSDT, SOXSUSDT, SPYUSDT, SQQQUSDT, TBTUSDT, TMFUSDT, TQQQUSDT, TSLLUSDT, TZAUSDT, URNMUSDT, UVXYUSDT, XBIUSDT, XLEUSDT

### SAHAM (185 USDT + 1 USD1) — `underlyingType ∈ {EQUITY, HK_EQUITY, KR_EQUITY, CN_EQUITY}, subType=TradFi`
AAOIUSDT, AAPLUSDT, ABBVUSDT, ACNUSDT, ADBEUSDT, AKAMUSDT, ALABUSDT, AMATUSDT, AMCUSDT, AMDUSDT, AMGNUSDT, AMZNUSDT, ANETUSDT, APLDUSDT, APPUSDT, ARMUSDT, ASMLUSDT, ASTSUSDT, AVGOUSDT, AXTIUSDT, BABAUSDT, BACUSDT, BBXUSDT, BEUSDT, BMNRUSDT, BNCUSDT, BOTUSDT, BRKBUSDT, BSPUSDT, BWETUSDT, BXUSDT, BYDUSDT, CATUSDT, CBRSUSDT, CEGUSDT, CIENUSDT, COHRUSDT, COINUSDT, COSTUSDT, CRCLUSDT, CRDOUSDT, CRMUSDT, CRWDUSDT, CRWVUSDT, CSCOUSDT, CVNAUSDT, CXMTUSDT, CYPHUSDT, DDOGUSDT, DELLUSDT, DISUSDT, DJTUSDT, DKNGUSDT, EBAYUSDT, EQTUSDT, FLEXUSDT, FLNCUSDT, FWDIUSDT, GEVUSDT, GIGADEVUSDT, GLWUSDT, GMEUSDT, GOOGLUSDT, GPROUSDT, GSUSDT, GTLBUSDT, HANMIUSDT, HDUSDT, HIMSUSDT, HK0625USDT, HK0700USDT, HK0992USDT, HK1810USDT, HOODUSDT, HPEUSDT, HUTUSDT, HYUNDAIUSDT, IBKRUSDT, IBMUSDT, INTCUSDT, INTWUSDT, IONQUSDT, IRENUSDT, JPMUSDT, KLACUSDT, KOUSDT, KSTRUSDT, KUAISHOUUSDT, LGELECTRONICSUSDT, LITEUSDT, LLYUSDT, LRCXUSDT, LYTEUSDT, MARAUSDT, MCDUSDT, MDBUSDT, MDTUSDT, MEITUANUSDT, MELIUSDT, METAUSDT, MINIMAXUSDT, MPUSDT, MRKUSDT, MRNAUSDT, MRVLUSDT, MSFTUSDT, MSTRUSDT, MUUSDT, MVLLUSDT, NAVERUSDT, NBISUSDT, NETUSDT, NFLXUSDT, NKEUSDT, NOKUSDT, NOWUSDT, NVDAUSDT, NVOUSDT, OKLOUSDT, ONDSUSDT, ORCLUSDT, OXYUSDT, PANWUSDT, PATHUSDT, PAYPUSDT, PDDUSDT, PENGUSDT, PFEUSDT, PLTRUSDT, POPMARTUSDT, PYPLUSDT, QCOMUSDT, QNTXUSDT, RAMUSDT, RDDTUSDT, RDWUSDT, RIVNUSDT, RKLBUSDT, RUMUSDT, SAMSUNGEMUSDT, SAMSUNGUSDT, SECZUSDT, SHAZUSDT, SHOPUSDT, SKHYNIXUSDT, SKHYUSDT, SMCIUSDT, SMRUSDT, SNDKUSDT, SNOWUSDT, SNPSUSDT, SNXXUSDT, SOFIUSDT, SONYUSDT, SPCXUSD1, SPCXUSDT, STMUSDT, STRCUSDT, STXXUSDT, TEAMUSDT, TEMUSDT, TENCENTUSDT, TERUSDT, TSLAUSDT, TSMUSDT, TTWOUSDT, TWSTUSDT, TXNUSDT, UBERUSDT, UNHUSDT, UNITREEUSDT, USARUSDT, USDEXUSDT, VKTXUSDT, VRTUSDT, VSTUSDT, VUSDT, WDCUSDT, WENUSDT, WMTUSDT, XOMUSDT, ZCSHUSDT, ZHIPUUSDT, ZHONGJIUSDT, ZMUSDT, ZSUSDT

## 4. Review manual (diizinkan, dipantau)
- MANTRAUSDT — subType ['RWA', 'Crypto']: RWA-tag tapi token protokol crypto-native; diizinkan, review manual
- CFGUSDT — subType ['RWA', 'Crypto']: RWA-tag tapi token protokol crypto-native; diizinkan, review manual

## 5. Requirement & test
- BL-01 Builder dijalankan saat start dan tiap 60 menit dari `/fapi/v1/exchangeInfo` (BL-R8). Gagal fetch → pakai daftar terakhir ≤ 24 jam; > 24 jam → `DATA_INVALID` [→ MARKET_READER.md §4 RG-0].
- BL-02 Test: fixture exchangeInfo ini → allowlist tepat 516 simbol; XAUUSDT, TSLAUSDT, SPYUSDT, PAXGUSDT, BTCDOMUSDT, USDCUSDT ∉ allowlist; BTCUSDT, ETHUSDT, SOLUSDT ∈ allowlist.
- BL-03 Test: simbol sintetis `underlyingType="NEWTYPE"` → ditolak (default deny).
- BL-04 Order engine menolak simbol di luar allowlist walaupun dipanggil langsung (pertahanan berlapis).
- A-08 🔺 `underlyingSubType` tag `Alpha`/`Meme` tetap crypto-native → diizinkan; risiko likuiditasnya diurus volume gate [→ VOLUME_GATE.md].
- A-09 🔺 Daftar token komoditas BL-R6 & stablecoin BL-R7 manual; ditinjau saat listing baru ber-tag `RWA` muncul (alert).
