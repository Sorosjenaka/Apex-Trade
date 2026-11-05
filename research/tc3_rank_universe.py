"""Select a research universe using public, pre-test daily candles only.

The current exchange metadata and project allowlist introduce survivorship bias;
the selection must never be described as a point-in-time complete universe.
"""
import concurrent.futures
import json
import math
import pathlib
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "research" / "tc3_data"
BASE = "https://fapi.binance.com"
START = 1733011200000  # 2024-12-01 UTC
END = 1735689600000  # 2025-01-01 UTC, exclusive
stop = threading.Event()


def get(path, params=None):
    url = BASE + path
    if params:
        url += "?" + urllib.parse.urlencode(params)
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if error.code in (418, 429):
            stop.set()
        raise


def rank_one(symbol):
    if stop.is_set():
        return None
    rows = get("/fapi/v1/klines", {"symbol": symbol, "interval": "1d", "startTime": START, "endTime": END - 1, "limit": 31})
    if len(rows) != 31 or any(int(row[0]) != START + i * 86400000 for i, row in enumerate(rows)):
        return None
    volumes = [float(row[7]) for row in rows]
    if any(not math.isfinite(v) or v < 0 for v in volumes) or sum(volumes) <= 0:
        raise ValueError("invalid historical quote volume")
    return {"symbol": symbol, "dec_2024_quote_volume": sum(volumes), "days": len(rows)}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    allow = set(json.loads((ROOT / "config" / "blacklist.json").read_text(encoding="utf-8"))["crypto_allowlist_usdt"])
    metadata = get("/fapi/v1/exchangeInfo")
    denied_bases = {"PAXG", "XAUT", "BUSD", "DAI", "EUR", "EURI", "FDUSD", "PYUSD", "RLUSD", "TUSD", "USD1", "USDC", "USDE", "USDP"}
    symbols = sorted(s["symbol"] for s in metadata["symbols"] if s.get("symbol") in allow and s.get("symbol") != "BTCUSDT" and s.get("status") == "TRADING" and s.get("contractType") == "PERPETUAL" and s.get("quoteAsset") == "USDT" and s.get("underlyingType") == "COIN" and s.get("baseAsset") not in denied_bases and not {"tradfi", "pre-market"}.intersection(x.lower() for x in s.get("underlyingSubType", [])) and s.get("onboardDate", END) <= START)
    (OUT / "exchange_info_current.json").write_text(json.dumps(metadata, separators=(",", ":")), encoding="utf-8")
    print(f"Checking {len(symbols)} current allowlisted contracts against December 2024 candles", flush=True)
    found, errors = [], []
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
        for offset in range(0, len(symbols), 4):
            if stop.is_set():
                break
            futures = {executor.submit(rank_one, s): s for s in symbols[offset:offset+4]}
            for future, symbol in futures.items():
                try:
                    row = future.result()
                    if row:
                        found.append(row)
                except Exception as error:
                    errors.append({"symbol": symbol, "error": str(error)})
            if offset % 40 == 0:
                print(f"checked {min(offset+4, len(symbols))}/{len(symbols)}, valid {len(found)}, errors {len(errors)}", flush=True)
            time.sleep(0.3)
    found.sort(key=lambda row: (-row["dec_2024_quote_volume"], row["symbol"]))
    result = {"start": "2024-12-01", "end_exclusive": "2025-01-01", "candidates": len(symbols), "results": found, "errors": errors, "rate_limited": stop.is_set(), "limitations": "current metadata and allowlist imply survivorship bias; rank by December 2024 quote volume, not performance"}
    (OUT / "universe_rank.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    if stop.is_set() or errors or len(found) < 40:
        print("INCOMPLETE: no universe selected; see universe_rank.json", flush=True)
        sys.exit(1)
    selected = [row["symbol"] for row in found[:40]]
    if "ETHUSDT" not in selected:
        print("INCOMPLETE: ETH context missing from top 40", flush=True)
        sys.exit(1)
    (OUT / "universe.json").write_text(json.dumps(selected, indent=2), encoding="utf-8")
    print("Selected: " + ", ".join(selected), flush=True)


if __name__ == "__main__":
    main()
