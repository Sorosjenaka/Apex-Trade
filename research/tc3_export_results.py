"""Save per-run metrics and symbol/month concentrations, without trade data."""
import json
import pathlib
import re
import sys
from tc3_metrics import summarize

source = pathlib.Path(sys.argv[1])
label = sys.argv[2]
if not re.fullmatch(r"[a-z0-9_]+", label):
    raise SystemExit("invalid result label")
files = sorted(source.glob("bt_*.json"))
if len(files) != 3:
    raise SystemExit(f"expected S1, S3 and combined reports, found {len(files)}")
result = {}
for file in files:
    report = json.loads(file.read_text(encoding="utf-8"))
    key = "+".join(report["strategies"])
    if key in result:
        raise SystemExit(f"duplicate run {key}")
    result[key] = summarize(report)
out = pathlib.Path(__file__).resolve().parent / "results"
out.mkdir(exist_ok=True)
(out / (label + ".json")).write_text(json.dumps(result, indent=2), encoding="utf-8")
print(f"saved {label}: " + ", ".join(result))
