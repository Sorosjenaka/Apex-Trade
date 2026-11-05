"""Assemble an isolated 5m research dataset using only pre-holdout data."""
import csv
import json
import pathlib
import shutil
import tomllib
import urllib.parse
import urllib.request

root = pathlib.Path(__file__).resolve().parent.parent
base = root / "research" / "tc3_data"
config = base / "config5"
market = base / "market5"
start, end = 1735689600000, 1777593600000
default = ["ETHUSDT", "SOLUSDT", "XRPUSDT", "DOGEUSDT", "SUIUSDT", "LINKUSDT", "ENAUSDT", "NEARUSDT", "1000PEPEUSDT", "HYPEUSDT", "WLDUSDT"]
if tomllib.loads((root / "config" / "engine.toml").read_text(encoding="utf-8"))["universe"]:
    raise SystemExit("unexpected custom universe; refusing to alter experiment")
config.mkdir(parents=True, exist_ok=True)
for name in ("engine.toml", "risk.toml", "screener.toml", "blacklist.json", "filters_snapshot.json"):
    shutil.copyfile(root / "config" / name, config / name)
strategies = (root / "config" / "strategies.toml").read_text(encoding="utf-8")
if strategies.count("research_entry_5m = false") != 1:
    raise SystemExit("unexpected 5m config default")
(config / "strategies.toml").write_text(strategies.replace("research_entry_5m = false", "research_entry_5m = true"), encoding="utf-8")
for name in ("klines", "funding"):
    (market / name).mkdir(parents=True, exist_ok=True)
for symbol in ["BTCUSDT", "BTCDOMUSDT"] + default:
    for tf in ("15m", "1h"):
        source = (root / "data" if symbol == "HYPEUSDT" else base / "market_clean") / "klines" / f"{symbol}_{tf}.csv"
        shutil.copyfile(source, market / "klines" / source.name)
    if symbol != "BTCDOMUSDT":
        five = base / "five_minute" / "klines" / f"{symbol}_5m.csv"
        shutil.copyfile(five, market / "klines" / five.name)
        if symbol != "HYPEUSDT":
            source = base / "market_clean" / "funding" / f"{symbol}.csv"
            shutil.copyfile(source, market / "funding" / source.name)

# Re-fetch this contract's funding with an explicitly exclusive endpoint.
rows = []
cursor = start
while cursor < end:
    params = {"symbol": "HYPEUSDT", "startTime": cursor, "endTime": end - 1, "limit": 1000}
    url = "https://fapi.binance.com/fapi/v1/fundingRate?" + urllib.parse.urlencode(params)
    with urllib.request.urlopen(url, timeout=30) as response:
        page = json.load(response)
    if not page:
        break
    for row in page:
        stamp = int(row["fundingTime"])
        if not cursor <= stamp < end:
            raise SystemExit("unexpected funding timestamp")
        rows.append((stamp, float(row["fundingRate"])))
    if len(page) < 1000:
        break
    cursor = rows[-1][0] + 1
if not rows or any(a[0] >= b[0] for a, b in zip(rows, rows[1:])):
    raise SystemExit("HYPE funding missing or unsorted")
with (market / "funding" / "HYPEUSDT.csv").open("w", encoding="utf-8", newline="") as handle:
    writer = csv.writer(handle)
    writer.writerow(["time", "rate"])
    writer.writerows(rows)
print(f"Prepared 5m config and isolated data; HYPE pre-holdout funding rows: {len(rows)}")
