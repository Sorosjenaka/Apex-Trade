"""Fail closed on incomplete pre-holdout research files before a replay."""
import csv
import json
import pathlib
import sys

from tc3_metrics import START, END

root = pathlib.Path(__file__).resolve().parent
market = pathlib.Path(sys.argv[1])
rank = json.loads((root / "tc3_universe_2025.json").read_text(encoding="utf-8"))
symbols = ["BTCUSDT", "BTCDOMUSDT"] + [x["symbol"] for x in rank["results"][:40]]
issues = []
for symbol in symbols:
    for tf, step in (("15m", 900_000), ("1h", 3_600_000)):
        path = market / "klines" / f"{symbol}_{tf}.csv"
        try:
            with path.open(encoding="utf-8", newline="") as handle:
                timestamps = [int(row["open_time"]) for row in csv.DictReader(handle)]
            expected = (END - START) // step
            if len(timestamps) != expected or any(t != START + i * step for i, t in enumerate(timestamps)):
                issues.append(f"{symbol} {tf}: missing, duplicate, out-of-range or misaligned bars")
        except (OSError, ValueError, KeyError) as error:
            issues.append(f"{symbol} {tf}: {error}")
    if symbol != "BTCDOMUSDT":
        try:
            with (market / "funding" / f"{symbol}.csv").open(encoding="utf-8", newline="") as handle:
                times = [int(row["time"]) for row in csv.DictReader(handle)]
            if not times or any(not START <= t < END for t in times) or any(a >= b for a, b in zip(times, times[1:])):
                issues.append(f"{symbol} funding: missing, duplicate, unsorted or out-of-range")
        except (OSError, ValueError, KeyError) as error:
            issues.append(f"{symbol} funding: {error}")
print(json.dumps({"symbols": len(symbols), "issues": issues}, indent=2))
if issues:
    sys.exit(1)
