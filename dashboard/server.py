#!/usr/bin/env python3
"""Apex Scalper RS — dashboard paper (read-only, kecuali notes.jsonl di dashboard/library).
Jalankan dari root repo: python dashboard/server.py
Buka http://127.0.0.1:8787
Tidak ada tombol order. Tidak membaca .env. Tidak mengubah config/state engine.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from datetime import datetime

ROOT = Path(__file__).resolve().parent.parent
HERE = Path(__file__).resolve().parent
SDIR = ROOT / "state" / "paper"
LIBDIR = HERE / "library"
NOTES = LIBDIR / "notes.jsonl"
PORT = int(os.environ.get("APEX_DASH_PORT", "8787"))
TRACEBACK_LOG = ROOT / "state" / "paper" / "dashboard_traceback.log"
V10_HASH = "92a1fe3f5d2d45e3"

CYCLE = re.compile(
    r"^(\S+Z?)\s+regime=(\S+)(?:\s+macro=\S+\s+alt_ok=\S+)?\s+long_ok=(\S+)\s+short_ok=(\S+)\s+max_pos=(\S+)\s+"
    r"equity=\$([\d.\-]+)\s+open=(\d+)\s+pending=(\d+)"
)
ENTRY = re.compile(r"ENTRY\s+(\S+)\s+(\S+)\s+(\S+)")
SKIP = re.compile(r"sinyal\s+(\S+)\s+(\S+)\s+(\S+).*?\bSKIP[_\s]?(\S+)", re.I)
SKIP2 = re.compile(r"SKIP[_\s]([A-Z0-9_]+)", re.I)

_lock = threading.RLock()
_cache: dict = {}
_ban_until = 0.0
_SYMBOLS: set[str] = set()
_SYMBOLS_META: list = []
_SSE_CLIENTS: list = []
_VISIBLE: set[str] = set()  # "SYM|tf" or SYM for tickers
# Keep the full public USD-M perpetual universe visible to the dashboard.
# Signal overlays are removed in the frontend; this server supplies market data only.
_UNIVERSE_EXACT = True
_POLL_THREAD = None
_WS_STATUS = {"status": "idle", "note": "ISP likely drops WS data frames after handshake; REST+SSE used"}



def fmt2(x, nd=2):
    """Round display numbers: floats -> nd decimals; None -> em dash."""
    if x is None:
        return "—"
    try:
        f = float(x)
        if f != f or f in (float("inf"), float("-inf")):  # NaN/inf
            return "—"
        if abs(f - round(f)) < 1e-9 and abs(f) >= 1:
            return str(int(round(f))) if nd == 0 else f"{f:.{nd}f}"
        return f"{f:.{nd}f}"
    except (TypeError, ValueError):
        return str(x)


def fmt_pct(x, nd=2):
    if x is None:
        return "—"
    try:
        return f"{float(x):.{nd}f}%"
    except (TypeError, ValueError):
        return str(x)


def _now() -> float:
    return time.time()


def cache_get(key, ttl):
    with _lock:
        hit = _cache.get(key)
        if hit and _now() - hit[0] < ttl:
            return hit[1], True
        return (hit[1] if hit else None), False


def cache_set(key, val):
    with _lock:
        _cache[key] = (_now(), val)


def tail_lines(path: Path, n=400):
    try:
        with path.open("r", encoding="utf-8", errors="replace") as f:
            return f.readlines()[-n:]
    except OSError:
        return []


def read_json(path: Path):
    try:
        with path.open("r", encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None



def refresh_symbols():
    """Cache TRADING USDT-M perpetuals from exchangeInfo (1h)."""
    global _SYMBOLS, _SYMBOLS_META
    data, fresh = cache_get("exchangeInfo", 3600)
    if (not fresh or not data) and _now() >= _ban_until:
        raw = binance_get("/fapi/v1/exchangeInfo", ttl=3600)
        if isinstance(raw, dict):
            data = raw
            cache_set("exchangeInfo", data)
    elif (not fresh or not data):
        # banned: keep last-good cache key if any
        good, _ = cache_get("bn_good:/fapi/v1/exchangeInfo", 86400)
        if isinstance(good, dict):
            data = good
    if not isinstance(data, dict):
        return list(_SYMBOLS)
    syms = set()
    meta = []
    for s in data.get("symbols") or []:
        if s.get("status") != "TRADING":
            continue
        if s.get("quoteAsset") != "USDT":
            continue
        # Dashboard universe = every active USD-M USDT perpetual; exclude
        # delivery contracts but keep all currently trading perpetual symbols.
        ct = s.get("contractType") or ""
        if ct and ct != "PERPETUAL":
            continue
        sym = s.get("symbol")
        if not sym:
            continue
        syms.add(sym)
        tick_size = None
        try:
            for filt in s.get("filters") or []:
                if filt.get("filterType") == "PRICE_FILTER":
                    tick_size = str(filt.get("tickSize") or "")
                    break
        except Exception:
            tick_size = None
        precision = 0
        if tick_size and "." in tick_size:
            precision = len(tick_size.rstrip("0").split(".", 1)[1])
        meta.append({"symbol": sym, "base": s.get("baseAsset"), "quote": s.get("quoteAsset"),
                     "tick_size": tick_size, "precision": precision})
    if syms:
        _SYMBOLS = syms
        _SYMBOLS_META = meta
    return sorted(_SYMBOLS)


def valid_symbol(sym: str) -> bool:
    sym = (sym or "").upper()
    if sym in _SYMBOLS:
        return True
    # lazy refresh once
    refresh_symbols()
    return sym in _SYMBOLS


def alive() -> bool:
    val, fresh = cache_get("apex_alive", 2.5)
    if fresh and val is not None:
        return bool(val)
    try:
        out = subprocess.run(
            ["tasklist", "/FI", "IMAGENAME eq apex.exe", "/FO", "CSV", "/NH"],
            capture_output=True,
            text=True,
            timeout=3,
        ).stdout
        ok = "apex.exe" in out.lower()
        cache_set("apex_alive", ok)
        return ok
    except Exception:
        cache_set("apex_alive", False)
        return False


def _binance_payload_ok(path: str, raw) -> bool:
    """Never treat empty/error payloads as cacheable success."""
    if raw is None:
        return False
    if isinstance(raw, list):
        # 24hr all-tickers must be non-empty; single-symbol endpoints may be list
        if "ticker/24hr" in path and "?" not in path:
            return len(raw) > 0
        return True
    if isinstance(raw, dict):
        if raw.get("code") not in (None, 0) and raw.get("msg"):
            return False
        return True
    return False


def binance_get(path: str, ttl: float, weight_hint: int = 1):
    """Public GET with cache + 418/429 backoff. Serves last-good on ban/error.
    Never caches empty/error results as success.
    """
    global _ban_until
    key = "bn:" + path
    good_key = "bn_good:" + path
    data, fresh = cache_get(key, ttl)
    good, _ = cache_get(good_key, 86400)
    if fresh and _binance_payload_ok(path, data):
        return data
    if _now() < _ban_until:
        return good if _binance_payload_ok(path, good) else data
    url = "https://fapi.binance.com" + path
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "ApexDash/1.0"})
        with urllib.request.urlopen(req, timeout=12) as r:
            raw = json.loads(r.read().decode())
        if _binance_payload_ok(path, raw):
            _ban_until = 0.0  # successful fetch clears stale IP ban latch
            cache_set(key, raw)
            cache_set(good_key, raw)
            return raw
        # non-ok body: keep last good
        return good if _binance_payload_ok(path, good) else raw
    except urllib.error.HTTPError as e:
        if e.code in (418, 429):
            retry = 60.0
            try:
                ra = e.headers.get("Retry-After")
                if ra:
                    retry = float(ra)
            except Exception:
                pass
            _ban_until = _now() + min(900.0, max(30.0, retry))
            print(f"[binance] HTTP {e.code} backoff {min(900.0, max(30.0, retry)):.0f}s until {_ban_until:.0f}", flush=True)
            # annotate last-good wrapper only for callers that inspect meta — raw return stays payload
        return good if _binance_payload_ok(path, good) else data
    except Exception:
        return good if _binance_payload_ok(path, good) else data


def parse_log():
    log = tail_lines(SDIR / "run.log", 4000)
    err = [l.rstrip() for l in tail_lines(SDIR / "run.err.log", 50) if l.strip()]
    cycles, entries, skips, lines = [], [], [], []
    for l in log:
        s = l.rstrip()
        if not s:
            continue
        lines.append(s)
        m = CYCLE.match(s.strip())
        if m:
            cycles.append(
                {
                    "ts": m[1],
                    "regime": m[2],
                    "long_ok": m[3].lower() == "true",
                    "short_ok": m[4].lower() == "true",
                    "max_pos": int(m[5]) if m[5].isdigit() else 0,
                    "equity": float(m[6]),
                    "open": int(m[7]),
                    "pending": int(m[8]),
                }
            )
            continue
        m = ENTRY.search(s)
        if m:
            entries.append(
                {
                    "kind": "ENTRY",
                    "strategy": m[1],
                    "symbol": m[2],
                    "side": m[3],
                    "line": s.strip(),
                    "ts": None,
                }
            )
            continue
        m = SKIP.search(s)
        if m:
            skips.append(
                {
                    "kind": "SKIP",
                    "strategy": m[1],
                    "symbol": m[2],
                    "side": m[3],
                    "reason": m[4].lstrip("_"),
                    "line": s.strip(),
                    "ts": None,
                }
            )
    # try extract leading ISO ts from entry/skip lines
    for arr in (entries, skips):
        for e in arr:
            m = re.match(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})", e["line"])
            if m:
                e["ts"] = m[1] + "Z"
    return cycles, entries, skips, lines, err


def parse_audit(n=500):
    audit = []
    signals = []
    mr09 = []
    for l in tail_lines(SDIR / "audit.jsonl", n):
        try:
            o = json.loads(l)
        except ValueError:
            continue
        audit.append(o)
        ev = o.get("event", "")
        d = o.get("data") or {}
        ts = o.get("ts")
        if ev == "MR09_SHADOW":
            mr09.append(
                {
                    "ts": ts,
                    "rev_up": (None if d.get("rev_up") is None else round(float(d.get("rev_up")), 2)),
                    "rev_down": (None if d.get("rev_down") is None else round(float(d.get("rev_down")), 2)),
                    "as_of": d.get("as_of"),
                    "context": d.get("context"),
                }
            )
        elif ev in ("SKIP", "ENTRY", "SUBMIT", "FILL", "SIGNAL", "SIGNAL_SKIP") or "SKIP" in ev or "ENTRY" in ev:
            signals.append(
                {
                    "ts": ts,
                    "event": ev,
                    "symbol": d.get("symbol"),
                    "strategy": d.get("strategy"),
                    "side": d.get("side"),
                    "reason": d.get("reason") or d.get("skip") or d.get("code"),
                    "entry": d.get("entry") or d.get("entry_price"),
                    "sl": d.get("sl"),
                    "tp": d.get("tp"),
                    "data": d,
                }
            )
    return audit, signals, mr09


def engine_nodes(cycles, skips, entries, oms, kill, mr09, audit_signals=None):
    """DAG mirroring live.rs::on_bar_close call order — no invented stages."""
    c = cycles[-1] if cycles else None
    regime = c["regime"] if c else "—"
    trade_ok = bool(c and not re.search(r"CHOP|NO_TRADE|OFF", str(regime), re.I))
    risk = (oms or {}).get("risk") or {}
    cool = (risk.get("cooldown_until") or 0) > int(_now() * 1000)
    audit_signals = audit_signals or []
    skip_n = len(skips or [])
    entry_n = len(entries or [])
    cycle_n = len(cycles or [])
    skip_reasons = {}
    for s in skips or []:
        r = str(s.get("reason") or "?")
        skip_reasons[r] = skip_reasons.get(r, 0) + 1
    last_skip = skips[-1] if skips else None
    last_entry = entries[-1] if entries else None
    last_mr = mr09[-1] if mr09 else None

    def ts_wib(ts):
        if ts is None:
            return ""
        try:
            if isinstance(ts, (int, float)):
                ms = int(ts) if ts > 1e12 else int(ts * 1000)
            else:
                s = str(ts).replace("Z", "+00:00")
                # leave formatting to client mostly; keep ISO short
                return str(ts)[:19].replace("T", " ") + " UTC"
            return str(ms)
        except Exception:
            return str(ts)[:19]

    stop_id = None
    stop_reason = None
    if last_skip:
        stop_reason = last_skip.get("reason")
        rr = str(stop_reason or "").upper()
        if "VOLUME" in rr:
            stop_id = "screener"
        elif "OFCC" in rr:
            stop_id = "ofcc"
        elif "KNN" in rr or "MAGNET" in rr:
            stop_id = "s1"  # S5 path collapsed; still strategy layer
        elif any(x in rr for x in ("SIZE", "NOTIONAL", "COOLDOWN", "DAILY", "MAX_POS", "REGIME", "ARMED", "BUSY", "SPREAD")):
            stop_id = "risk"
        else:
            stop_id = "risk"
    elif c and not trade_ok:
        stop_id = "market_reader"
        stop_reason = f"regime {regime}"

    def node(id_, label, ref, status, value, compact=False, reason=None, counters=None):
        st = status
        rs = None
        if stop_id == id_ and stop_reason:
            st = "block"
            rs = str(stop_reason)
        return {
            "id": id_,
            "label": label,
            "ref": ref,
            "status": st,
            "value": value,
            "detail": value,
            "compact": compact,
            "reason": rs or reason,
            "counters": counters or {},
            "tsWib": "",
        }

    nodes = [
        node("feed", "Data Feed", "live.rs::snapshot",
             "ok" if c else "idle",
             "REST public · M15/H1 bars",
             counters={"cycles": cycle_n}),
        node("market_reader", "Market Reader", "live.rs::decide",
             "ok" if trade_ok else ("block" if c else "idle"),
             f"regime={regime}" + (f" · long={c['long_ok']} short={c['short_ok']}" if c else ""),
             counters={"cycles": cycle_n}),
        node("mr09", "MR-09 Shadow", "live.rs::reversal_shadow",
             "ok" if last_mr else "idle",
             (f"up={fmt2(last_mr.get('rev_up'))} dn={fmt2(last_mr.get('rev_down'))}" if last_mr else "info only · no gate"),
             compact=True),
        node("screener", "Screener", "live.rs::on_bar_close + screener",
             "ok" if c else "idle",
             "allowlist · volume gate · SymState",
             counters={"skips_vol": skip_reasons.get("VOLUME_GATE", 0) + skip_reasons.get("SkipReason::VolumeGate", 0)}),
        node("s1", "S1_TFP V10", "strats.rs::s1_tfp + evaluate",
             "ok" if entry_n else ("warn" if skip_n else ("ok" if trade_ok else "idle")),
             "asia_ny · FVG/SRV · pivot20 · direct_limit",
             counters={"entries": entry_n, "skips": skip_n}),
        node("s2s5", "S2–S5 OFF", "cfg GO flags · strats.rs",
             "off",
             "LSR / SBR / WSL / RMP · NO-GO",
             compact=True),
        node("ofcc", "OFCC", "ofcc_watch.rs::observe",
             "idle",
             "S1 closed-bar confirm",
             counters={k: v for k, v in skip_reasons.items() if "OFCC" in k.upper()}),
        node("risk", "Risk Gate", "risk.rs::evaluate_entry",
             "warn" if cool or (risk.get("daily_sl_count") or 0) >= 6 else ("ok" if c else "idle"),
             f"SL {risk.get('daily_sl_count', 0)}/6 · day ${float(risk.get('daily_realized_net') or 0):.2f} · cool={'Y' if cool else 'N'}",
             counters={"open": len((oms or {}).get("trades") or {})}),
        node("oms", "OMS", "manager.rs::submit",
             "ok",
             f"MockExchange · seq={(oms or {}).get('seq', 0)} · pending={c.get('pending') if c else 0}"),
        node("kill", "Kill Switch", "killswitch.rs",
             "block" if kill else "ok",
             "ENGAGED" if kill else "OFF",
             compact=True),
        node("audit", "State / Audit", "AuditLog + StateStore",
             "ok" if c else "idle",
             "run.log · audit.jsonl · oms_state.json",
             compact=True),
    ]
    # Real edges (LR DAG). MR-09 is side info after decide. Kill constrains OMS.
    edges = [
        ("feed", "market_reader"),
        ("market_reader", "mr09"),
        ("market_reader", "screener"),
        ("screener", "s1"),
        ("screener", "s2s5"),
        ("s1", "ofcc"),
        ("ofcc", "risk"),
        ("risk", "oms"),
        ("kill", "oms"),
        ("oms", "audit"),
    ]
    pipeline = [
        "snapshot → decide(regime) → MR09 shadow",
        "screener(allow/volume/SymState) → evaluate(S1)",
        "OFCC watch → risk.evaluate_entry → OMS.submit",
        "killswitch gates OMS · audit/state writers",
    ]
    summary = {
        "cycle_n": cycle_n,
        "regime": regime,
        "long_ok": c.get("long_ok") if c else None,
        "short_ok": c.get("short_ok") if c else None,
        "max_pos": c.get("max_pos") if c else None,
        "equity": c.get("equity") if c else None,
        "open": c.get("open") if c else 0,
        "pending": c.get("pending") if c else 0,
        "stop_id": stop_id,
        "stop_reason": stop_reason,
        "last_cycle_ts": c.get("ts") if c else None,
    }
    return {"nodes": nodes, "edges": edges, "pipeline": pipeline, "summary": summary, "skip_reasons": skip_reasons}


def event_path_for(kind, reason=None, strategy=None):
    """Edges traversed for a real engine event (for pulse animation)."""
    kind = (kind or "").upper()
    reason = (reason or "").upper()
    base = ["feed>market_reader", "market_reader>screener"]
    if "MR09" in kind:
        return {"path": ["feed>market_reader", "market_reader>mr09"], "stop": "mr09", "kind": kind}
    if kind in ("BOOT",):
        return {"path": ["oms>audit"], "stop": "audit", "kind": kind}
    if kind == "CYCLE":
        path = base[:]
        stop = "screener"
        if reason and ("CHOP" in reason or "NO_TRADE" in reason or "OFF" in reason):
            stop = "market_reader"
            path = ["feed>market_reader"]
        return {"path": path, "stop": stop, "kind": kind, "reason": reason}
    if "SKIP" in kind:
        path = base + ["screener>s1", "s1>ofcc", "ofcc>risk"]
        stop = "risk"
        if "VOLUME" in reason:
            path = base
            stop = "screener"
        elif "OFCC" in reason:
            path = base + ["screener>s1", "s1>ofcc"]
            stop = "ofcc"
        elif any(x in reason for x in ("SIZE", "NOTIONAL", "COOLDOWN", "DAILY", "MAX_POS", "REGIME", "BUSY", "SPREAD", "ARMED")):
            stop = "risk"
        return {"path": path, "stop": stop, "kind": kind, "reason": reason, "strategy": strategy}
    if "ENTRY" in kind or kind in ("SUBMIT", "FILL"):
        path = base + ["screener>s1", "s1>ofcc", "ofcc>risk", "risk>oms", "oms>audit"]
        return {"path": path, "stop": "oms", "kind": kind, "strategy": strategy}
    if kind in ("CLOSE", "CLOSED", "SL", "TP"):
        return {"path": ["oms>audit"], "stop": "oms", "kind": kind}
    if kind == "S1_TRACE":
        path = base + ["screener>s1"]
        stop = "s1"
        if "VOLUME" in reason:
            path = base
            stop = "screener"
        elif "TREND" in reason:
            path = base + ["screener>s1"]
            stop = "screener"
        elif "OFCC" in reason:
            path = base + ["screener>s1", "s1>ofcc"]
            stop = "ofcc"
        return {"path": path, "stop": stop, "kind": kind, "reason": reason, "strategy": strategy or "S1_TFP"}
    return {"path": base, "stop": "market_reader", "kind": kind}


def build_engine_events(limit=80):
    """Ordered real events from run.log + audit.jsonl for SSE / replay."""
    events = []
    cycles, entries, skips, lines, err = parse_log()
    audit, audit_signals, mr09 = parse_audit(800)
    for i, c in enumerate(cycles):
        ev = {
            "id": f"cycle-{i}-{c.get('ts')}",
            "source": "run.log",
            "kind": "CYCLE",
            "ts": c.get("ts"),
            "regime": c.get("regime"),
            "long_ok": c.get("long_ok"),
            "short_ok": c.get("short_ok"),
            "equity": c.get("equity"),
            "summary": f"regime={c.get('regime')}",
        }
        ev.update(event_path_for("CYCLE", c.get("regime")))
        events.append(ev)
    for i, e in enumerate(entries):
        ev = {
            "id": f"entry-{i}-{e.get('ts')}-{e.get('symbol')}",
            "source": "run.log",
            "kind": "ENTRY",
            "ts": e.get("ts"),
            "symbol": e.get("symbol"),
            "strategy": e.get("strategy"),
            "side": e.get("side"),
            "summary": f"ENTRY {e.get('strategy')} {e.get('symbol')} {e.get('side')}",
        }
        ev.update(event_path_for("ENTRY", strategy=e.get("strategy")))
        events.append(ev)
    for i, e in enumerate(skips):
        ev = {
            "id": f"skip-{i}-{e.get('ts')}-{e.get('symbol')}-{e.get('reason')}",
            "source": "run.log",
            "kind": "SKIP",
            "ts": e.get("ts"),
            "symbol": e.get("symbol"),
            "strategy": e.get("strategy"),
            "side": e.get("side"),
            "reason": e.get("reason"),
            "summary": f"SKIP {e.get('reason')} {e.get('symbol')}",
        }
        ev.update(event_path_for("SKIP", e.get("reason"), e.get("strategy")))
        events.append(ev)
    for i, a in enumerate(audit):
        kind = str(a.get("event") or "")
        d = a.get("data") or {}
        if not kind:
            continue
        if kind not in ("BOOT", "MR09_SHADOW", "MR09_SHADOW_UNAVAILABLE", "S1_TRACE", "SKIP", "ENTRY", "SUBMIT", "FILL", "CLOSE", "CLOSED") and "SKIP" not in kind and "ENTRY" not in kind:
            # keep other audit sparsely: only known pulse-worthy
            if kind not in ("KILL", "ENGAGE_KILL"):
                continue
        mc = d.get("micro_context") if isinstance(d, dict) else None
        mc_str = ""
        if isinstance(mc, dict) and mc.get("poc") is not None:
            mc_str = f" · POC={mc.get('poc')} VAH={mc.get('vah')} VAL={mc.get('val')} Mark={mc.get('mark')}"
        reason_val = d.get("reason") or d.get("skip") or d.get("code") or d.get("stop_at")
        summary_str = kind + (f" {d.get('symbol')}" if d.get("symbol") else "")
        if kind == "S1_TRACE":
            summary_str = f"S1_TRACE {d.get('symbol') or ''} · stop={d.get('stop_at') or 'ok'}{mc_str}"
        ev = {
            "id": f"audit-{i}-{a.get('ts')}-{kind}-{d.get('symbol') or ''}",
            "source": "audit.jsonl",
            "kind": kind,
            "ts": a.get("ts"),
            "symbol": d.get("symbol"),
            "strategy": d.get("strategy") or ("S1_TFP" if kind == "S1_TRACE" else None),
            "reason": reason_val,
            "data": d if isinstance(d, dict) else {},
            "micro_context": mc if isinstance(mc, dict) else None,
            "summary": summary_str,
        }
        ev.update(event_path_for(kind, ev.get("reason"), ev.get("strategy")))
        events.append(ev)
    # sort by ts string / number best-effort, keep stable
    def key(ev):
        ts = ev.get("ts")
        if isinstance(ts, (int, float)):
            return (0, float(ts))
        if isinstance(ts, str) and ts:
            return (1, ts)
        return (2, "")
    events.sort(key=key)
    return events[-limit:]



def decision_chain(cycles, skips, entries, oms, mr09, audit_signals, audit=None):
    c = cycles[-1] if cycles else None
    risk = (oms or {}).get("risk") or {}
    cool_until = risk.get("cooldown_until") or 0
    cool = cool_until > int(_now() * 1000)
    last_skip = skips[-1] if skips else None
    last_entry = entries[-1] if entries else None
    last_sig = audit_signals[-1] if audit_signals else None
    traces = [a.get("data") or {} for a in (audit or []) if a.get("event") == "S1_TRACE" and isinstance(a.get("data"), dict)]
    eth_traces = [t for t in traces if t.get("symbol") == "ETHUSDT"]
    tr = eth_traces[-1] if eth_traces else (traces[-1] if traces else None)
    stop_reason = None
    if last_skip:
        stop_reason = last_skip.get("reason")
    elif last_sig and "SKIP" in str(last_sig.get("event", "")):
        stop_reason = last_sig.get("reason")
    elif tr and tr.get("stop_at"):
        stop_reason = f"S1_TRACE stop_at={tr.get('stop_at')} ({tr.get('symbol')})"

    def step(key, label, status, value, source):
        return {"key": key, "label": label, "status": status, "value": value, "source": source}

    steps = []
    if c:
        trade = not re.search(r"CHOP|NO_TRADE|OFF", c["regime"], re.I)
        steps.append(step("regime", "Market Reader · regime", "pass" if trade else "fail",
                          c["regime"], "run.log cycle"))
        steps.append(step("long_ok", "long_ok", "pass" if c["long_ok"] else "fail",
                          str(c["long_ok"]), "run.log cycle"))
        steps.append(step("short_ok", "short_ok", "pass" if c["short_ok"] else "fail",
                          str(c["short_ok"]), "run.log cycle"))
        steps.append(step("max_pos", "max_pos eng.", "pass", str(c["max_pos"]), "run.log cycle"))
    else:
        steps.append(step("regime", "Market Reader · regime", "unknown", "belum ada siklus", "run.log"))

    def unlogged(key, label, value, source):
        return step(key, label, "unlogged", value, source)

    if tr:
        sym = tr.get("symbol") or "ETHUSDT"
        sess_ok = bool(tr.get("session_ok"))
        steps.append(step("session", f"Session filter asia_ny ({sym})",
                          "pass" if sess_ok else "fail",
                          f"session_ok={sess_ok} · hour_utc={tr.get('session_hour_utc')}",
                          "audit.jsonl::S1_TRACE"))
        steps.append(step("chop", "CHOP filter (adx≤15, er≤0.25, trend_share≥15)",
                          "fail" if c and re.search(r"CHOP", c["regime"], re.I) else ("pass" if c else "unlogged"),
                          c["regime"] if c else "—", "regime label"))
        vg_ok = bool(tr.get("volume_gate_ok"))
        steps.append(step("volume_gate", f"Screener volume_gate ({sym})",
                          "pass" if vg_ok else "fail",
                          f"vg_ok={vg_ok} · fails={tr.get('volume_gate_fails')}",
                          "audit.jsonl::S1_TRACE"))
        t_ok = bool(tr.get("trend_ok"))
        steps.append(step("trend", f"Screener trend_ok ({sym})",
                          "pass" if t_ok else "fail",
                          f"trend_ok={t_ok} · trend_4h_ok={tr.get('trend_4h_ok')}",
                          "audit.jsonl::S1_TRACE"))
        srv_v = tr.get("srv")
        atr_v = tr.get("atr")
        srv_ok = srv_v is not None and float(srv_v) >= 0.15  # strategies.toml s1_srv_min
        steps.append(step("srv", f"SRV ≥ s1_srv_min + ATR finite ({sym})",
                          "pass" if srv_ok else "fail",
                          f"srv={fmt2(srv_v, 3)} · atr={fmt2(atr_v, 4)}",
                          "audit.jsonl::S1_TRACE"))
        ap_v = tr.get("atr_pctile")
        ap_ok = ap_v is not None  # band 0..100 (strategies.toml) = tidak memfilter
        steps.append(step("atr_pctile", f"ATR percentile band ({sym})",
                          "pass" if ap_ok else "fail",
                          f"atr_pctile={fmt2(ap_v, 1)}%",
                          "audit.jsonl::S1_TRACE"))
        fvg_v = tr.get("fvg_gap_atr")
        bf_v = tr.get("body_frac")
        md_v = tr.get("mid_delta")
        fvg_ok = fvg_v is not None and float(fvg_v) >= 0.20 and bf_v is not None and float(bf_v) >= 0.50 and md_v is not None and float(md_v) > 0  # s1_gap_atr=0.20, s1_body_frac=0.50
        steps.append(step("fvg", f"FVG gap ≥ s1_gap_atr·ATR + middle bar bullish ({sym})",
                          "pass" if fvg_ok else "fail",
                          f"gap_atr={fmt2(fvg_v, 3)} · body={fmt2(bf_v, 2)} · mid_delta={fmt2(md_v, 1)}",
                          "audit.jsonl::S1_TRACE"))
        sp = bool(tr.get("sell_push"))
        steps.append(step("direct", f"direct_limit: tolak sell_push ({sym})",
                          "fail" if sp else "pass",
                          f"sell_push={sp} · delta={fmt2(tr.get('delta'), 1)} · close_pos={fmt2(tr.get('close_pos'), 2)}",
                          "audit.jsonl::S1_TRACE"))
        sl_pct = tr.get("sl_pct")
        sl_ok = sl_pct is not None and 0.02 <= float(sl_pct) <= 0.04  # sl_min_pct / sl_max_pct
        steps.append(step("pivot", f"SL dari pivot lookback ({sym})",
                          "pass" if sl_ok else "fail",
                          f"pivot={fmt2(tr.get('pivot_price'), 4)} · sl={fmt2(tr.get('sl'), 4)} · sl_pct={(float(sl_pct)*100 if sl_pct is not None else 0):.2f}%",
                          "audit.jsonl::S1_TRACE"))
        ofcc_out = tr.get("ofcc_outcome") or "none"
        steps.append(step("ofcc", f"OFCC closed-bar confirm ({sym})",
                          "pass" if ofcc_out == "confirmed" else "idle",
                          f"outcome={ofcc_out} · seen={tr.get('ofcc_seen')} · rem={tr.get('ofcc_remaining_bars')}",
                          "audit.jsonl::S1_TRACE"))
        mc = tr.get("micro_context")
        if isinstance(mc, dict) and mc.get("poc") is not None:
            steps.append(step("micro_context", f"Orderflow & Value Area ({sym})",
                              "pass",
                              f"POC={mc.get('poc')} · VAH={mc.get('vah')} · VAL={mc.get('val')} · Mark={mc.get('mark')} · LargeTrades={mc.get('large_trades_count')}",
                              "audit.jsonl::S1_TRACE.micro_context"))
    else:
        steps.append(unlogged("session", "Session filter asia_ny (UTC <8 atau 13–21)",
                              "belum di-log (lolos/gagal diam)", "strats.rs::session_ok"))
        steps.append(step("chop", "CHOP filter (adx≤15, er≤0.25, trend_share≥15)",
                          "fail" if c and re.search(r"CHOP", c["regime"], re.I) else ("pass" if c else "unlogged"),
                          c["regime"] if c else "—", "regime label"))
        steps.append(unlogged("trend", "Screener trend_ok", "belum di-log", "lib.rs View.trend_ok / evaluate"))
        steps.append(unlogged("srv", "SRV ≥ s1_srv_min + ATR finite", "belum di-log", "strats.rs::s1_tfp"))
        steps.append(unlogged("atr_pctile", "ATR percentile band", "belum di-log", "strats.rs::s1_tfp atr_pctile"))
        steps.append(unlogged("fvg", "FVG gap ≥ s1_gap_atr·ATR + middle bar bullish", "belum di-log", "strats.rs::s1_tfp"))
        steps.append(unlogged("direct", "direct_limit: tolak sell_push (delta≤−P90 & close_pos≤0.25)",
                              "belum di-log", "strats.rs::sell_push"))
        steps.append(unlogged("pivot", "SL dari pivot lookback (2–4%)",
                              "belum di-log kecuali SKIP_SL_RANGE", "strats.rs::s1_tfp"))
        steps.append(unlogged("ofcc", "OFCC closed-bar confirm", "belum di-log", "ofcc_watch.rs::observe"))

    open_n = len((oms or {}).get("trades") or {})
    steps.append(step("maxpos", "Risk · max posisi ≤ 2",
                      "fail" if open_n >= 2 else "pass", f"open={open_n}", "oms_state"))
    steps.append(step("daily_sl", "Risk · daily SL ≤ 6",
                      "fail" if (risk.get("daily_sl_count") or 0) >= 6 else "pass",
                      f"{risk.get('daily_sl_count', 0)}/6", "oms_state.risk"))
    dn = float(risk.get("daily_realized_net") or 0)
    steps.append(step("daily_loss", "Risk · rugi harian ≤ $6",
                      "fail" if dn <= -6 else "pass", f"${dn:.2f}", "oms_state.risk"))
    steps.append(step("cooldown", "Risk · cooldown 1 jam setelah SL",
                      "fail" if cool else "pass",
                      f"until={cool_until}" if cool else "tidak aktif", "oms_state.risk"))
    if tr and tr.get("sizing_qty") is not None:
        sq = float(tr.get("sizing_qty") or 0)
        mn = float(tr.get("min_notional") or 0)
        nt = float(tr.get("notional") or 0)
        sz_ok = sq >= float(tr.get("min_qty") or 0) and nt >= mn
        steps.append(step("size", f"Sizing / minQty / minNotional ({tr.get('symbol')})",
                          "pass" if sz_ok else "fail",
                          f"qty={sq} (min={tr.get('min_qty')}) · notional=${nt:.2f} (min=${mn:.0f})",
                          "audit.jsonl::S1_TRACE"))
    else:
        steps.append(step("size", "Sizing / minQty / minNotional", "unknown",
                          "SKIP_SIZE hanya muncul saat gagal (BTC sering)", "risk gate"))

    if last_entry:
        steps.append(step("outcome", "Hasil siklus", "pass",
                          f"ENTRY {last_entry.get('strategy')} {last_entry.get('symbol')} {last_entry.get('side')}",
                          "run.log"))
    elif stop_reason:
        steps.append(step("outcome", "Rantai berhenti", "fail", f"{stop_reason}", "run.log/audit"))
    else:
        steps.append(step("outcome", "Hasil siklus", "idle",
                          "belum ada sinyal/skip di log (engine baru jalan)", "run.log"))

    return {
        "steps": steps,
        "stop_reason": stop_reason,
        "missing_logs": [] if tr else [
            "session_ok / session_hour_utc — strats.rs::session_ok (live.rs evaluate path)",
            "trend_ok / trend_4h_ok — strategies View (lib.rs) before s1_tfp",
            "volume_gate_ok / volume_gate_fails — live.rs VolumeGateTracker.update",
            "atr / srv / atr_pctile — strats.rs::s1_tfp (v.f15.*[t])",
            "fvg_gap_atr / body_frac / mid_delta — strats.rs::s1_tfp FVG block",
            "sell_push / delta / p90_absdelta / close_pos — strats.rs::sell_push",
            "pivot_price / sl / sl_pct — strats.rs::s1_tfp SL lookback",
            "ofcc_outcome / ofcc_seen / ofcc_remaining_bars — ofcc_watch.rs::observe",
            "sizing_qty / min_qty / min_notional / notional — live.rs SizingInput + risk.evaluate_entry",
        ],
    }


def state_payload():
    cached, fresh = cache_get("state_payload", 1.0)
    if fresh and isinstance(cached, dict):
        return cached
    cycles, entries, skips, lines, err = parse_log()
    audit, audit_signals, mr09 = parse_audit()
    oms = read_json(SDIR / "oms_state.json")
    kill = (ROOT / "state" / "KILL").exists()
    # merge signals for chart
    chart_signals = []
    for e in entries:
        chart_signals.append({**e, "taken": True, "source": "run.log"})
    for e in skips:
        chart_signals.append({**e, "taken": False, "source": "run.log"})
    for e in audit_signals:
        taken = "SKIP" not in str(e.get("event", "")).upper()
        chart_signals.append(
            {
                "kind": e.get("event"),
                "strategy": e.get("strategy"),
                "symbol": e.get("symbol"),
                "side": e.get("side"),
                "reason": e.get("reason"),
                "ts": e.get("ts"),
                "entry": e.get("entry"),
                "sl": e.get("sl"),
                "tp": e.get("tp"),
                "taken": taken,
                "source": "audit.jsonl",
            }
        )
    nodes = engine_nodes(cycles, skips, entries, oms, kill, mr09, audit_signals)
    chain = decision_chain(cycles, skips, entries, oms, mr09, audit_signals, audit)
    boot = next((a for a in audit if a.get("event") == "BOOT"), None)
    paper_start_ms = boot.get("ts") if boot else None
    is_alive = alive()
    out = {
        "now": int(_now() * 1000),
        "alive": is_alive,
        "kill": kill,
        "mode": "paper",
        "arm": True,
        "arm_paper": True,
        "arm_live": False,
        "engine_status": "PAPER LIVE (apex.exe aktif)" if is_alive else "ENGINE OFF · DASHBOARD AKTIF (apex.exe tidak berjalan)",
        "cycles": cycles[-500:],
        "entries": entries[-100:],
        "skips": skips[-100:],
        "signals": chart_signals[-200:],
        "audit": audit[-200:],
        "mr09": mr09[-100:],
        "log": lines[-120:],
        "errors": err[-20:],
        "oms": oms,
        "engine": nodes,
        "engine_events": build_engine_events(60),
        "chain": chain,
        "paper_start_ms": paper_start_ms,
        "v10": {
            "param_hash": V10_HASH,
            "strategy": "S1_TFP",
            "session": "asia_ny",
            "rr": 2.0,
            "pivot_lookback": 20,
            "direct_limit": True,
            "research_entry_5m": True,
            "status": "NO-GO / INCONCLUSIVE",
        },
    }
    cache_set("state_payload", out)
    return out


_KLINES_DISK = HERE / "cache_klines_last.json"
_KLINES_REFRESHING: set[str] = set()


def _load_klines_disk():
    try:
        if _KLINES_DISK.exists():
            raw = json.loads(_KLINES_DISK.read_text(encoding="utf-8"))
            if isinstance(raw, dict):
                for k, v in raw.items():
                    if isinstance(v, list) and v:
                        cache_set(k, v)
    except Exception:
        pass


def _save_klines_disk():
    try:
        with _lock:
            dump = {k: v[1] for k, v in _cache.items() if k.startswith("kl:") and isinstance(v[1], list) and v[1]}
        if dump:
            tmp = _KLINES_DISK.with_suffix(".tmp")
            tmp.write_text(json.dumps(dump), encoding="utf-8")
            tmp.replace(_KLINES_DISK)
    except Exception:
        pass


def _klines_from_csv(sym: str, tf: str, limit: int = 500) -> list:
    """Fallback klines reader/aggregator from local data/klines/{sym}_{5m,15m,1h}.csv."""
    sym = sym.upper()
    kl_dir = ROOT / "data" / "klines"
    data_dir = kl_dir if kl_dir.exists() else (ROOT / "data")
    src_5m = data_dir / f"{sym}_5m.csv"
    src_1h = data_dir / f"{sym}_1h.csv"
    src_15m = data_dir / f"{sym}_15m.csv"
    if tf == "5m" and src_5m.exists():
        src = src_5m
    elif tf in ("1h", "4h", "1d") and src_1h.exists():
        src = src_1h
    else:
        src = src_15m if src_15m.exists() else (src_1h if src_1h.exists() else (src_5m if src_5m.exists() else None))
    if not src:
        return []
    try:
        need_raw = limit * (24 if tf == "1d" else (4 if tf == "4h" else 1)) + 50
        lines = tail_lines(src, max(600, need_raw))
        rows = []
        for ln in lines:
            parts = ln.strip().split(",")
            if len(parts) < 6 or not parts[0].strip().isdigit():
                continue
            t_sec = int(parts[0]) // 1000
            rows.append({
                "time": t_sec,
                "open": float(parts[1]),
                "high": float(parts[2]),
                "low": float(parts[3]),
                "close": float(parts[4]),
                "volume": float(parts[5]),
            })
        if not rows:
            return []
        if tf in ("4h", "1d"):
            bucket_sec = 14400 if tf == "4h" else 86400
            agg = {}
            for r in rows:
                b = (r["time"] // bucket_sec) * bucket_sec
                if b not in agg:
                    agg[b] = dict(r, time=b)
                else:
                    cell = agg[b]
                    if r["high"] > cell["high"]:
                        cell["high"] = r["high"]
                    if r["low"] < cell["low"]:
                        cell["low"] = r["low"]
                    cell["close"] = r["close"]
                    cell["volume"] += r["volume"]
            rows = [agg[k] for k in sorted(agg.keys())]
        return rows[-limit:]
    except Exception:
        return []


def _fetch_klines_sync(sym: str, tf: str, limit: int = 500) -> list:
    key = f"kl:{sym}:{tf}:{limit}"
    raw = binance_get(f"/fapi/v1/klines?symbol={sym}&interval={tf}&limit={limit}", ttl=45)
    if isinstance(raw, list) and raw:
        out = [
            {
                "time": int(k[0]) // 1000,
                "open": float(k[1]),
                "high": float(k[2]),
                "low": float(k[3]),
                "close": float(k[4]),
                "volume": float(k[5]),
            }
            for k in raw
        ]
        cache_set(key, out)
        return out
    return []


def klines(sym: str, tf: str, limit: int = 500):
    sym = sym.upper()
    if not valid_symbol(sym) or tf not in {"1m", "5m", "15m", "1h", "4h", "1d"}:
        return []
    key = f"kl:{sym}:{tf}:{limit}"
    data, fresh = cache_get(key, 45)
    if fresh and data:
        return data
    if data:
        # Stale-while-revalidate in background thread so HTTP returns in <5ms
        if key not in _KLINES_REFRESHING:
            _KLINES_REFRESHING.add(key)
            def _bg():
                try:
                    _fetch_klines_sync(sym, tf, limit)
                finally:
                    _KLINES_REFRESHING.discard(key)
            threading.Thread(target=_bg, daemon=True).start()
        return data
    out = _fetch_klines_sync(sym, tf, limit)
    if out:
        return out
    csv_fallback = _klines_from_csv(sym, tf, limit)
    if csv_fallback:
        cache_set(key, csv_fallback)
        return csv_fallback
    return []


def live(sym: str, tf: str):
    sym = (sym or "ETHUSDT").upper()
    tf = tf or "15m"
    key = f"live:{sym}:{tf}"
    data, fresh = cache_get(key, 2.0)
    if fresh:
        return data
    bars = klines(sym, tf, limit=3)
    last2 = bars[-2:] if bars else []
    tickers = {}
    for s in ("BTCUSDT", "ETHUSDT"):
        t = binance_get(f"/fapi/v1/ticker/24hr?symbol={s}", ttl=5)
        p = binance_get(f"/fapi/v1/premiumIndex?symbol={s}", ttl=5)
        price = None
        pct = None
        mark = None
        if isinstance(t, dict):
            try:
                price = float(t.get("lastPrice") or t.get("c") or 0)
                pct = float(t.get("priceChangePercent") or 0)
            except (TypeError, ValueError):
                pass
        if isinstance(p, dict):
            try:
                mark = float(p.get("markPrice") or 0)
            except (TypeError, ValueError):
                pass
        tickers[s] = {"last": price, "mark": mark, "pct24h": pct}
    out = {
        "ts": int(_now() * 1000),
        "symbol": sym,
        "interval": tf,
        "bars": last2,
        "tickers": tickers,
        "ban_until": int(_ban_until * 1000) if _ban_until > _now() else 0,
    }
    cache_set(key, out)
    return out


def parse_research():
    cached, fresh = cache_get("parse_research", 30.0)
    if fresh and isinstance(cached, dict):
        return cached
    # Prioritas: laporan Curated Liquid Majors terbaru, fallback ke BTC/ETH lama.
    _cm_dir = ROOT / "reports" / "curated_majors_val"
    vs = read_json(_cm_dir / "validation_summary.json") or {}
    _use_cm = bool(vs.get("gates"))
    if not _use_cm:
        vs = read_json(ROOT / "reports" / "btceth_validate" / "validation_summary.json") or {}
    _gates = vs.get("gates")
    if isinstance(_gates, list):
        gates = _gates[0] if _gates and isinstance(_gates[0], dict) else {}
    elif isinstance(_gates, dict):
        gates = _gates
    else:
        gates = {}
    test_j = None
    test_dir = ROOT / "reports" / "btceth_test"
    if test_dir.exists():
        for p in test_dir.glob("bt_S1_TFP_*.json"):
            test_j = read_json(p)
            break
    v10_j = read_json(ROOT / "reports" / "btceth_trials" / f"bt_S1_TFP_{V10_HASH}.json")
    equity = []
    eq_path = ROOT / "reports" / "curated_majors_equity_curve.csv"
    if not (_use_cm and eq_path.exists()):
        eq_path = ROOT / "reports" / "btceth_equity_curve.csv"
    if eq_path.exists():
        try:
            with eq_path.open("r", encoding="utf-8", errors="replace") as f:
                hdr = f.readline().strip().split(",")
                for line in f:
                    parts = line.strip().split(",")
                    if len(parts) < len(hdr):
                        continue
                    row = dict(zip(hdr, parts))
                    equity.append(row)
        except OSError:
            pass
    trials = []
    for path in [
        ROOT / "reports" / "btceth_trials" / "trials_ledger.jsonl",
        ROOT / "reports" / "trials_ledger.jsonl",
    ]:
        if not path.exists():
            continue
        for l in tail_lines(path, 500):
            try:
                trials.append(json.loads(l))
            except ValueError:
                pass
    # segments from gates + test
    is_row = {
        "seg": "IS",
        "range": "2025-01-01 .. 2025-12-01",
        "trades": gates.get("is_trades"),
        "e": None,
        "pf": None,
    }
    # try from fit plateau / report numbers in gates
    val_row = {
        "seg": "VAL",
        "range": "2025-12-01 .. 2026-07-01",
        "trades": gates.get("val_trades"),
        "e": gates.get("val_e"),
        "pf": gates.get("val_pf"),
    }
    test_m = {} if _use_cm else ((test_j or {}).get("metrics") or {})  # holdout universe baru = LOCKED (laporan TEST lama = BTC/ETH saja)
    test_row = {
        "seg": "TEST",
        "range": "2026-07-01 .. 2026-11-04",
        "trades": test_m.get("trades"),
        "e": test_m.get("expectancy_r"),
        "pf": test_m.get("profit_factor"),
        "wr": test_m.get("win_rate"),
        "net_usd": test_m.get("net_usd"),
        "max_dd_r": test_m.get("max_dd_r"),
    }
    # IS E/PF from equity curve or v10 metrics if available
    if equity:
        is_trades = [r for r in equity if r.get("split") == "IS"]
        if is_trades:
            rs = [float(r["r_mult"]) for r in is_trades if r.get("r_mult") not in (None, "")]
            wins = sum(1 for r in is_trades if float(r.get("pnl_net_usd") or 0) > 0)
            gp = sum(float(r["pnl_net_usd"]) for r in is_trades if float(r.get("pnl_net_usd") or 0) > 0)
            gl = abs(sum(float(r["pnl_net_usd"]) for r in is_trades if float(r.get("pnl_net_usd") or 0) < 0))
            is_row["trades"] = len(is_trades)
            is_row["e"] = (sum(rs) / len(rs)) if rs else None
            is_row["pf"] = (gp / gl) if gl > 0 else None
            is_row["wr"] = wins / len(is_trades) if is_trades else None
    gate_list = []
    # Map known gates from gates object + REPORT conventions
    def g(name, passed, detail):
        gate_list.append({"id": name, "pass": passed, "detail": detail})

    g("G-01 data/replay", bool(gates.get("g01_verified")),
      f"gaps={gates.get('data_gaps')} · checksum+prefix replay verified={gates.get('g01_verified')}")
    _is_n = gates.get("is_trades") or 0
    _val_n = gates.get("val_trades") or 0
    g("G-02 sampel", bool(_is_n >= 100 and _val_n >= 60),
      f"IS trades={gates.get('is_trades')} (butuh ≥100), VAL={gates.get('val_trades')} (butuh ≥60)")
    g("G-06 walk-forward", bool(gates.get("wf_raw_pass") or gates.get("wf_pass")),
      f"positif {gates.get('wf_positive')}/6 · worst E={gates.get('wf_worst')} · raw_pass={gates.get('wf_raw_pass')}")
    g("G-07 Monte Carlo", float(gates.get("mc_p95_dd_r") or 99) <= 15 and float(gates.get("mc_p_below_floor") or 1) <= 0.05,
      f"p95 DD={gates.get('mc_p95_dd_r')}R · P(<$40)={gates.get('mc_p_below_floor')}")
    g("G-08 DSR", float(gates.get("dsr") or 0) >= 0.95,
      f"DSR={gates.get('dsr')} (N trials={gates.get('n_trials')})")
    g("G-09 plateau", bool(gates.get("g09_raw_pass") or gates.get("g09_pass")),
      f"positive_frac={gates.get('g09_positive_frac')} median={gates.get('g09_median')}")
    g("G-10 konsentrasi", bool(gates.get("g10_raw_pass")),
      f"max_month_share={gates.get('max_month_profit_share')} max_symbol_share={gates.get('max_symbol_profit_share')}")
    g("G-11 holdout/TEST", False, f"g11={gates.get('g11')} · TEST E={test_row.get('e')} PF={test_row.get('pf')} (INCONCLUSIVE)")
    g("Survive MC/bulan", True, f"maxDD={gates.get('max_dd_r')}R / ${gates.get('max_dd_usd')} · net=${gates.get('net_usd')}")

    verdict = "INCONCLUSIVE / NO-GO"
    report_head = ""
    rp = ROOT / "REPORT_BTCETH.md"
    if rp.exists():
        try:
            report_head = rp.read_text(encoding="utf-8", errors="replace")[:2500]
        except OSError:
            pass
    folds = gates.get("folds") or []
    _res_out = {
        "v10_hash": V10_HASH,
        "verdict": verdict,
        "segments": [is_row, val_row, test_row],
        "gates": gate_list,
        "folds": folds,
        "mc": {
            "p95_dd_r": gates.get("mc_p95_dd_r"),
            "p_below_floor": gates.get("mc_p_below_floor"),
            "paths": gates.get("mc_paths"),
            "per_year": gates.get("mc_per_year"),
        },
        "dsr": gates.get("dsr"),
        "metrics": {
            "e": gates.get("e"),
            "pf": gates.get("pf"),
            "trades": gates.get("trades"),
            "win_rate": gates.get("win_rate"),
            "max_dd_r": gates.get("max_dd_r"),
            "max_dd_usd": gates.get("max_dd_usd"),
            "net_usd": gates.get("net_usd"),
        },
        "equity": equity,
        "trials": trials[-100:],
        "report_excerpt": report_head,
        "combo_regime": vs.get("combo_regime"),
        "test_metrics": test_m,
        "v10_trade_count": len((v10_j or {}).get("trades") or []),
    }
    cache_set("parse_research", _res_out)
    return _res_out



# ---------------------------------------------------------------------------
# Vike.io proxy (server-side only). Key from os.environ["VIKE_API_KEY"] only.
# Never log, echo, or return the key. Docs: https://data.vike.io/v1
# Auth header: X-API-KEY (same as apex-trading-system/data/vike_client.py).
# ---------------------------------------------------------------------------
VIKE_BASE = "https://data.vike.io/v1"
_vike_ban_until = 0.0
_vike_backoff = 30.0


def vike_configured():
    k = os.environ.get("VIKE_API_KEY")
    return bool(k and str(k).strip())


def vike_key_meta():
    """Presence + length only — never the value."""
    k = os.environ.get("VIKE_API_KEY") or ""
    k = str(k).strip()
    return {"configured": bool(k), "key_length": len(k) if k else 0}


def vike_coin(symbol: str) -> str:
    s = (symbol or "ETH").upper()
    for suf in ("USDT", "USDC", "USD", "PERP"):
        if s.endswith(suf):
            s = s[: -len(suf)]
            break
    return s or "ETH"


def vike_get(path: str, ttl: float = 60.0, params: dict | None = None):
    """Authenticated GET to data.vike.io. Returns (payload_dict, http_status).
    payload never includes the API key. On missing key -> status 0 + error.
    """
    global _vike_ban_until, _vike_backoff
    meta = vike_key_meta()
    if not meta["configured"]:
        return {
            "ok": False,
            "error": "key_missing",
            "message": "VIKE: key belum diset",
            "configured": False,
            "http": None,
        }, 0

    qs = urllib.parse.urlencode(params or {})
    cache_key = f"vike:{path}?{qs}"
    data, fresh = cache_get(cache_key, ttl)
    if fresh and isinstance(data, dict) and data.get("_cached_ok"):
        out = {k: v for k, v in data.items() if k != "_cached_ok"}
        out["cached"] = True
        return out, int(data.get("http") or 200)

    if _now() < _vike_ban_until:
        if isinstance(data, dict) and data.get("ok") is False:
            stale = {k: v for k, v in data.items() if k != "_cached_ok"}
            stale["cached"] = True
            stale["backoff"] = True
            return stale, int(data.get("http") or 0)
        return {
            "ok": False,
            "error": "backoff",
            "message": "VIKE: backoff setelah error",
            "configured": True,
            "http": None,
            "cached": bool(data),
        }, 0

    key = (os.environ.get("VIKE_API_KEY") or "").strip()
    url = VIKE_BASE.rstrip("/") + "/" + path.lstrip("/")
    if qs:
        url = url + ("&" if "?" in url else "?") + qs
    try:
        req = urllib.request.Request(
            url,
            headers={
                "User-Agent": "ApexDash/1.0",
                "Accept": "application/json",
                "X-API-KEY": key,
            },
        )
        with urllib.request.urlopen(req, timeout=20) as r:
            body = r.read().decode("utf-8", errors="replace")
            code = getattr(r, "status", 200) or 200
            try:
                raw = json.loads(body)
            except ValueError:
                raw = {"raw_type": "non_json", "bytes": len(body)}
        shape = sorted(list(raw.keys()))[:24] if isinstance(raw, dict) else type(raw).__name__
        out = {
            "ok": True,
            "configured": True,
            "http": code,
            "path": path,
            "shape": shape,
            "data": raw,
            "cached": False,
        }
        cache_set(cache_key, {**out, "_cached_ok": True})
        _vike_backoff = 30.0
        return out, code
    except urllib.error.HTTPError as e:
        code = e.code
        # Never include response body verbatim if it might echo auth; keep shape only.
        body_peek = ""
        detail = None
        try:
            body_peek = e.read().decode("utf-8", errors="replace")[:400]
            try:
                ej = json.loads(body_peek)
                detail = ej.get("detail") or (ej.get("error") or {}).get("code") or ej.get("error")
            except Exception:
                detail = "non_json"
        except Exception:
            detail = None
        if code in (401, 403):
            msg = "VIKE: 401 unauthorized" if code == 401 else "VIKE: 403 forbidden"
            low = (body_peek or "").lower()
            if code == 403 and ("cloudflare" in low or detail == "non_json" or "<html" in low):
                msg = "VIKE: 403 diblokir Cloudflare"
            err = {
                "ok": False,
                "configured": True,
                "http": code,
                "error": "forbidden" if code == 403 else "unauthorized",
                "message": msg,
                "detail_type": type(detail).__name__ if detail is not None else None,
            }
            _vike_ban_until = _now() + max(60.0, _vike_backoff)
            _vike_backoff = min(600.0, _vike_backoff * 2)
            cache_set(cache_key, {**err, "_cached_ok": False})
            return err, code
        if code == 429:
            retry = 60.0
            try:
                ra = e.headers.get("Retry-After")
                if ra:
                    retry = float(ra)
            except Exception:
                pass
            _vike_ban_until = _now() + max(30.0, retry)
            err = {
                "ok": False,
                "configured": True,
                "http": 429,
                "error": "rate_limited",
                "message": "VIKE: 429 rate limited",
            }
            cache_set(cache_key, {**err, "_cached_ok": False})
            return err, 429
        err = {
            "ok": False,
            "configured": True,
            "http": code,
            "error": f"http_{code}",
            "message": f"VIKE: HTTP {code}",
        }
        cache_set(cache_key, {**err, "_cached_ok": False})
        return err, code
    except Exception as e:
        err = {
            "ok": False,
            "configured": True,
            "http": None,
            "error": "network",
            "message": "VIKE: network error",
            "exc_type": type(e).__name__,
        }
        _vike_ban_until = _now() + 30.0
        return err, 0


def vike_status_payload():
    meta = vike_key_meta()
    out = {
        "configured": meta["configured"],
        "key_length": meta["key_length"],
        "base": VIKE_BASE,
        "auth_header": "X-API-KEY",
        "endpoints": {
            "liquidation_heatmap": "/v1/hyperliquid/symbols/{symbol}/liquidation-heatmap",
            "liquidations": "/v1/hyperliquid/liquidations",
        },
    }
    if not meta["configured"]:
        out["message"] = "VIKE: key belum diset"
        out["http"] = None
        return out
    # light probe: small bins heatmap for ETH (cached 90s)
    probe, code = vike_get(
        f"hyperliquid/symbols/{vike_coin('ETHUSDT')}/liquidation-heatmap",
        ttl=90.0,
        params={"bins": 10},
    )
    out["http"] = probe.get("http") if isinstance(probe, dict) else code
    out["ok"] = bool(probe.get("ok")) if isinstance(probe, dict) else False
    out["message"] = probe.get("message") if isinstance(probe, dict) else None
    out["shape"] = probe.get("shape") if isinstance(probe, dict) else None
    if out["ok"] and isinstance(probe.get("data"), dict):
        d = probe["data"]
        out["probe"] = {
            "symbol": d.get("symbol"),
            "mark_price": d.get("mark_price"),
            "bins": len(d.get("bins") or []),
            "snapshot_ts": d.get("snapshot_ts"),
            "keys": sorted(list(d.keys()))[:20],
        }
    return out


def vike_liquidation_heatmap(symbol: str, bins: int = 80):
    coin = vike_coin(symbol)
    bins = max(5, min(500, int(bins or 80)))
    payload, code = vike_get(
        f"hyperliquid/symbols/{coin}/liquidation-heatmap",
        ttl=60.0,
        params={"bins": bins},
    )
    if not payload.get("ok"):
        return payload
    raw = payload.get("data") or {}
    bins_rows = raw.get("bins") or []
    # Normalize for UI (price ladder + long/short split). No invented values.
    norm = []
    for b in bins_rows:
        if not isinstance(b, dict):
            continue
        try:
            norm.append({
                "price_start": float(b.get("price_bin_start")),
                "price_end": float(b.get("price_bin_end")),
                "price_mid": (float(b.get("price_bin_start")) + float(b.get("price_bin_end"))) / 2.0,
                "liq_usd": float(b.get("liquidation_value_usd") or 0),
                "long_usd": float(b.get("long_value_usd") or 0),
                "short_usd": float(b.get("short_value_usd") or 0),
                "positions": int(b.get("positions_count") or 0),
            })
        except (TypeError, ValueError):
            continue
    return {
        "ok": True,
        "configured": True,
        "http": payload.get("http") or code,
        "cached": payload.get("cached", False),
        "symbol": raw.get("symbol") or coin,
        "mark_price": raw.get("mark_price"),
        "snapshot_ts": raw.get("snapshot_ts"),
        "total_value_usd": raw.get("total_value_usd"),
        "window": raw.get("window"),
        "coverage": raw.get("coverage"),
        "bins": norm,
        "source": "vike:/hyperliquid/symbols/{symbol}/liquidation-heatmap",
    }


def vike_liquidations_feed(symbol: str, limit: int = 500):
    """Realized liquidations for price×time heatmap (not invented)."""
    coin = vike_coin(symbol)
    limit = max(10, min(2000, int(limit or 500)))
    payload, code = vike_get(
        "hyperliquid/liquidations",
        ttl=60.0,
        params={"symbol": coin, "limit": limit},
    )
    if not payload.get("ok"):
        return payload
    raw = payload.get("data") or {}
    rows = raw.get("liquidations") if isinstance(raw, dict) else None
    if rows is None and isinstance(raw, list):
        rows = raw
    rows = rows or []
    out_rows = []
    for r in rows:
        if not isinstance(r, dict):
            continue
        ts = r.get("liq_ts") or r.get("ts")
        # epoch ms
        tms = None
        if isinstance(ts, (int, float)):
            tms = int(ts if ts > 1e12 else ts * 1000)
        elif isinstance(ts, str):
            try:
                # ISO
                tms = int(datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp() * 1000)
            except Exception:
                tms = None
        try:
            px = float(r.get("mark_price") or r.get("price") or 0)
            usd = float(r.get("size_usd") or 0)
        except (TypeError, ValueError):
            continue
        if not tms or px <= 0:
            continue
        direction = r.get("direction") or r.get("side") or ""
        out_rows.append({
            "t": tms,
            "price": px,
            "usd": usd,
            "direction": direction,
        })
    return {
        "ok": True,
        "configured": True,
        "http": payload.get("http") or code,
        "cached": payload.get("cached", False),
        "symbol": coin,
        "rows": out_rows,
        "count": len(out_rows),
        "source": "vike:/hyperliquid/liquidations",
    }




_HEATMAP_LAST_GOOD = {"items": [], "ts": 0}
_HEATMAP_DISK = Path(__file__).resolve().parent / "cache_heatmap_last.json"


def _load_heatmap_disk():
    global _HEATMAP_LAST_GOOD
    try:
        if _HEATMAP_DISK.exists():
            raw = json.loads(_HEATMAP_DISK.read_text(encoding="utf-8"))
            if isinstance(raw, dict) and isinstance(raw.get("items"), list) and raw["items"]:
                _HEATMAP_LAST_GOOD = {"items": raw["items"], "ts": int(raw.get("ts") or 0)}
    except Exception:
        pass


def _save_heatmap_disk():
    try:
        if _HEATMAP_LAST_GOOD.get("items"):
            _HEATMAP_DISK.write_text(json.dumps(_HEATMAP_LAST_GOOD), encoding="utf-8")
    except Exception:
        pass



def heatmap_market():
    """USDT-M treemap from 24hr tickers. Never poison cache with empty; keep last good."""
    global _HEATMAP_LAST_GOOD, _ban_until
    path = "/fapi/v1/ticker/24hr"
    # Bust poisoned empty cache entry if present
    with _lock:
        hit = _cache.get("bn:" + path)
        if hit and isinstance(hit[1], list) and len(hit[1]) == 0:
            _cache.pop("bn:" + path, None)
        # also clear non-list poison
        if hit and hit[1] is not None and not isinstance(hit[1], list):
            _cache.pop("bn:" + path, None)

    ban_active = _now() < _ban_until
    raw = binance_get(path, ttl=45)
    err = None
    if ban_active and not (isinstance(raw, list) and raw):
        err = f"BINANCE backoff aktif hingga {int(_ban_until)} (418/429)"
    if raw is None:
        err = err or "BINANCE ticker/24hr gagal (null)"
    elif not isinstance(raw, list):
        err = err or f"BINANCE ticker/24hr shape={type(raw).__name__}"
    elif len(raw) == 0:
        err = err or "BINANCE ticker/24hr kosong"

    items = []
    if isinstance(raw, list) and raw:
        for row in raw:
            if not isinstance(row, dict):
                continue
            sym = row.get("symbol") or ""
            if not sym.endswith("USDT"):
                continue
            if any(x in sym for x in ("USDC", "BUSD", "TUSD", "EUR", "TRY", "BRL")):
                continue
            try:
                qv = float(row.get("quoteVolume") or 0)
                pct = float(row.get("priceChangePercent") or 0)
                last = float(row.get("lastPrice") or 0)
            except (TypeError, ValueError):
                continue
            if qv < 5_000_000:
                continue
            items.append({"symbol": sym, "pct": pct, "quoteVolume": qv, "last": last})
        items.sort(key=lambda x: -x["quoteVolume"])
        items = items[:80]
        if items:
            _HEATMAP_LAST_GOOD = {"items": items, "ts": int(_now() * 1000)}
            _save_heatmap_disk()
            return {
                "items": items,
                "ts": _HEATMAP_LAST_GOOD["ts"],
                "source": "BINANCE",
                "count_raw": len(raw),
                "stale": False,
            }

    # Serve last good with honest error
    good = _HEATMAP_LAST_GOOD.get("items") or []
    return {
        "items": good,
        "ts": _HEATMAP_LAST_GOOD.get("ts") or int(_now() * 1000),
        "source": "BINANCE",
        "stale": bool(good),
        "error": err or "tidak ada items setelah filter",
        "message": (err or "HEATMAP kosong") + ((" · serving last-good n="+str(len(good))) if good else ""),
        "ban_until": int(_ban_until * 1000) if _ban_until > _now() else 0,
    }


def market_metrics():
    cached, fresh = cache_get("market_metrics", 20.0)
    if fresh and isinstance(cached, dict):
        return dict(cached)
    out = {"btc": {}, "eth": {}, "ts": int(_now() * 1000)}
    for name, sym in (("btc", "BTCUSDT"), ("eth", "ETHUSDT")):
        prem = binance_get(f"/fapi/v1/premiumIndex?symbol={sym}", ttl=60) or {}
        oi = binance_get(f"/fapi/v1/openInterest?symbol={sym}", ttl=60) or {}
        # global long/short account ratio
        lsr = binance_get(
            f"/futures/data/globalLongShortAccountRatio?symbol={sym}&period=1h&limit=1", ttl=60
        )
        taker = binance_get(
            f"/futures/data/takerlongshortRatio?symbol={sym}&period=1h&limit=1", ttl=60
        )
        out[name] = {
            "mark": float(prem.get("markPrice") or 0) if isinstance(prem, dict) else None,
            "index": float(prem.get("indexPrice") or 0) if isinstance(prem, dict) else None,
            "funding": float(prem.get("lastFundingRate") or 0) if isinstance(prem, dict) else None,
            "next_funding": prem.get("nextFundingTime") if isinstance(prem, dict) else None,
            "oi": float(oi.get("openInterest") or 0) if isinstance(oi, dict) else None,
            "long_short": (lsr[-1] if isinstance(lsr, list) and lsr else lsr),
            "taker": (taker[-1] if isinstance(taker, list) and taker else taker),
        }
    cache_set("market_metrics", out)
    return dict(out)


def learning_payload():
    cached, fresh = cache_get("learning_payload", 3.0)
    if fresh and isinstance(cached, dict):
        return cached
    cycles, entries, skips, lines, err = parse_log()
    audit, audit_signals, mr09 = parse_audit(800)
    oms = read_json(SDIR / "oms_state.json") or {}
    closed = list(oms.get("closed") or [])
    open_t = list((oms.get("trades") or {}).values())
    research = parse_research()
    boot = next((a for a in audit if a.get("event") == "BOOT"), None)
    start_ms = boot.get("ts") if boot else None
    days = 0.0
    if start_ms:
        days = max(0.0, (_now() * 1000 - float(start_ms)) / 86400000.0)
    # skip stats
    reason_count: dict = {}
    for s in skips:
        r = s.get("reason") or "?"
        reason_count[r] = reason_count.get(r, 0) + 1
    for s in audit_signals:
        if "SKIP" in str(s.get("event", "")).upper():
            r = s.get("reason") or "?"
            reason_count[r] = reason_count.get(r, 0) + 1
    notes = []
    if not closed and not open_t:
        notes.append("Paper run baru dimulai: belum ada trade. Pantau sinyal & skip 4 minggu ke depan.")
    if skips or any("SKIP" in str(s.get("event", "")) for s in audit_signals):
        notes.append("Ada sinyal yang di-skip — pelajari alasan teratas di statistik skip.")
    else:
        notes.append("Belum ada sinyal/skip di log. Keputusan filter di dalam S1 sering diam (tidak di-log).")
    notes.append(
        f"Backtest V10: IS/VAL/TEST dari laporan — verdict {research.get('verdict')}. "
        "Paper clock 15m ≠ riset 5m entry."
    )
    glossary = [
        {"id": "mr", "title": "Market Reader", "body": "Mengklasifikasi regime global (RISK_ON_ALTS, BTC_LED, RISK_OFF, CHOP_NO_TRADE, dsb.) dari BTC/ETH/dominance. Menghasilkan long_ok/short_ok/max_pos tiap siklus."},
        {"id": "chop", "title": "CHOP Filter", "body": "Ambang default adx_max=15, er_max=0.25, trend_share_min=15. Saat sideways, regime CHOP_NO_TRADE — dilarang entry."},
        {"id": "s1", "title": "S1_TFP (Trend FVG Pullback)", "body": "Mencari Fair Value Gap bullish 3-bar di timeframe entry, filter sesi, SRV, lalu SL dari pivot lookback. V10: asia_ny, pivot=20, direct_limit, RR=2, riset 5m."},
        {"id": "ofcc", "title": "OFCC", "body": "Konfirmasi order-flow closed-bar sebelum limit S1 diaktifkan. Outcome: Waiting / Confirmed / Rejected / Expired."},
        {"id": "mr09", "title": "MR-09 Shadow", "body": "Skor reversal multi-TF (rev_up/rev_down) + konteks Fib/struktur. Shadow saja — tidak mengeksekusi order."},
        {"id": "risk", "title": "Risk Gate / Golden Rules", "body": "$60 start, $1 risk/trade, SL 2–4%, 25x cross, max 2 posisi, daily 6 SL / −$6, cooldown 1 jam setelah SL, equity floor $40."},
        {"id": "oms", "title": "OMS + Kill Switch", "body": "Paper memakai MockExchange. File state/KILL menghentikan aktivitas. arm=false — tidak ada order ke Binance."},
        {"id": "gates", "title": "Gates G-01..G-11", "body": "Gerbang validasi backtest (sampel, E/PF, walk-forward, Monte Carlo, DSR, plateau, konsentrasi, holdout). Gagal/INCONCLUSIVE = NO-GO."},
    ]
    _learn_out = {
        "journal": closed,
        "open": open_t,
        "skips": skips[-100:],
        "skip_reasons": reason_count,
        "signals": (entries + skips)[-100:],
        "days_elapsed": days,
        "days_target": 28,
        "paper_trades": len(closed),
        "notes": notes,
        "glossary": glossary,
        "vs_backtest": {
            "v10_segments": research.get("segments"),
            "verdict": research.get("verdict"),
            "test": research.get("test_metrics"),
        },
        "chain_missing": (state_payload().get("chain") or {}).get("missing_logs"),
        "pattern": {
            "by_hour": {},
            "by_session": {},
            "by_regime": {},
            "by_side": {},
            "taken_vs_skip": {"taken": len(entries), "skipped": len(skips)},
        },
    }
    cache_set("learning_payload", _learn_out)
    return _learn_out


def library_docs():
    names = [
        "REPORT_BTCETH.md", "REPORT_MERGE.md", "REPORT_CODEX.md", "REPORT_GROK.md",
        "REPORT_HERMES.md", "REPORT_INTEGRATION_REVIEW.md", "UNDERSTANDING.md",
        "AGENTS.md", "CHANGELOG.md", "README.md", "TASKS.md",
    ]
    docs = []
    for n in names:
        p = ROOT / n
        if p.exists():
            try:
                txt = p.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            docs.append({"name": n, "path": str(p), "chars": len(txt), "preview": txt[:400]})
    rules = list(ROOT.glob("**/Rules.md"))
    for p in rules[:3]:
        try:
            txt = p.read_text(encoding="utf-8", errors="replace")
            docs.append({"name": str(p.relative_to(ROOT)), "path": str(p), "chars": len(txt), "preview": txt[:400]})
        except OSError:
            pass
    return docs


def library_data():
    cached, fresh = cache_get("library_data", 5.0)
    if fresh and isinstance(cached, dict):
        return cached
    cycles, entries, skips, lines, err = parse_log()
    audit, audit_signals, mr09 = parse_audit(1000)
    oms = read_json(SDIR / "oms_state.json") or {}
    research = parse_research()
    # backtest trades from equity csv already in research.equity
    articles = learning_payload()["glossary"]
    _lib_out = {
        "cycles": cycles[-300:],
        "entries": entries,
        "skips": skips,
        "audit_signals": audit_signals,
        "oms": oms,
        "equity_bt": research.get("equity"),
        "trials": research.get("trials"),
        "articles": articles,
        "docs": library_docs(),
        "verdict": research.get("verdict"),
        "segments": research.get("segments"),
        "gates": research.get("gates"),
    }
    cache_set("library_data", _lib_out)
    return _lib_out


def read_notes():
    LIBDIR.mkdir(parents=True, exist_ok=True)
    items = {}
    if not NOTES.exists():
        return []
    for l in NOTES.read_text(encoding="utf-8", errors="replace").splitlines():
        if not l.strip():
            continue
        try:
            o = json.loads(l)
        except ValueError:
            continue
        nid = o.get("id")
        if not nid:
            continue
        if o.get("tombstone"):
            items.pop(nid, None)
        else:
            items[nid] = o
    return sorted(items.values(), key=lambda x: x.get("ts", 0), reverse=True)


def append_note(obj: dict):
    LIBDIR.mkdir(parents=True, exist_ok=True)
    with NOTES.open("a", encoding="utf-8") as f:
        f.write(json.dumps(obj, ensure_ascii=False) + "\n")


def git_milestones():
    try:
        out = subprocess.run(
            ["git", "-C", str(ROOT), "log", "--pretty=format:%H|%cI|%s", "-40"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        rows = []
        for line in out.stdout.splitlines():
            parts = line.split("|", 2)
            if len(parts) == 3:
                rows.append({"hash": parts[0][:10], "full": parts[0], "date": parts[1], "subject": parts[2]})
        return rows
    except Exception:
        return []


def config_snapshots():
    out = []
    cfg = ROOT / "config"
    if not cfg.exists():
        return out
    for p in sorted(cfg.glob("*.toml")):
        try:
            txt = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        # redact anything that looks like a secret
        red = re.sub(r"(?i)(api[_-]?key|secret|password|token)\s*=\s*[\"']?[^\"'\n]+",
                      r"\1 = \"[REDACTED]\"", txt)
        out.append({"name": p.name, "text": red})
    for p in cfg.glob("*.json"):
        if p.name.startswith("."):
            continue
        try:
            txt = p.read_text(encoding="utf-8", errors="replace")[:50000]
            out.append({"name": p.name, "text": txt})
        except OSError:
            pass
    return out


def read_doc(name: str):
    # only allow markdown under ROOT, no .. and no .env
    name = name.replace("\\", "/").lstrip("/")
    if ".." in name or name.endswith(".env") or "/." in f"/{name}":
        return None
    if not (name.endswith(".md") or name.endswith(".toml") or name.endswith(".json") or name.endswith(".csv") or name.endswith(".jsonl")):
        return None
    p = (ROOT / name).resolve()
    try:
        p.relative_to(ROOT.resolve())
    except ValueError:
        return None
    if not p.exists() or not p.is_file():
        return None
    if p.name == ".env" or "secret" in p.name.lower():
        return None
    try:
        return p.read_text(encoding="utf-8", errors="replace")[:400000]
    except OSError:
        return None



def sse_broadcast(obj):
    line = "data: " + json.dumps(obj, default=str) + "\n\n"
    dead = []
    with _lock:
        clients = list(_SSE_CLIENTS)
    for c in clients:


        try:
            c["q"].append(line)
        except Exception:
            dead.append(c)
    if dead:
        with _lock:
            for c in dead:
                if c in _SSE_CLIENTS:
                    _SSE_CLIENTS.remove(c)




_LOG_TAIL_THREAD = None
_LOG_POS = {"run": 0, "audit": 0}
_EVENT_SEEN = set()


def _file_size(path: Path) -> int:
    try:
        return path.stat().st_size
    except OSError:
        return 0


def _emit_engine_event(ev: dict):
    eid = ev.get("id")
    if eid and eid in _EVENT_SEEN:
        return
    if eid:
        _EVENT_SEEN.add(eid)
        if len(_EVENT_SEEN) > 2000:
            _EVENT_SEEN.clear()
    sse_broadcast({"type": "engine_event", **ev})


def log_tail_loop():
    """Push new run.log / audit.jsonl lines as engine_event over SSE (not 3s polling)."""
    run_p = SDIR / "run.log"
    audit_p = SDIR / "audit.jsonl"
    # seed positions at EOF so we only stream fresh after boot
    _LOG_POS["run"] = _file_size(run_p)
    _LOG_POS["audit"] = _file_size(audit_p)
    buf_run = ""
    buf_audit = ""
    while True:
        try:
            for key, path, is_audit in (("run", run_p, False), ("audit", audit_p, True)):
                sz = _file_size(path)
                pos = _LOG_POS.get(key, 0)
                if sz < pos:
                    pos = 0
                if sz == pos:
                    continue
                try:
                    with path.open("r", encoding="utf-8", errors="replace") as f:
                        f.seek(pos)
                        chunk = f.read()
                        _LOG_POS[key] = f.tell()
                except OSError:
                    continue
                if not chunk:
                    continue
                if is_audit:
                    buf_audit += chunk
                    parts = buf_audit.splitlines(True)
                    if parts and not parts[-1].endswith("\n"):
                        buf_audit = parts[-1]
                        parts = parts[:-1]
                    else:
                        buf_audit = ""
                    for line in parts:
                        line = line.strip()
                        if not line:
                            continue
                        try:
                            o = json.loads(line)
                        except ValueError:
                            continue
                        kind = str(o.get("event") or "")
                        d = o.get("data") or {}
                        ev = {
                            "id": f"live-audit-{o.get('ts')}-{kind}-{d.get('symbol')}",
                            "source": "audit.jsonl",
                            "kind": kind,
                            "ts": o.get("ts"),
                            "symbol": d.get("symbol"),
                            "strategy": d.get("strategy"),
                            "reason": d.get("reason") or d.get("skip") or d.get("code"),
                            "summary": kind,
                        }
                        ev.update(event_path_for(kind, ev.get("reason"), ev.get("strategy")))
                        _emit_engine_event(ev)
                else:
                    buf_run += chunk
                    parts = buf_run.splitlines(True)
                    if parts and not parts[-1].endswith("\n"):
                        buf_run = parts[-1]
                        parts = parts[:-1]
                    else:
                        buf_run = ""
                    for line in parts:
                        s = line.rstrip()
                        if not s:
                            continue
                        m = CYCLE.match(s.strip())
                        if m:
                            ev = {
                                "id": f"live-cycle-{m[1]}",
                                "source": "run.log",
                                "kind": "CYCLE",
                                "ts": m[1],
                                "regime": m[2],
                                "long_ok": m[3].lower() == "true",
                                "short_ok": m[4].lower() == "true",
                                "equity": float(m[6]),
                                "summary": f"regime={m[2]}",
                            }
                            ev.update(event_path_for("CYCLE", m[2]))
                            _emit_engine_event(ev)
                            continue
                        m = ENTRY.search(s)
                        if m:
                            ev = {
                                "id": f"live-entry-{m[1]}-{m[2]}-{time.time()}",
                                "source": "run.log",
                                "kind": "ENTRY",
                                "strategy": m[1],
                                "symbol": m[2],
                                "side": m[3],
                                "summary": f"ENTRY {m[1]} {m[2]} {m[3]}",
                            }
                            ev.update(event_path_for("ENTRY", strategy=m[1]))
                            _emit_engine_event(ev)
                            continue
                        m = SKIP.search(s)
                        if m:
                            ev = {
                                "id": f"live-skip-{m[2]}-{m[4]}-{time.time()}",
                                "source": "run.log",
                                "kind": "SKIP",
                                "strategy": m[1],
                                "symbol": m[2],
                                "side": m[3],
                                "reason": m[4].lstrip("_"),
                                "summary": f"SKIP {m[4]} {m[2]}",
                            }
                            ev.update(event_path_for("SKIP", m[4], m[1]))
                            _emit_engine_event(ev)
        except Exception as ex:
            sse_broadcast({"type": "error", "error": "log_tail: " + str(ex)})
        time.sleep(0.35)


def ensure_log_tailer():
    global _LOG_TAIL_THREAD
    if _LOG_TAIL_THREAD and _LOG_TAIL_THREAD.is_alive():
        return
    _LOG_TAIL_THREAD = threading.Thread(target=log_tail_loop, name="log-tail", daemon=True)
    _LOG_TAIL_THREAD.start()


def rest_poll_loop():
    """Fast shared REST poll for visible symbols — primary live path (WS data frames blocked)."""
    global _WS_STATUS
    _WS_STATUS = {
        "status": "rest_sse",
        "note": "WS handshake OK on PC but market data frames timeout after SUBSCRIBE ACK; using REST→SSE",
    }
    while True:
        try:
            with _lock:
                vis = set(_VISIBLE)
            syms = set()
            pairs = []
            for item in vis:
                if "|" in item:
                    s, tf = item.split("|", 1)
                    syms.add(s)
                    pairs.append((s, tf))
                else:
                    syms.add(item)
            if not syms:
                # still keep BTC/ETH tickers warm for topbar
                syms = {"BTCUSDT", "ETHUSDT"}
            tickers = {}
            # one 24hr call cached 5s for favorites/% — heavy if uncached; use per-symbol price (weight 1)
            # Skip probing while IP banned — serve last-good only via binance_get short-circuit
            for s in list(syms)[:12]:
                pr = binance_get(f"/fapi/v1/ticker/price?symbol={s}", ttl=1.0)
                mk = binance_get(f"/fapi/v1/premiumIndex?symbol={s}", ttl=2.0)
                last = float(pr.get("price")) if isinstance(pr, dict) and pr.get("price") else None
                mark = float(mk.get("markPrice")) if isinstance(mk, dict) and mk.get("markPrice") else None
                tickers[s] = {"last": last, "mark": mark}
            bars = {}
            for s, tf in pairs[:16]:
                raw = binance_get(f"/fapi/v1/klines?symbol={s}&interval={tf}&limit=2", ttl=1.2)
                if isinstance(raw, list) and raw:
                    bars[f"{s}|{tf}"] = [
                        {
                            "time": int(k[0]) // 1000,
                            "open": float(k[1]),
                            "high": float(k[2]),
                            "low": float(k[3]),
                            "close": float(k[4]),
                            "volume": float(k[5]),
                        }
                        for k in raw
                    ]
            sse_broadcast({"type": "tick", "ts": int(time.time() * 1000), "tickers": tickers, "bars": bars, "via": "rest"})
        except Exception as e:
            sse_broadcast({"type": "error", "error": str(e)})
        time.sleep(5.0 if _now() < _ban_until else 1.0)


def ensure_poller():
    global _POLL_THREAD
    if _POLL_THREAD and _POLL_THREAD.is_alive():
        return
    _POLL_THREAD = threading.Thread(target=rest_poll_loop, name="rest-sse", daemon=True)
    _POLL_THREAD.start()


def try_start_ws_probe():
    """Best-effort: confirm WS still only ACKs; do not rely on it for data."""
    def run():
        global _WS_STATUS
        try:
            from wsclient import WSClient
            got = {"ack": False, "data": False}

            def on_msg(text):
                if '"result"' in text:
                    got["ack"] = True
                elif "aggTrade" in text or "kline" in text or "markPrice" in text:
                    got["data"] = True

            def on_st(s):
                _WS_STATUS["ws_probe"] = s

            c = WSClient("fstream.binance.com", "/market/ws", on_message=on_msg, on_status=on_st)
            c.start()
            time.sleep(1.5)
            c.send_text(json.dumps({"method": "SUBSCRIBE", "params": ["ethusdt@aggTrade"], "id": 99}))
            time.sleep(6)
            _WS_STATUS.update({"ack": got["ack"], "data_frames": got["data"], "status": "rest_sse" if not got["data"] else "ws_ok"})
            c.stop()
        except Exception as e:
            _WS_STATUS["probe_error"] = str(e)

    threading.Thread(target=run, name="ws-probe", daemon=True).start()


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _safe_write(self, body: bytes):
        try:
            self.wfile.write(body)
            return True
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError, OSError):
            return False

    def _json(self, obj, code=200):
        body = json.dumps(obj, default=str).encode()
        try:
            self.send_response(code)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError, OSError):
            return None
        self._safe_write(body)
        return None

    def _bytes(self, body: bytes, ctype: str, code=200):
        try:
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError, OSError):
            return None
        self._safe_write(body)
        return None

    def _send_error_json(self, message="dashboard request failed"):
        try:
            self._json({"ok": False, "error": "internal_server_error", "message": message}, 500)
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError, OSError):
            pass
        return None

    def do_GET(self):
        try:
            return self._do_GET_impl()
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            return None
        except Exception as exc:
            try:
                import traceback
                TRACEBACK_LOG.parent.mkdir(parents=True, exist_ok=True)
                with TRACEBACK_LOG.open("a", encoding="utf-8") as f:
                    f.write(f"\\n--- {datetime.utcnow().isoformat()}Z {type(exc).__name__}: {exc} ---\\n")
                    traceback.print_exc(file=f)
            except Exception:
                pass
            self._send_error_json()
            return None

    def _do_GET_impl(self):
        u = urllib.parse.urlparse(self.path)
        path = u.path
        q = urllib.parse.parse_qs(u.query)
        try:
            if path == "/api/state":
                return self._json(state_payload())
            if path == "/api/klines":
                sym = q.get("symbol", ["ETHUSDT"])[0]
                tf = q.get("interval", ["15m"])[0]
                return self._json(klines(sym, tf))
            if path == "/api/live":
                return self._json(live(q.get("symbol", ["ETHUSDT"])[0], q.get("interval", ["15m"])[0]))
            if path == "/api/research":
                return self._json(parse_research())
            if path in ("/api/alerts", "/api/paper"):
                import paper_monitor as _pm
                return self._json(_pm.alerts() if path == "/api/alerts" else _pm.progress())

            if path == "/api/micro/health":
                from micro_live import HUB as MICRO_HUB
                sym = (q.get("symbol") or ["ETHUSDT"])[0].upper()
                MICRO_HUB.ensure_symbol(sym)
                return self._json(MICRO_HUB.health(sym))
            if path == "/api/micro/snapshot":
                from micro_live import HUB as MICRO_HUB
                sym = (q.get("symbol") or ["ETHUSDT"])[0].upper()
                tab = (q.get("tab") or ["orderflow"])[0]
                window = (q.get("window") or ["15m"])[0]
                MICRO_HUB.ensure_symbol(sym)
                MICRO_HUB.set_active([sym, "BTCUSDT" if sym != "BTCUSDT" else "ETHUSDT"], window=window)
                if tab == "command":
                    snap = MICRO_HUB.snapshot(sym, "walls", window=window)
                    now_ms = int(time.time() * 1000)
                    # Compose one read-only command snapshot from the same live hub.
                    def cached_payload(owner, name):
                        row = MICRO_HUB.vike_cache.get(owner, {}).get(name) or {}
                        return row.get("data") if isinstance(row, dict) else {}
                    ctx = {
                        "cohorts": cached_payload(sym, "cohort_metrics"),
                        "liqmap": cached_payload(sym, "liq_heatmap"),
                        "large_fills": cached_payload(sym, "large_fills"),
                        "liq_risk": cached_payload("_global", "liq_risk"),
                    }
                    snap["vike"] = ctx
                    # Binance metrics are fetched from public endpoints; this is only context, never execution.
                    prem = binance_get(f"/fapi/v1/premiumIndex?symbol={sym}", ttl=10) or {}
                    oi = binance_get(f"/fapi/v1/openInterest?symbol={sym}", ttl=10) or {}
                    lsr = binance_get(f"/futures/data/globalLongShortAccountRatio?symbol={sym}&period=1h&limit=1", ttl=10) or []
                    taker = binance_get(f"/futures/data/takerlongshortRatio?symbol={sym}&period=1h&limit=1", ttl=10) or []
                    last_lsr = lsr[-1] if isinstance(lsr, list) and lsr else lsr
                    last_taker = taker[-1] if isinstance(taker, list) and taker else taker
                    snap["binance"] = {
                        "funding": prem.get("lastFundingRate") if isinstance(prem, dict) else None,
                        "next_funding": prem.get("nextFundingTime") if isinstance(prem, dict) else None,
                        "mark": prem.get("markPrice") if isinstance(prem, dict) else snap.get("mark"),
                        "index": prem.get("indexPrice") if isinstance(prem, dict) else None,
                        "oi": oi.get("openInterest") if isinstance(oi, dict) else None,
                        "long_short": last_lsr,
                        "taker": last_taker,
                    }
                    # Explicit panel contracts consumed by the browser Command Center.
                    liq_raw = ctx.get("liqmap") if isinstance(ctx.get("liqmap"), dict) else {}
                    cohort_raw = ctx.get("cohorts") if isinstance(ctx.get("cohorts"), dict) else {}
                    snap["volume_profile"] = {
                        "levels": snap.get("profile") or [],
                        "poc": snap.get("poc"), "vah": snap.get("vah"), "val": snap.get("val"),
                        "source": "BINANCE",
                    }
                    snap["orderbook"] = {
                        "bids": (snap.get("book") or {}).get("bids") or [],
                        "asks": (snap.get("book") or {}).get("asks") or [],
                        "mid": (snap.get("book") or {}).get("mid") or snap.get("mark"),
                        "spread": ((snap.get("book") or {}).get("asks") or [[None]])[0][0] - ((snap.get("book") or {}).get("bids") or [[None]])[0][0] if ((snap.get("book") or {}).get("asks") and (snap.get("book") or {}).get("bids")) else None,
                        "source": "BINANCE",
                    }
                    snap["market_stats"] = {
                        "mark": snap["binance"].get("mark") or snap.get("mark"),
                        "index": snap["binance"].get("index"),
                        "funding": snap["binance"].get("funding"),
                        "open_interest": snap["binance"].get("oi"),
                        "long_short": snap["binance"].get("long_short"),
                        "taker": snap["binance"].get("taker"),
                        "source": "BINANCE",
                    }
                    snap["liquidation_map"] = {
                        "bins": liq_raw.get("bins") or [],
                        "mark_price": liq_raw.get("mark_price") or snap.get("mark"),
                        "total_value_usd": liq_raw.get("total_value_usd"),
                        "coverage": liq_raw.get("coverage"),
                        "realized": snap.get("realized") or [],
                        "source": "VIKE / HYPERLIQUID",
                    }
                    snap["cohort_board"] = {
                        "metrics": cohort_raw.get("metrics") or [],
                        "freshness": cohort_raw.get("freshness"),
                        "axis": cohort_raw.get("axis") or "size",
                        "source": "VIKE / HYPERLIQUID",
                    }
                    snap["analysis"] = {"execution": "DISABLED_READ_ONLY", "formula": "client_context_score_v0"}
                    snap["source"] = "BINANCE + VIKE"
                    snap["ts"] = now_ms
                    return self._json(snap)
                return self._json(MICRO_HUB.snapshot(sym, tab, window=window))
            if path == "/api/micro/weight":
                from micro_live import HUB as MICRO_HUB
                return self._json(MICRO_HUB.weight_status())
            if path == "/api/micro/stream":
                from micro_live import HUB as MICRO_HUB
                sym = (q.get("symbol") or ["ETHUSDT"])[0].upper()
                MICRO_HUB.ensure_symbol(sym)
                window = (q.get("window") or ["15m"])[0]
                MICRO_HUB.set_active([sym, "BTCUSDT" if sym != "BTCUSDT" else "ETHUSDT"], window=window)
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Connection", "keep-alive")
                self.end_headers()
                mq = MICRO_HUB.add_sse_client()
                try:
                    self.wfile.write(b'data: {"type":"hello","channel":"micro"}\n\n')
                    self.wfile.flush()
                    idle = 0
                    while idle < 6000:
                        if mq:
                            while mq:
                                line = mq.popleft()
                                self.wfile.write(("data: " + line + "\n\n").encode())
                            self.wfile.flush()
                            idle = 0
                        else:
                            self.wfile.write(b": hb\n\n")
                            self.wfile.flush()
                            idle += 1
                            time.sleep(0.2)
                except Exception:
                    pass
                finally:
                    MICRO_HUB.remove_sse_client(mq)
                return
            if path == "/api/vike/cohorts":
                from micro_live import HUB as MICRO_HUB
                sym = (q.get("symbol") or ["ETHUSDT"])[0].upper()
                return self._json(MICRO_HUB.snapshot(sym, "cohorts"))
            if path == "/api/vike/funding-oi":
                from micro_live import HUB as MICRO_HUB
                sym = (q.get("symbol") or ["ETHUSDT"])[0].upper()
                return self._json(MICRO_HUB.snapshot(sym, "funding_oi"))
            if path == "/api/vike/liq-risk":
                from micro_live import HUB as MICRO_HUB
                sym = (q.get("symbol") or ["ETHUSDT"])[0].upper()
                return self._json(MICRO_HUB.snapshot(sym, "liq_risk"))

            if path == "/api/vike/status":
                return self._json(vike_status_payload())
            if path == "/api/vike/liquidation-heatmap":
                sym = q.get("symbol", ["ETHUSDT"])[0]
                try:
                    bins = int(q.get("bins", ["80"])[0])
                except ValueError:
                    bins = 80
                return self._json(vike_liquidation_heatmap(sym, bins))
            if path == "/api/vike/liquidations":
                sym = q.get("symbol", ["ETHUSDT"])[0]
                try:
                    lim = int(q.get("limit", ["500"])[0])
                except ValueError:
                    lim = 500
                return self._json(vike_liquidations_feed(sym, lim))
            if path == "/api/heatmap":
                return self._json(heatmap_market())
            if path == "/api/market":
                cycles, *_ = parse_log()
                _, _, mr09 = parse_audit(200)
                m = market_metrics()
                m["regimes"] = [{"ts": c["ts"], "regime": c["regime"]} for c in cycles[-200:]]
                m["mr09"] = mr09[-50:]
                return self._json(m)
            if path == "/api/learning":
                return self._json(learning_payload())
            if path in (
                "/api/artatrade",
                "/api/markets/scan",
                "/api/live/mesh",
                "/api/universe/audit",
                "/api/learning/probability",
            ):
                candidates = [SDIR / "artatrade_console.json", ROOT / "state" / "artatrade_console.json"]
                existing = [cp for cp in candidates if cp.exists()]
                console = {}
                if existing:
                    existing.sort(key=lambda cp: cp.stat().st_mtime, reverse=True)
                    try:
                        console = json.loads(existing[0].read_text(encoding="utf-8"))
                    except Exception:
                        console = {}
                if path == "/api/markets/scan":
                    return self._json({
                        "trading_style": console.get("trading_style", "SCALP"),
                        "scalp_lane_active": console.get("scalp_lane_active", True),
                        "swing_lane_active": False,
                        "macro_gate": console.get("macro_gate"),
                        "progressive_policy": console.get("progressive_policy"),
                        "rows": console.get("scan_rows", []),
                    })
                if path == "/api/live/mesh":
                    return self._json({
                        "agent_mesh": console.get("agent_mesh", {"agents": [], "council_decisions": []}),
                        "rate_guard": console.get("rate_guard"),
                        "mirror_reconcile": console.get("mirror_reconcile"),
                    })
                if path == "/api/universe/audit":
                    return self._json(console.get("universe_audit", {}))
                if path == "/api/learning/probability":
                    return self._json(console.get("learning", {}))
                return self._json(console)
            if path == "/api/library":
                return self._json(library_data())
            if path == "/api/notes":
                return self._json({"notes": read_notes()})
            if path == "/api/milestones":
                return self._json({"commits": git_milestones(), "configs": config_snapshots()})
            if path == "/api/doc":
                name = q.get("name", [""])[0]
                txt = read_doc(name)
                if txt is None:
                    return self._json({"error": "not found"}, 404)
                return self._json({"name": name, "text": txt})
            if path == "/api/engine-events":
                return self._json({"events": build_engine_events(120)})
            if path == "/api/anatomy":
                return self._json({
                    "v10": state_payload()["v10"],
                    "chain": state_payload()["chain"],
                    "session_utc": {"asia": [0, 8], "ny": [13, 21], "label": "asia_ny"},
                    "params": {
                        "s1_gap_atr": 0.20, "s1_body_frac": 0.50, "s1_srv_min": 0.15,
                        "s1_pivot_lookback": 20, "s1_direct_limit": True, "s1_rr": 2.0,
                        "sl_min_pct": 0.02, "sl_max_pct": 0.04, "sl_buffer_atr": 0.15,
                    },
                    "disclaimer": "Overlay chart = aproksimasi logika engine di JS dari kline; bukan log resmi.",
                })
            if path == "/api/symbols":
                refresh_symbols()
                qsearch = (q.get("q", [""])[0] or "").upper()
                tick = binance_get("/fapi/v1/ticker/24hr", ttl=60)
                tmap = {}
                if isinstance(tick, list):
                    for row in tick:
                        tmap[row.get("symbol")] = {
                            "pct": float(row.get("priceChangePercent") or 0),
                            "qv": float(row.get("quoteVolume") or 0),
                            "last": float(row.get("lastPrice") or 0),
                        }
                items = []
                for s in _SYMBOLS_META or [{"symbol": x} for x in sorted(_SYMBOLS)]:
                    sym = s["symbol"]
                    if qsearch and qsearch not in sym:
                        continue
                    meta = tmap.get(sym) or {}
                    items.append({"symbol": sym, "base": s.get("base"), "quote": s.get("quote"),
                                  "tick_size": s.get("tick_size"), "precision": s.get("precision", 0), **meta})
                items.sort(key=lambda x: -(x.get("qv") or 0))
                return self._json({"count": len(_SYMBOLS), "items": items, "ws": _WS_STATUS})
            if path == "/api/stream":
                ensure_log_tailer()
                # SSE — client may pass symbols=ETHUSDT,BTCUSDT&intervals=ETHUSDT|15m,...
                syms = [x.strip().upper() for x in (q.get("symbols", [""])[0] or "").split(",") if x.strip()]
                ivs = [x.strip().upper() for x in (q.get("intervals", [""])[0] or "").split(",") if x.strip()]
                with _lock:
                    for s in syms:
                        if valid_symbol(s):
                            _VISIBLE.add(s)
                    for it in ivs:
                        _VISIBLE.add(it)
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Connection", "keep-alive")
                self.end_headers()
                qlist = []
                client = {"q": qlist}
                with _lock:
                    _SSE_CLIENTS.append(client)
                try:
                    self.wfile.write(b'data: {"type":"hello","via":"rest_sse"}\n\n')
                    self.wfile.flush()
                    while True:
                        while qlist:
                            line = qlist.pop(0)
                            self.wfile.write(line.encode())
                            self.wfile.flush()
                        time.sleep(0.05)
                except Exception:
                    pass
                finally:
                    with _lock:
                        if client in _SSE_CLIENTS:
                            _SSE_CLIENTS.remove(client)
                return
            if path == "/api/visible":
                # POST-like via GET for simplicity: set visible cells
                raw = q.get("set", [""])[0]
                items = [x.strip().upper() for x in raw.split(",") if x.strip()]
                with _lock:
                    _VISIBLE.clear()
                    for it in items:
                        _VISIBLE.add(it)
                ensure_poller()
                return self._json({"ok": True, "visible": sorted(_VISIBLE), "ws": _WS_STATUS})
            # static
            if path in ("/", "/index.html"):
                p = HERE / "index.html"
                return self._bytes(p.read_bytes(), "text/html; charset=utf-8")
            if path.startswith("/static/"):
                rel = path[len("/static/"):]
                if ".." in rel:
                    return self._json({"error": "bad"}, 400)
                p = HERE / "static" / rel
                if not p.exists():
                    self.send_response(404); self.end_headers(); return
                ctype = "application/javascript" if p.suffix == ".js" else (
                    "text/css" if p.suffix == ".css" else "application/octet-stream")
                return self._bytes(p.read_bytes(), ctype)
            self.send_response(404); self.end_headers()
        except Exception as e:
            self._json({"error": str(e)}, 500)

    def do_POST(self):
        u = urllib.parse.urlparse(self.path)
        if u.path != "/api/notes":
            return self._json({"error": "forbidden"}, 403)
        try:
            n = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            n = 0
        if n <= 0 or n > 8000:
            return self._json({"error": "payload terlalu besar/kosong"}, 400)
        raw = self.rfile.read(n)
        try:
            body = json.loads(raw.decode("utf-8"))
        except ValueError:
            return self._json({"error": "JSON tidak valid"}, 400)
        action = body.get("action") or "create"
        if action == "create":
            text = (body.get("text") or "").strip()
            if not text or len(text) > 4000:
                return self._json({"error": "teks wajib 1..4000 karakter"}, 400)
            note = {
                "id": f"n{int(_now()*1000)}",
                "ts": int(_now() * 1000),
                "text": text,
                "tag": (body.get("tag") or "umum")[:64],
                "ref": (body.get("ref") or "")[:128],
                "kind": (body.get("kind") or "general")[:32],
            }
            append_note(note)
            return self._json({"ok": True, "note": note})
        if action in ("delete", "edit"):
            nid = body.get("id")
            if not nid:
                return self._json({"error": "id wajib"}, 400)
            if action == "delete":
                append_note({"id": nid, "tombstone": True, "ts": int(_now() * 1000)})
                return self._json({"ok": True})
            text = (body.get("text") or "").strip()
            if not text or len(text) > 4000:
                return self._json({"error": "teks wajib"}, 400)
            # find existing
            cur = next((n for n in read_notes() if n.get("id") == nid), None)
            if not cur:
                return self._json({"error": "tidak ketemu"}, 404)
            note = {
                "id": nid,
                "ts": int(_now() * 1000),
                "text": text,
                "tag": body.get("tag", cur.get("tag")),
                "ref": body.get("ref", cur.get("ref")),
                "kind": body.get("kind", cur.get("kind")),
                "edited": True,
            }
            append_note(note)
            return self._json({"ok": True, "note": note})
        return self._json({"error": "action tidak dikenal"}, 400)


if __name__ == "__main__":
    LIBDIR.mkdir(parents=True, exist_ok=True)
    import os as _os
    try:
        _seed = float(_os.environ.get("APEX_FAPI_BAN_SECS") or 0)
        if _seed > 0:
            _ban_until = _now() + min(900.0, _seed)
            print(f"[binance] seeded ban {_seed:.0f}s until {int(_ban_until)}", flush=True)
    except Exception:
        pass
    try:
        n = len(refresh_symbols())
        print(f"symbols TRADING USDT-M: {n}")
    except Exception as e:
        print("symbols refresh failed", e)
    with _lock:
        _cache.pop("bn:/fapi/v1/ticker/24hr", None)
    # Drop poisoned exchangeInfo / empty 24hr so first paint can refill
    with _lock:
        for k in list(_cache.keys()):
            if k in ("exchangeInfo", "bn:/fapi/v1/exchangeInfo", "bn:/fapi/v1/ticker/24hr"):
                val=_cache.get(k)
                if val and isinstance(val[1], (list,dict)):
                    body=val[1]
                    if (isinstance(body,list) and len(body)==0) or (isinstance(body,dict) and not body.get("symbols")):
                        _cache.pop(k, None)
    # Re-apply seeded ban AFTER any reset — never wipe a deliberate backoff
    try:
        _seed = float(_os.environ.get("APEX_FAPI_BAN_SECS") or 0)
        if _seed > 0:
            _ban_until = max(_ban_until, _now() + min(900.0, _seed))
            print(f"[binance] ban latch held until {int(_ban_until)} (seed {_seed:.0f}s)", flush=True)
    except Exception:
        pass

    _load_heatmap_disk()
    _load_klines_disk()
    # Seed CSV klines into memory cache immediately so all TFs (incl. ETHUSDT 4h) are ready at boot
    for _s in ("ETHUSDT", "BTCUSDT"):
        for _tf in ("1m", "5m", "15m", "1h", "4h", "1d"):
            _k = f"kl:{_s}:{_tf}:500"
            _d, _ = cache_get(_k, 86400)
            if not _d:
                _csv_d = _klines_from_csv(_s, _tf, 500)
                if _csv_d:
                    cache_set(_k, _csv_d)

    def _bg_prewarm_loop():
        while True:
            try:
                state_payload()
                parse_research()
                learning_payload()
                library_data()
                heatmap_market()
                market_metrics()
                for _s in ("ETHUSDT", "BTCUSDT"):
                    for _tf in ("1m", "5m", "15m", "1h", "4h", "1d"):
                        try:
                            _fetch_klines_sync(_s, _tf, 500)
                        except Exception:
                            pass
                _save_klines_disk()
            except Exception:
                pass
            time.sleep(25.0)

    threading.Thread(target=_bg_prewarm_loop, name="bg-prewarm", daemon=True).start()
    ensure_poller()
    try:
        from micro_live import HUB as MICRO_HUB
        MICRO_HUB.start()
    except Exception as _micro_ex:
        print("micro_live start failed:", _micro_ex)
    try_start_ws_probe()
    try:
        import paper_monitor as _pm
        _pm.start()
    except Exception as _pm_ex:
        print("paper_monitor start failed:", _pm_ex)
    print(f"Apex dashboard (read-only+) -> http://127.0.0.1:{PORT} | state: {SDIR}")
    class _ApexHTTPServer(ThreadingHTTPServer):
        request_queue_size = 128
        daemon_threads = True
        allow_reuse_address = True

        def handle_error(self, request, client_address):
            # Browser reloads/EventSource closures are normal on localhost; do not emit noisy tracebacks.
            exc_type = __import__("sys").exc_info()[0]
            if exc_type in (BrokenPipeError, ConnectionResetError, ConnectionAbortedError, OSError):
                return
            super().handle_error(request, client_address)
    _ApexHTTPServer(("127.0.0.1", PORT), H).serve_forever()
