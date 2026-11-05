"""Create a separate research config; leave the project config untouched."""
import json
import hashlib
import pathlib
import shutil

root = pathlib.Path(__file__).resolve().parent.parent
data = root / "research" / "tc3_data"
target = data / "config"
target.mkdir(parents=True, exist_ok=True)
rank = json.loads((root / "research" / "tc3_universe_2025.json").read_text(encoding="utf-8"))
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
if digest(data / "exchange_info_current.json") != rank["exchange_info_sha256"]:
    raise SystemExit("exchange metadata snapshot does not match ranking")
if digest(root / "config" / "blacklist.json") != rank["allowlist_sha256"]:
    raise SystemExit("allowlist snapshot does not match ranking")
if rank["errors"] or rank["rate_limited"]:
    raise SystemExit("ranking is incomplete")
universe = [entry["symbol"] for entry in rank["results"][:40]]
if len(universe) != 40 or "ETHUSDT" not in universe or "BTCUSDT" in universe:
    raise SystemExit("invalid universe")
engine = (root / "config" / "engine.toml").read_text(encoding="utf-8")
if engine.count("universe = []") != 1:
    raise SystemExit("unexpected engine universe")
engine = engine.replace("universe = []", "universe = " + json.dumps(universe))
(target / "engine.toml").write_text(engine, encoding="utf-8")
for name in ("risk.toml", "screener.toml", "strategies.toml", "blacklist.json"):
    shutil.copyfile(root / "config" / name, target / name)
shutil.copyfile(data / "exchange_info_current.json", target / "filters_snapshot.json")
print(f"Prepared independent research configuration with {len(universe)} symbols")
