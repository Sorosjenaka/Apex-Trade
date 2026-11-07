"""Paper monitor (read-only): alert + progress paper trading untuk dashboard Apex.

- Tidak menulis ke state engine. Satu-satunya file tulis: dashboard/cache_alerts.json (cache server).
- Tidak mengirim order. Notifikasi = daftar alert di dashboard + toast Windows lokal (opsional).
"""
from __future__ import annotations

import json
import subprocess
import threading
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
SDIR = ROOT / "state" / "paper"
CACHE = HERE / "cache_alerts.json"
WIB = timezone(timedelta(hours=7))

# Kriteria sukses paper (tertulis di muka; selaras gate G-02/G-03/G-04 Rules).
TARGET_TRADES = 60
MIN_E_R = 0.10
MIN_PF = 1.25

_lock = threading.Lock()
_state = {"seen": {}, "alerts": [], "flags": {}, "last_summary_day": None}
_started = False


def _load():
    try:
        if CACHE.exists():
            d = json.loads(CACHE.read_text(encoding="utf-8"))
            if isinstance(d, dict):
                _state.update({k: d.get(k, _state[k]) for k in _state})
    except Exception:
        pass


def _save():
    try:
        tmp = CACHE.with_suffix(".tmp")
        tmp.write_text(json.dumps(_state), encoding="utf-8")
        tmp.replace(CACHE)
    except Exception:
        pass


def _read_json(p: Path):
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except Exception:
        return None


def _toast(title: str, msg: str):
    """Toast Windows lokal; gagal diam-diam (mis. sesi non-interaktif)."""
    ps = (
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime] | Out-Null;"
        "$t=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02);"
        "$n=$t.GetElementsByTagName('text');"
        f"$n.Item(0).AppendChild($t.CreateTextNode('{title}'))|Out-Null;"
        f"$n.Item(1).AppendChild($t.CreateTextNode('{msg}'))|Out-Null;"
        "$toast=[Windows.UI.Notifications.ToastNotification]::new($t);"
        "[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Apex Paper').Show($toast)"
    )
    try:
        subprocess.Popen(
            ["powershell", "-NoProfile", "-WindowStyle", "Hidden", "-Command", ps],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=0x08000000,
        )
    except Exception:
        pass


def _push(level: str, kind: str, text: str, key: str | None = None, toast: bool = True):
    with _lock:
        if key:
            if key in _state["seen"]:
                return
            _state["seen"][key] = int(time.time())
            if len(_state["seen"]) > 2000:
                for k in sorted(_state["seen"], key=_state["seen"].get)[:500]:
                    _state["seen"].pop(k, None)
        _state["alerts"].append({"ts": int(time.time() * 1000), "level": level, "kind": kind, "text": text})
        _state["alerts"] = _state["alerts"][-200:]
    if toast:
        _toast(f"Apex {kind}", text.replace("'", "")[:120])


def _closed_stats(closed):
    rs, pnl, wins, gp, gl = [], 0.0, 0, 0.0, 0.0
    for t in closed:
        p = t.get("pnl_net")
        if p is None:
            continue
        risk = float(t.get("planned_risk") or 0) or 1.0
        rs.append(float(p) / risk)
        pnl += float(p)
        if p > 0:
            wins += 1
            gp += float(p)
        else:
            gl += abs(float(p))
    n = len(rs)
    return {
        "n": n,
        "e_r": (sum(rs) / n) if n else None,
        "pf": (gp / gl) if gl > 0 else None,
        "win_rate": (wins / n) if n else None,
        "net_usd": pnl,
    }


def _alive_proc() -> bool:
    try:
        out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq apex.exe", "/FO", "CSV", "/NH"],
                             capture_output=True, text=True, timeout=4, creationflags=0x08000000).stdout
        return "apex.exe" in out.lower()
    except Exception:
        return False


def _last_cycle_age_min():
    try:
        p = SDIR / "run.log"
        return (time.time() - p.stat().st_mtime) / 60.0
    except Exception:
        return None


def _tick():
    oms = _read_json(SDIR / "oms_state.json") or {}
    trades = oms.get("trades") or {}
    closed = oms.get("closed") or []
    alive = _alive_proc()

    # ENTRY / state perubahan trade aktif
    for cid, t in trades.items():
        st = t.get("state")
        sym, side = t.get("symbol"), t.get("side")
        base = f"{t.get('strategy')} {sym} {side} entry={t.get('entry_price')} sl={t.get('sl')} tp={t.get('tp')}"
        if st == "PendingEntry":
            _push("info", "PENDING", f"Limit pending: {base}", key=f"pend:{cid}", toast=False)
        elif st in ("Open", "Unprotected", "Closing"):
            _push("info", "ENTRY", f"Posisi terisi: {base} fill={t.get('avg_fill')}", key=f"open:{cid}")
            if st == "Unprotected":
                _push("crit", "UNPROTECTED", f"{sym} tanpa proteksi SL/TP!", key=f"unprot:{cid}")
    # CLOSE
    for t in closed:
        cid = t.get("client_id")
        p = t.get("pnl_net")
        risk = float(t.get("planned_risk") or 0) or 1.0
        r = (float(p) / risk) if p is not None else None
        lvl = "info" if (r or 0) >= 0 else "warn"
        _push(lvl, "CLOSE",
              f"{t.get('symbol')} {t.get('side')} ditutup {t.get('exit_kind')} · "
              f"{'' if r is None else f'{r:+.2f}R'} · ${0 if p is None else p:+.2f}", key=f"close:{cid}")

    # Engine down / up (butuh 2 tick berturut-turut agar tidak false alarm)
    fl = _state["flags"]
    if not alive:
        fl["down_n"] = fl.get("down_n", 0) + 1
        if fl["down_n"] >= 2 and not fl.get("down"):
            fl["down"] = True
            _push("crit", "ENGINE_DOWN", "apex.exe paper TIDAK berjalan (watchdog akan restart)")
    else:
        fl["down_n"] = 0
        if fl.get("down"):
            fl["down"] = False
            _push("info", "ENGINE_UP", "apex.exe paper berjalan kembali")

    # Data stale: run.log tidak bergerak > 8 menit padahal engine hidup (siklus 5m)
    age = _last_cycle_age_min()
    if alive and age is not None and age > 8:
        if not fl.get("stale"):
            fl["stale"] = True
            _push("warn", "DATA_STALE", f"run.log tidak bergerak {age:.0f} menit (siklus 5m)")
    elif fl.get("stale"):
        fl["stale"] = False
        _push("info", "DATA_FRESH", "siklus engine bergerak lagi")

    # Ringkasan harian (WIB), sekali per hari saat hari berganti
    today = datetime.now(WIB).strftime("%Y-%m-%d")
    last = _state.get("last_summary_day")
    if last is None:
        _state["last_summary_day"] = today
    elif last != today:
        day_ms0 = int(datetime.strptime(last, "%Y-%m-%d").replace(tzinfo=WIB).timestamp() * 1000)
        day = [t for t in closed if day_ms0 <= int(t.get("closed_ms") or 0) < day_ms0 + 86_400_000]
        s = _closed_stats(day)
        risk = oms.get("risk") or {}
        _push("info", "DAILY",
              f"Ringkasan {last}: {s['n']} trade · net ${s['net_usd']:+.2f} · SL harian {risk.get('daily_sl_count', 0)}",
              key=f"daily:{last}")
        _state["last_summary_day"] = today
    _save()


def _parse_yahudi_audit() -> dict:
    """Ambil telemetri terbaru YAHUDI_MACRO_GATE, YAHUDI_TRACE per simbol, dan YAHUDI_POST_TRADE dari audit.jsonl."""
    audit_p = SDIR / "audit.jsonl"
    macro_gate = None
    traces = {}
    post_trades = []
    if audit_p.exists():
        try:
            with audit_p.open("rb") as f:
                f.seek(0, 2)
                sz = f.tell()
                f.seek(max(0, sz - 262144))
                lines = f.read().decode("utf-8", errors="replace").splitlines()
            for ln in lines[-400:]:
                ln = ln.strip()
                if not ln.startswith("{"):
                    continue
                try:
                    ev = json.loads(ln)
                except Exception:
                    continue
                kind = ev.get("event")
                data = ev.get("data") or {}
                if kind == "YAHUDI_MACRO_GATE":
                    macro_gate = data
                elif kind == "YAHUDI_TRACE":
                    sym = data.get("symbol")
                    if sym:
                        traces[sym] = data
                elif kind == "YAHUDI_POST_TRADE":
                    post_trades.append(data)
        except Exception:
            pass
    return {
        "mode": "SCALP_PRO (S01_SWEEP + S02_SNIPER)",
        "enabled_strategies": ["S01_SWEEP", "S02_SNIPER", "S1_TFP"],
        "swing_rsw_disabled": True,
        "scalp_rr": 1.7,
        "sl_band_pct": [2.0, 4.0],
        "sl_buffer_atr": 0.46,
        "min_sl_atr": 0.805,
        "pull_bias_threshold": 1.6,
        "macro_gate": macro_gate,
        "symbol_traces": traces,
        "post_trades": post_trades[-20:],
    }


def progress() -> dict:
    oms = _read_json(SDIR / "oms_state.json") or {}
    closed = oms.get("closed") or []
    s = _closed_stats(closed)
    trades = oms.get("trades") or {}
    active = [t for t in trades.values() if t.get("state") in ("Open", "Unprotected", "Closing")]
    pending = [t for t in trades.values() if t.get("state") == "PendingEntry"]
    n = s["n"]
    remaining = max(0, TARGET_TRADES - n)
    ok_e = s["e_r"] is not None and s["e_r"] >= MIN_E_R
    ok_pf = s["pf"] is not None and s["pf"] >= MIN_PF
    return {
        "target_trades": TARGET_TRADES,
        "closed": n,
        "remaining": remaining,
        "pct": round(100.0 * min(1.0, n / TARGET_TRADES), 1),
        "e_r": s["e_r"], "pf": s["pf"], "win_rate": s["win_rate"], "net_usd": s["net_usd"],
        "criteria": {"min_e_r": MIN_E_R, "min_pf": MIN_PF,
                     "e_ok": ok_e, "pf_ok": ok_pf,
                     "verdict": ("SAMPEL KURANG" if n < TARGET_TRADES else ("LOLOS KRITERIA PAPER" if (ok_e and ok_pf) else "BELUM LOLOS"))},
        "open": len(active), "pending": len(pending),
        "yahudi_scalp": _parse_yahudi_audit(),
        "note": "Mode SCALP Yahudi-Bot (S-01 Sweep Reclaim & S-02 Liquidity Sniper · 1.7R · Tanpa RSW Swing) + S1_TFP aktif di Paper Trading (arm=false).",
    }


def alerts(limit: int = 50) -> dict:
    with _lock:
        items = list(_state["alerts"][-limit:])
    return {"alerts": items, "flags": dict(_state["flags"]), "progress": progress()}


def start():
    global _started
    if _started:
        return
    _started = True
    _load()

    def loop():
        time.sleep(5)
        while True:
            try:
                _tick()
            except Exception:
                pass
            time.sleep(20)

    threading.Thread(target=loop, name="paper-monitor", daemon=True).start()
