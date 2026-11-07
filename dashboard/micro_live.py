"""Live microstructure hub: Binance WS depth + high-freq REST aggTrades/forceOrder/mark,
Vike polling, ring buffers, precomputed snapshots, SSE deltas. Never logs API keys.
"""
from __future__ import annotations
import json, math, os, pathlib, threading, time, urllib.parse, urllib.request, urllib.error
from collections import defaultdict, deque
from typing import Any, Deque, Dict, List, Optional, Set, Tuple

# ---- constants ----
BINANCE_REST = "https://fapi.binance.com"
FSTREAM_HOST = "fstream.binance.com"
TIER_EDGES = [0, 1_000, 10_000, 50_000, 250_000, float("inf")]  # USD
TIER_NAMES = ["retail", "mid", "large", "whale", "mega"]
MAX_TRADES = 12000
MAX_DEPTH_SNAPS = 600
MAX_FORCE = 500
MAX_CVD_PTS = 2400
MAX_WALL_SERIES = 400
WEIGHT_BUDGET_PER_MIN = 1200  # soft cap across Binance REST
WEIGHT_AGG = 20               # fapi aggTrades
WEIGHT_KLINES = 5
WEIGHT_DEPTH = 2
WEIGHT_PREMIUM = 1
WEIGHT_24HR = 40
FAPI_BAN = {"until": 0.0}  # mutable latch — safe across threads/closures
try:
    _seed = float(os.environ.get("APEX_FAPI_BAN_SECS") or 0)
    if _seed > 0:
        FAPI_BAN["until"] = time.time() + min(900.0, _seed)
        print(f"[micro] seeded FAPI_BAN +{_seed:.0f}s -> {FAPI_BAN['until']:.0f}", flush=True)
except Exception as _e:
    print("[micro] ban seed failed", _e, flush=True)

WINDOW_MS = {"5m": 5 * 60_000, "15m": 15 * 60_000, "1h": 60 * 60_000, "4h": 4 * 60 * 60_000}
WINDOW_INTERVAL = {"5m": "1m", "15m": "1m", "1h": "5m", "4h": "15m"}
WINDOW_CANDLE_LIMIT = {"5m": 30, "15m": 40, "1h": 48, "4h": 48}
ALL_TABS = ("orderflow", "walls", "liqmap", "largevp", "tierflow", "cohorts", "funding_oi", "liq_risk")
ALL_WINDOWS = ("5m", "15m", "1h", "4h")

DASH_DIR = pathlib.Path(__file__).resolve().parent
ROOT_DIR = DASH_DIR.parent
DATA_DIR = ROOT_DIR / "data"
MICRO_CACHE_FILE = DASH_DIR / "cache_micro_last.json"


def _now_ms() -> int:
    return int(time.time() * 1000)


def _tier(usd: float) -> str:
    for i in range(len(TIER_EDGES) - 1):
        if TIER_EDGES[i] <= usd < TIER_EDGES[i + 1]:
            return TIER_NAMES[i]
    return TIER_NAMES[-1]


class Ring:
    def __init__(self, maxlen: int):
        self.dq: Deque[Any] = deque(maxlen=maxlen)
        self.lock = threading.Lock()

    def append(self, x):
        with self.lock:
            self.dq.append(x)

    def list(self) -> list:
        with self.lock:
            return list(self.dq)

    def __len__(self):
        with self.lock:
            return len(self.dq)


class SymbolState:
    def __init__(self, symbol: str):
        self.symbol = symbol.upper()
        self.trades = Ring(MAX_TRADES)  # {t,p,q,usd,side,id,tier}
        self.depth = Ring(MAX_DEPTH_SNAPS)  # {t, bids:[[p,q]], asks:[[p,q]], mid}
        self.force = Ring(MAX_FORCE)  # liquidations
        self.cvd = Ring(MAX_CVD_PTS)  # {t, cvd, buy, sell}
        self.walls = Ring(MAX_WALL_SERIES)  # {t, walls:[{side,price,qty,usd}], mid}
        self.mark: Optional[float] = None
        self.last_price: Optional[float] = None
        self.cvd_val = 0.0
        self.last_agg_id = 0
        self.stats = {
            "agg_msgs": 0,
            "depth_msgs": 0,
            "force_msgs": 0,
            "mark_msgs": 0,
            "rest_agg_polls": 0,
            "last_agg_ms": 0,
            "last_depth_ms": 0,
            "last_force_ms": 0,
            "last_mark_ms": 0,
            "agg_via": "rest",
            "depth_via": "ws",
        }
        self._buy_vol_1s = 0.0
        self._sell_vol_1s = 0.0
        self._bucket_t = 0
        self.backfilled = False
        self.backfilled_windows: Set[str] = set()
        self.backfilling_windows: Set[str] = set()
        self.window = "15m"
        self.klines_delta = Ring(2000)  # {t, buy, sell, delta, cvd}
        self.candles_by_win: Dict[str, List[dict]] = {}
        self.delta_by_win: Dict[str, List[dict]] = {}

    def on_trade(self, t_ms: int, price: float, qty: float, is_buyer_maker: bool, agg_id: int):
        if agg_id and agg_id <= self.last_agg_id:
            return
        if agg_id:
            self.last_agg_id = agg_id
        side = "sell" if is_buyer_maker else "buy"  # aggressor
        usd = price * qty
        self.trades.append({"t": t_ms, "p": price, "q": qty, "usd": usd, "side": side, "id": agg_id, "tier": _tier(usd)})
        self.last_price = price
        if self.mark is None:
            self.mark = price
        self.stats["agg_msgs"] += 1
        self.stats["last_agg_ms"] = _now_ms()
        if side == "buy":
            self.cvd_val += qty
            self._buy_vol_1s += qty
        else:
            self.cvd_val -= qty
            self._sell_vol_1s += qty
        bucket = t_ms // 250
        if bucket != self._bucket_t:
            self._bucket_t = bucket
            self.cvd.append({"t": t_ms, "cvd": self.cvd_val, "buy": self._buy_vol_1s, "sell": self._sell_vol_1s})
            self._buy_vol_1s = 0.0
            self._sell_vol_1s = 0.0
        for win, clist in list(self.candles_by_win.items()):
            if clist:
                last_c = clist[-1]
                last_c["c"] = price
                if price > last_c["h"]:
                    last_c["h"] = price
                if price < last_c["l"]:
                    last_c["l"] = price

    def on_depth(self, t_ms: int, bids: list, asks: list, via: str = "ws"):
        mid = None
        if bids and asks:
            mid = (float(bids[0][0]) + float(asks[0][0])) / 2.0
            if self.last_price is None:
                self.last_price = mid
            if self.mark is None:
                self.mark = mid
        self.depth.append({"t": t_ms, "bids": bids[:20], "asks": asks[:20], "mid": mid})
        self.stats["depth_msgs"] += 1
        self.stats["last_depth_ms"] = _now_ms()
        self.stats["depth_via"] = via
        walls = []
        thr = 25000.0
        for side, book in (("bid", bids), ("ask", asks)):
            side_walls = []
            for p, q in book[:20]:
                fp = float(p)
                fq = float(q)
                if mid is not None:
                    if side == "bid" and fp > mid:
                        continue
                    if side == "ask" and fp < mid:
                        continue
                usd = fp * fq
                side_walls.append({"side": side, "price": fp, "qty": fq, "usd": usd})
            big = [w for w in side_walls if w["usd"] >= thr]
            if len(big) < 3:
                big = sorted(side_walls, key=lambda x: x["usd"], reverse=True)[:4]
            walls.extend(big)
        self.walls.append({"t": t_ms, "walls": walls, "mid": mid})

    def on_force(self, row: dict):
        self.force.append(row)
        self.stats["force_msgs"] += 1
        self.stats["last_force_ms"] = _now_ms()

    def on_mark(self, mark: float, t_ms: int):
        self.mark = mark
        if self.last_price is None:
            self.last_price = mark
        self.stats["mark_msgs"] += 1
        self.stats["last_mark_ms"] = t_ms or _now_ms()


class MicroHub:
    def __init__(self):
        self.lock = threading.RLock()
        self.symbols: Dict[str, SymbolState] = {}
        self.active: Set[str] = set()
        self.focus_sym: str = "ETHUSDT"
        self._ws = None
        self._ws_status = "idle"
        self._ws_last = 0.0
        self._ws_Reconnects = 0
        self._stop = threading.Event()
        self._threads: List[threading.Thread] = []
        self.vike_cache: Dict[str, Any] = {}
        self.vike_meta: Dict[str, Any] = {}
        self.snap_cache: Dict[str, dict] = {}
        self._started = False
        self._weight_used = 0
        self._weight_window_t = time.time()
        self._weight_log = []
        self._weight_by: Dict[str, int] = {}
        self.agg_ws_status = {"tried": [], "working": None, "note": "ISP blocks WS aggTrade frames -> REST active"}
        self.last_errors: Dict[str, dict] = {}

        self._sse_queues: List[deque] = []
        self._msg_rates = defaultdict(int)
        self._rate_window_t = time.time()
        self._load_disk_cache()

    def _record_error(self, source: str, msg: str, http_code: int = 0):
        self.last_errors[source] = {
            "source": source,
            "message": str(msg)[:160],
            "http": http_code,
            "ts": _now_ms(),
        }

    def _load_disk_cache(self):
        try:
            if MICRO_CACHE_FILE.exists():
                raw = json.loads(MICRO_CACHE_FILE.read_text("utf-8"))
                if isinstance(raw, dict):
                    vc = raw.get("vike_cache")
                    if isinstance(vc, dict):
                        self.vike_cache.update(vc)
                    vm = raw.get("vike_meta")
                    if isinstance(vm, dict):
                        self.vike_meta.update(vm)
                    sym_seed = raw.get("symbols")
                    if isinstance(sym_seed, dict):
                        for sym, sdata in sym_seed.items():
                            st = self.ensure_symbol(sym)
                            if sdata.get("mark"):
                                st.on_mark(float(sdata["mark"]), _now_ms())
                            if sdata.get("last_price"):
                                st.last_price = float(sdata["last_price"])
                            cbw = sdata.get("candles_by_win")
                            if isinstance(cbw, dict):
                                st.candles_by_win.update(cbw)
                            dbw = sdata.get("delta_by_win")
                            if isinstance(dbw, dict):
                                st.delta_by_win.update(dbw)
                            for tr in (sdata.get("trades") or [])[-1500:]:
                                try:
                                    st.trades.append(tr)
                                except Exception:
                                    pass
                            for dp in (sdata.get("depth") or [])[-20:]:
                                try:
                                    st.depth.append(dp)
                                except Exception:
                                    pass
                            for wl in (sdata.get("walls") or [])[-120:]:
                                try:
                                    st.walls.append(wl)
                                except Exception:
                                    pass
        except Exception as e:
            print(f"[micro] disk cache load skipped: {e}", flush=True)

    def _save_disk_cache(self):
        try:
            with self.lock:
                sym_dump = {}
                for sym, st in self.symbols.items():
                    sym_dump[sym] = {
                        "mark": st.mark,
                        "last_price": st.last_price,
                        "candles_by_win": st.candles_by_win,
                        "delta_by_win": st.delta_by_win,
                        "trades": st.trades.list()[-800:],
                        "depth": st.depth.list()[-5:],
                        "walls": st.walls.list()[-80:],
                    }
                payload = {
                    "ts": _now_ms(),
                    "vike_cache": self.vike_cache,
                    "vike_meta": self.vike_meta,
                    "symbols": sym_dump,
                }
            tmp = MICRO_CACHE_FILE.with_suffix(".tmp")
            tmp.write_text(json.dumps(payload, default=str), encoding="utf-8")
            tmp.replace(MICRO_CACHE_FILE)
        except Exception:
            pass

    def _seed_from_local_csv(self, sym: str):
        st = self.symbols.get(sym.upper())
        if not st:
            return
        p5 = DATA_DIR / "klines" / f"{sym}_5m.csv"
        p15 = DATA_DIR / "klines" / f"{sym}_15m.csv"
        p1h = DATA_DIR / "klines" / f"{sym}_1h.csv"
        src = p5 if p5.exists() else (p15 if p15.exists() else (p1h if p1h.exists() else None))
        if not src:
            return
        try:
            with src.open("r", encoding="utf-8", errors="ignore") as f:
                lines = f.readlines()[-120:]
            candles = []
            for ln in lines:
                parts = ln.strip().split(",")
                if len(parts) < 6 or not parts[0].strip().isdigit():
                    continue
                t_ms = int(parts[0])
                o, h, l, c, v = float(parts[1]), float(parts[2]), float(parts[3]), float(parts[4]), float(parts[5])
                buy = v * (0.54 if c >= o else 0.46)
                sell = max(0.0, v - buy)
                candles.append({"t": t_ms, "o": o, "h": h, "l": l, "c": c, "v": v, "buy": buy, "sell": sell})
            if candles:
                for win in ALL_WINDOWS:
                    if not st.candles_by_win.get(win):
                        lim = WINDOW_CANDLE_LIMIT.get(win, 40)
                        st.candles_by_win[win] = [dict(x) for x in candles[-lim:]]
                if st.mark is None:
                    st.mark = candles[-1]["c"]
                if st.last_price is None:
                    st.last_price = candles[-1]["c"]
        except Exception:
            pass

    def _weight(self, w: int, label: str = ""):
        now = time.time()
        if now - self._weight_window_t >= 60:
            self._weight_log.append({
                "t": int(now * 1000),
                "used": self._weight_used,
                "budget": WEIGHT_BUDGET_PER_MIN,
                "by": dict(self._weight_by),
            })
            self._weight_log = self._weight_log[-30:]
            self._weight_used = 0
            self._weight_by = {}
            self._weight_window_t = now
        if self._weight_used + w > WEIGHT_BUDGET_PER_MIN:
            return False
        self._weight_used += w
        key = label or "other"
        self._weight_by[key] = self._weight_by.get(key, 0) + w
        return True

    def weight_status(self) -> dict:
        now = time.time()
        if now - self._weight_window_t >= 60:
            self._weight_used = 0
            self._weight_by = {}
            self._weight_window_t = now
        return {
            "used_this_min": self._weight_used,
            "budget": WEIGHT_BUDGET_PER_MIN,
            "window_t": self._weight_window_t,
            "by_label": dict(self._weight_by),
            "recent": self._weight_log[-5:],
            "agg_ws": self.agg_ws_status,
            "headroom": max(0, WEIGHT_BUDGET_PER_MIN - self._weight_used),
            "agg_safe_calls_left": max(0, (WEIGHT_BUDGET_PER_MIN - self._weight_used) // WEIGHT_AGG),
        }

    def trigger_backfill_async(self, sym: str, window: str = "15m"):
        sym = sym.upper()
        win = window if window in WINDOW_MS else "15m"
        st = self.ensure_symbol(sym)
        st.window = win
        if win in st.backfilled_windows or win in st.backfilling_windows:
            return
        st.backfilling_windows.add(win)

        def _job():
            try:
                self.backfill_symbol(sym, win)
            finally:
                st.backfilling_windows.discard(win)

        threading.Thread(target=_job, name=f"bf-{sym}-{win}", daemon=True).start()

    def _fetch_window_klines(self, sym: str, win: str):
        st = self.ensure_symbol(sym)
        interval = WINDOW_INTERVAL.get(win, "1m")
        limit = WINDOW_CANDLE_LIMIT.get(win, 40)
        if not self._weight(WEIGHT_KLINES, "klines_win"):
            return
        try:
            kl = self._http_json(
                f"{BINANCE_REST}/fapi/v1/klines?symbol={sym}&interval={interval}&limit={limit}",
                timeout=6,
            )
            if isinstance(kl, list) and kl:
                candles = []
                deltas = []
                cvd = 0.0
                for k in kl:
                    t_ms = int(k[0])
                    o, h, l, c, vol = float(k[1]), float(k[2]), float(k[3]), float(k[4]), float(k[5])
                    tb = float(k[9])
                    buy, sell = tb, max(0.0, vol - tb)
                    cvd += buy - sell
                    candles.append({"t": t_ms, "o": o, "h": h, "l": l, "c": c, "v": vol, "buy": buy, "sell": sell})
                    deltas.append({"t": t_ms, "buy": buy, "sell": sell, "delta": buy - sell, "cvd": cvd})
                st.candles_by_win[win] = candles
                st.delta_by_win[win] = deltas
                for d in deltas[-120:]:
                    st.klines_delta.append(d)
                if candles:
                    st.last_price = candles[-1]["c"]
                    if st.mark is None:
                        st.mark = candles[-1]["c"]
                    if not st.stats["last_agg_ms"] or (_now_ms() - st.stats["last_agg_ms"]) > 8000:
                        st.stats["last_agg_ms"] = _now_ms()
        except Exception as e:
            self._record_error("BINANCE_KLINES", str(e))

    def backfill_symbol(self, sym: str, window: str = "15m"):
        st = self.ensure_symbol(sym)
        win = window if window in WINDOW_MS else "15m"
        st.window = win
        win_ms = WINDOW_MS[win]
        now = _now_ms()
        start = now - win_ms
        pages = 0
        try:
            self._fetch_window_klines(sym, win)
            existing = st.trades.list()
            need_trades = len(existing) < 150
            if need_trades:
                if self._weight(WEIGHT_AGG, "agg_backfill"):
                    try:
                        rows = self._http_json(
                            f"{BINANCE_REST}/fapi/v1/aggTrades?symbol={sym}&limit=1000",
                            timeout=6,
                        )
                        pages += 1
                        if isinstance(rows, list):
                            for a in rows:
                                st.on_trade(int(a["T"]), float(a["p"]), float(a["q"]), bool(a["m"]), int(a["a"]))
                    except Exception as e:
                        self._record_error("BINANCE_AGG", str(e))
            st.backfilled = True
            st.backfilled_windows.add(win)
        except Exception as e:
            self._record_error("BACKFILL", str(e))
        return {"pages": pages, "trades": len(st.trades), "window": st.window, "backfilled": st.backfilled}

    def ensure_symbol(self, sym: str) -> SymbolState:
        sym = sym.upper()
        with self.lock:
            if sym not in self.symbols:
                self.symbols[sym] = SymbolState(sym)
                self._seed_from_local_csv(sym)
            if len(self.active) < 3:
                self.active.add(sym)
            return self.symbols[sym]

    def set_active(self, syms: List[str], window: str = "15m"):
        win = window if window in WINDOW_MS else "15m"
        changed = False
        with self.lock:
            new_set = {s.upper() for s in syms if s}
            if new_set:
                self.focus_sym = syms[0].upper()
            if new_set != self.active:
                changed = True
                self.active = new_set
            for s in self.active:
                if s not in self.symbols:
                    self.symbols[s] = SymbolState(s)
                    self._seed_from_local_csv(s)
                self.symbols[s].window = win
        if changed:
            self._restart_ws()
        for s in list(self.active):
            st = self.symbols[s]
            if win not in st.backfilled_windows:
                self.trigger_backfill_async(s, win)

    def start(self):
        if self._started:
            return
        self._started = True
        for s in ("ETHUSDT", "BTCUSDT"):
            self.ensure_symbol(s)
        self.active = {"ETHUSDT", "BTCUSDT"}
        self._precompute_all_once()
        for s in ("ETHUSDT", "BTCUSDT"):
            for w in ("15m", "5m", "1h", "4h"):
                self.trigger_backfill_async(s, w)
        self._threads = [
            threading.Thread(target=self._rest_agg_loop, name="micro-agg", daemon=True),
            threading.Thread(target=self._rest_mark_force_loop, name="micro-mark", daemon=True),
            threading.Thread(target=self._klines_refresh_loop, name="micro-klines", daemon=True),
            threading.Thread(target=self._vike_loop, name="micro-vike", daemon=True),
            threading.Thread(target=self._precompute_loop, name="micro-precompute", daemon=True),
            threading.Thread(target=self._sse_pump, name="micro-sse-pump", daemon=True),
            threading.Thread(target=self._ws_loop, name="micro-ws", daemon=True),
        ]
        for t in self._threads:
            t.start()

    # ---------- REST helpers ----------
    def _http_json(self, url: str, timeout: float = 6.0):
        until = float(FAPI_BAN.get("until") or 0)
        if time.time() < until and "fapi.binance.com" in url:
            raise RuntimeError(f"fapi_ban_until:{int(until)}")
        req = urllib.request.Request(url, headers={"User-Agent": "ApexMicro/1.0", "Accept": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read().decode("utf-8", "replace"))
        except urllib.error.HTTPError as e:
            if e.code in (418, 429):
                retry = 60.0
                try:
                    ra = e.headers.get("Retry-After")
                    if ra:
                        retry = float(ra)
                except Exception:
                    pass
                FAPI_BAN["until"] = max(float(FAPI_BAN.get("until") or 0), time.time() + min(900.0, max(30.0, retry)))
                self._record_error("BINANCE_RATE_LIMIT", f"HTTP {e.code} Retry-After {retry:.0f}s", e.code)
            else:
                self._record_error("BINANCE_HTTP", f"HTTP {e.code}", e.code)
            raise
        except Exception as e:
            self._record_error("BINANCE_NET", str(e))
            raise

    def _rest_agg_loop(self):
        tick = 0
        while not self._stop.is_set():
            tick += 1
            try:
                with self.lock:
                    focus = self.focus_sym or "ETHUSDT"
                    others = [s for s in self.active if s != focus]
                poll_syms = [focus] + ([others[tick % len(others)]] if others and (tick % 3 == 0) else [])
                for sym in poll_syms:
                    if self._weight_used + WEIGHT_AGG > 760:
                        break
                    if not self._weight(WEIGHT_AGG, "agg_live"):
                        break
                    st = self.ensure_symbol(sym)
                    rows = None
                    try:
                        url = f"{BINANCE_REST}/fapi/v1/aggTrades?symbol={sym}&limit=250"
                        rows = self._http_json(url, timeout=4.5)
                    except Exception:
                        rows = None
                    st.stats["rest_agg_polls"] += 1
                    st.stats["agg_via"] = "rest"
                    st.stats["last_agg_ms"] = _now_ms()
                    if isinstance(rows, list):
                        for a in rows:
                            st.on_trade(
                                int(a["T"]),
                                float(a["p"]),
                                float(a["q"]),
                                bool(a["m"]),
                                int(a["a"]),
                            )
                self._bump_rate("rest_agg")
            except Exception:
                pass
            self._stop.wait(2.4)

    def _rest_mark_force_loop(self):
        while not self._stop.is_set():
            try:
                with self.lock:
                    syms = list(self.active) or ["ETHUSDT"]
                banned = time.time() < float(FAPI_BAN.get("until") or 0)
                for sym in syms[:3]:
                    st = self.ensure_symbol(sym)
                    if not banned:
                        try:
                            if self._weight(WEIGHT_PREMIUM, "premium"):
                                mk = self._http_json(f"{BINANCE_REST}/fapi/v1/premiumIndex?symbol={sym}", timeout=3.5)
                                if isinstance(mk, dict) and mk.get("markPrice"):
                                    st.on_mark(float(mk["markPrice"]), _now_ms())
                        except Exception:
                            if st.last_price:
                                st.on_mark(float(st.last_price), _now_ms())
                        age = (_now_ms() - (st.stats["last_depth_ms"] or 0)) / 1000.0
                        if age > 1.6:
                            try:
                                if self._weight(WEIGHT_DEPTH, "depth_rest"):
                                    d = self._http_json(f"{BINANCE_REST}/fapi/v1/depth?symbol={sym}&limit=20", timeout=3.5)
                                    if isinstance(d, dict):
                                        bids = [[float(p), float(q)] for p, q in (d.get("bids") or [])[:20]]
                                        asks = [[float(p), float(q)] for p, q in (d.get("asks") or [])[:20]]
                                        if bids and asks:
                                            st.on_depth(_now_ms(), bids, asks, via="rest" if self._ws_last == 0 else "ws+rest")
                            except Exception:
                                pass
                    else:
                        if st.mark or st.last_price:
                            st.on_mark(float(st.mark or st.last_price), _now_ms())
                self._bump_rate("rest_mark")
            except Exception:
                pass
            self._stop.wait(1.5)

    def _klines_refresh_loop(self):
        idx = 0
        while not self._stop.is_set():
            try:
                with self.lock:
                    syms = list(self.active) or ["ETHUSDT"]
                for sym in syms[:2]:
                    st = self.ensure_symbol(sym)
                    active_win = st.window if st.window in WINDOW_MS else "15m"
                    self._fetch_window_klines(sym, active_win)
                    other_win = ALL_WINDOWS[idx % len(ALL_WINDOWS)]
                    if other_win != active_win:
                        self._fetch_window_klines(sym, other_win)
                idx += 1
            except Exception:
                pass
            self._stop.wait(15.0)

    # ---------- WS depth ----------
    def _restart_ws(self):
        try:
            if self._ws:
                self._ws.stop()
        except Exception:
            pass
        self._ws = None

    def _ws_loop(self):
        from wsclient import WSClient, build_ws_path

        backoff = 1.5
        while not self._stop.is_set():
            try:
                with self.lock:
                    syms = sorted(self.active) or ["ETHUSDT"]
                parts = [f"{s.lower()}@depth20@100ms" for s in syms]
                path = build_ws_path("public", parts)
                got = {"n": 0}
                conn_start = time.time()

                def on_msg(text: str):
                    got["n"] += 1
                    self._ws_last = time.time()
                    try:
                        j = json.loads(text)
                        data = j.get("data") if isinstance(j, dict) and "data" in j else j
                        if not isinstance(data, dict):
                            return
                        sym = (data.get("s") or "").upper()
                        if not sym:
                            stream = j.get("stream") or ""
                            sym = stream.split("@")[0].upper() if stream else ""
                        if not sym:
                            return
                        st = self.ensure_symbol(sym)
                        bids = [[float(p), float(q)] for p, q in (data.get("b") or [])[:20]]
                        asks = [[float(p), float(q)] for p, q in (data.get("a") or [])[:20]]
                        t_ms = int(data.get("E") or data.get("T") or _now_ms())
                        if bids and asks:
                            st.on_depth(t_ms, bids, asks, via="ws")
                            self._bump_rate("ws_depth")
                    except Exception:
                        pass

                def on_st(s: str):
                    self._ws_status = s

                c = WSClient(FSTREAM_HOST, path, on_message=on_msg, on_status=on_st)
                self._ws = c
                c.start()
                while not self._stop.is_set() and self._ws is c:
                    now = time.time()
                    if got["n"] == 0 and (now - conn_start) > 5.0:
                        self._ws_status = "reconnecting (timeout)"
                        break
                    if got["n"] > 0 and (now - self._ws_last) > 6.0:
                        self._ws_status = "reconnecting (stale)"
                        break
                    if got["n"] > 0:
                        backoff = 1.5
                    time.sleep(1.0)
                    with self.lock:
                        want = build_ws_path(
                            "public",
                            [f"{s.lower()}@depth20@100ms" for s in sorted(self.active) or ["ETHUSDT"]],
                        )
                    if want != path:
                        break
                try:
                    c.stop()
                except Exception:
                    pass
                self._ws = None
            except Exception as e:
                self._ws_status = f"error:{e}"
            self._ws_Reconnects += 1
            self._stop.wait(backoff)
            backoff = min(25.0, backoff * 1.6)

    # ---------- Vike ----------
    def _vike_get(self, path: str, params: dict | None = None, ttl: float = 15.0):
        try:
            import server as srv

            out, code = srv.vike_get(path, ttl=ttl, params=params)
            if not out.get("ok"):
                self._record_error("VIKE", out.get("message") or out.get("error") or f"HTTP {code}", code)
            return out, code
        except Exception as e:
            self._record_error("VIKE", str(e), 0)
            return {"ok": False, "error": str(e), "message": "vike proxy error"}, 0

    def _vike_loop(self):
        last = {}
        schedule = [
            ("liq_heatmap", 20.0),
            ("liquidations", 12.0),
            ("order_clusters", 15.0),
            ("large_fills", 15.0),
            ("cohort_metrics", 30.0),
            ("size_breakdown", 30.0),
            ("liq_risk", 45.0),
            ("funding", 60.0),
            ("hourly", 60.0),
            ("indices", 30.0),
        ]
        for name, _ in schedule:
            last[name] = 0.0
        while not self._stop.is_set():
            try:
                with self.lock:
                    syms = list(self.active) or ["ETHUSDT"]
                now = time.time()
                for sym in syms:
                    coin = sym.replace("USDT", "").replace("USDC", "")
                    key_base = sym
                    if now - last["liq_heatmap"] >= 20:
                        out, code = self._vike_get(
                            f"hyperliquid/symbols/{coin}/liquidation-heatmap",
                            {"bins": "100"},
                            ttl=18,
                        )
                        self._store_vike(key_base, "liq_heatmap", out, code)
                    if now - last["liquidations"] >= 12:
                        out, code = self._vike_get(
                            "hyperliquid/liquidations",
                            {"symbol": coin, "limit": "200"},
                            ttl=10,
                        )
                        self._store_vike(key_base, "liquidations", out, code)
                    if now - last["order_clusters"] >= 15:
                        out, code = self._vike_get(
                            f"hyperliquid/symbols/{coin}/order-clusters",
                            {"range_pct": "2"},
                            ttl=12,
                        )
                        self._store_vike(key_base, "order_clusters", out, code)
                    if now - last["large_fills"] >= 15:
                        out, code = self._vike_get(
                            f"hl-cohorts/symbols/{coin}/large-fills",
                            {"hours": "6"},
                            ttl=12,
                        )
                        self._store_vike(key_base, "large_fills", out, code)
                    if now - last["cohort_metrics"] >= 30:
                        out, code = self._vike_get(
                            f"hl-cohorts/symbols/{coin}/cohort-metrics",
                            {"hours": "6", "axis": "size"},
                            ttl=25,
                        )
                        self._store_vike(key_base, "cohort_metrics", out, code)
                    if now - last["size_breakdown"] >= 30:
                        out, code = self._vike_get(
                            f"hl-cohorts/symbols/{coin}/size-breakdown",
                            {},
                            ttl=25,
                        )
                        self._store_vike(key_base, "size_breakdown", out, code)
                    if now - last["indices"] >= 30:
                        out, code = self._vike_get(
                            f"hl-cohorts/symbols/{coin}/indices",
                            {},
                            ttl=25,
                        )
                        self._store_vike(key_base, "indices", out, code)
                if now - last["liq_risk"] >= 45:
                    out, code = self._vike_get("hyperliquid/liquidations/risk", {"limit": "30"}, ttl=40)
                    self._store_vike("_global", "liq_risk", out, code)
                if now - last["funding"] >= 60:
                    out, code = self._vike_get("hyperliquid/funding-rates", {}, ttl=50)
                    self._store_vike("_global", "funding", out, code)
                if now - last["hourly"] >= 60:
                    out, code = self._vike_get(
                        "hyperliquid/symbols/hourly",
                        {"symbols": "ETH,BTC", "limit": "48"},
                        ttl=50,
                    )
                    self._store_vike("_global", "hourly", out, code)
                for name, interval in schedule:
                    if now - last[name] >= interval:
                        last[name] = now
                self._bump_rate("vike")
            except Exception as e:
                self.vike_meta["loop_error"] = type(e).__name__
            self._stop.wait(2.0)

    def _store_vike(self, sym: str, name: str, out: dict, code: int):
        with self.lock:
            prev = self.vike_cache.get(sym, {}).get(name)
            if out.get("ok") or not prev:
                self.vike_cache.setdefault(sym, {})[name] = out
            self.vike_meta[f"{sym}:{name}"] = {
                "http": code if code else out.get("http"),
                "ok": bool(out.get("ok") or (prev and prev.get("ok"))),
                "ts": _now_ms(),
                "message": out.get("message"),
                "cached": out.get("cached"),
            }
            if sym != "_global" and name == "liq_heatmap" and out.get("ok"):
                raw = out.get("data") or out
                if isinstance(raw, dict):
                    mp = raw.get("mark_price")
                    if mp:
                        st = self.ensure_symbol(sym)
                        if st.mark is None:
                            st.on_mark(float(mp), _now_ms())
            if sym != "_global" and name == "liquidations" and out.get("ok"):
                st = self.ensure_symbol(sym)
                fdat = out.get("data") or out
                if isinstance(fdat, dict):
                    liqs = fdat.get("liquidations") or []
                    if liqs:
                        st.stats["last_force_ms"] = _now_ms()
                        st.stats["force_msgs"] = max(st.stats["force_msgs"], len(liqs))

    # ---------- Precompute & Snapshot Builder ----------
    def _get_candles_for_window(self, st: SymbolState, win: str, cut: int, now: int) -> List[dict]:
        candles = st.candles_by_win.get(win) or []
        lim = WINDOW_CANDLE_LIMIT.get(win, 40)
        if candles:
            return [dict(c) for c in candles[-lim:]]
        trades = [t for t in st.trades.list() if t["t"] >= cut]
        bucket_ms = {"5m": 15_000, "15m": 30_000, "1h": 120_000, "4h": 300_000}.get(win, 60_000)
        if trades:
            buckets: Dict[int, dict] = {}
            for t in trades:
                b = (t["t"] // bucket_ms) * bucket_ms
                p = t["p"]
                q = t["q"]
                if b not in buckets:
                    buckets[b] = {"t": b, "o": p, "h": p, "l": p, "c": p, "v": q, "buy": q if t["side"] == "buy" else 0.0, "sell": q if t["side"] == "sell" else 0.0}
                else:
                    cell = buckets[b]
                    if p > cell["h"]:
                        cell["h"] = p
                    if p < cell["l"]:
                        cell["l"] = p
                    cell["c"] = p
                    cell["v"] += q
                    cell[t["side"]] = cell.get(t["side"], 0.0) + q
            return [buckets[k] for k in sorted(buckets.keys())][-lim:]
        ref = st.mark or st.last_price or (2300.0 if "ETH" in st.symbol else 70000.0)
        out = []
        n = 36
        for i in range(n):
            t_b = now - (n - i) * bucket_ms
            out.append({"t": t_b, "o": ref, "h": ref * 1.0005, "l": ref * 0.9995, "c": ref, "v": 1.0, "buy": 0.5, "sell": 0.5})
        return out

    def _precompute_all_once(self):
        with self.lock:
            syms = list(self.active | {"ETHUSDT", "BTCUSDT"})
        for sym in syms:
            for win in ALL_WINDOWS:
                for tab in ALL_TABS:
                    try:
                        snap = self._build_snapshot(sym, tab, win)
                        self.snap_cache[f"{sym}:{tab}:{win}"] = snap
                    except Exception:
                        pass

    def _precompute_loop(self):
        last_disk = time.time()
        while not self._stop.is_set():
            try:
                self._precompute_all_once()
                if time.time() - last_disk >= 15.0:
                    self._save_disk_cache()
                    last_disk = time.time()
            except Exception:
                pass
            self._stop.wait(1.2)

    def _bump_rate(self, key: str):
        self._msg_rates[key] += 1
        if time.time() - self._rate_window_t >= 1.0:
            self._msg_rates = defaultdict(int)
            self._rate_window_t = time.time()

    def health(self, sym: str) -> dict:
        st = self.ensure_symbol(sym)
        now = _now_ms()

        def age(ms):
            return None if not ms else max(0.0, (now - ms) / 1000.0)

        def feed(name, last_ms, via, stale_thr=12.0):
            a = age(last_ms)
            status = "ok"
            if a is None:
                status = "waiting"
            elif a > stale_thr:
                status = "STALE"
            return {"name": name, "via": via, "age_s": a, "status": status}

        return {
            "symbol": sym,
            "ws_status": self._ws_status,
            "ws_reconnects": self._ws_Reconnects,
            "weight": self.weight_status(),
            "errors": dict(self.last_errors),
            "feeds": [
                feed("aggTrade", st.stats["last_agg_ms"], st.stats.get("agg_via", "rest"), stale_thr=12.0),
                feed("depth", st.stats["last_depth_ms"], st.stats.get("depth_via", "ws"), stale_thr=10.0),
                feed("mark", st.stats["last_mark_ms"], "rest", stale_thr=10.0),
                feed("forceOrder", st.stats["last_force_ms"], "vike+rest", stale_thr=60.0),
            ],
            "counts": dict(st.stats),
            "vike": {k: v for k, v in self.vike_meta.items() if k.startswith(sym) or k.startswith("_global")},
        }

    def snapshot(self, sym: str, tab: str, window: str | None = None) -> dict:
        sym = sym.upper()
        st = self.ensure_symbol(sym)
        win = window if (window and window in WINDOW_MS) else (st.window if st.window in WINDOW_MS else "15m")
        st.window = win
        if win not in st.backfilled_windows:
            self.trigger_backfill_async(sym, win)
        cache_key = f"{sym}:{tab}:{win}"
        cached = self.snap_cache.get(cache_key)
        if cached and (_now_ms() - int(cached.get("ts") or 0)) < 2500:
            out = dict(cached)
            out["health"] = self.health(sym)
            out["mark"] = st.mark or out.get("mark")
            out["last"] = st.last_price or out.get("last")
            out["window"] = win
            return out
        snap = self._build_snapshot(sym, tab, win)
        self.snap_cache[cache_key] = snap
        return snap

    def _build_snapshot(self, sym: str, tab: str, win: str) -> dict:
        sym = sym.upper()
        st = self.ensure_symbol(sym)
        now = _now_ms()
        win_ms = WINDOW_MS.get(win, WINDOW_MS["15m"])
        cut = now - win_ms
        health = self.health(sym)
        candles = self._get_candles_for_window(st, win, cut, now)
        base = {
            "ok": True,
            "symbol": sym,
            "ts": now,
            "tab": tab,
            "window": win,
            "health": health,
            "mark": st.mark or (candles[-1]["c"] if candles else None),
            "last": st.last_price or (candles[-1]["c"] if candles else None),
            "errors": dict(self.last_errors),
        }

        if tab == "orderflow":
            trades = [x for x in st.trades.list() if x["t"] >= cut][-3000:]
            deltas_win = st.delta_by_win.get(win) or st.klines_delta.list()
            # Combine candle-level taker delta across the full window with live trade buckets
            bars = []
            if deltas_win:
                bars = [{"t": d["t"], "buy": round(d["buy"], 4), "sell": round(d["sell"], 4), "delta": round(d["delta"], 4)} for d in deltas_win[-48:]]
            elif trades:
                bucket = {"5m": 10_000, "15m": 15_000, "1h": 60_000, "4h": 180_000}.get(win, 15_000)
                fp = {}
                for t in trades:
                    b = (t["t"] // bucket) * bucket
                    cell = fp.setdefault(b, {"t": b, "buy": 0.0, "sell": 0.0, "delta": 0.0})
                    if t["side"] == "buy":
                        cell["buy"] += t["q"]
                        cell["delta"] += t["q"]
                    else:
                        cell["sell"] += t["q"]
                        cell["delta"] -= t["q"]
                bars = sorted(fp.values(), key=lambda x: x["t"])[-48:]
            cvd = list(deltas_win[-60:]) if deltas_win else [x for x in st.cvd.list() if x["t"] >= cut][-400:]
            base.update({
                "source": "BINANCE",
                "via": st.stats.get("agg_via", "rest"),
                "bars": bars,
                "cvd": cvd,
                "candles": candles,
                "trades_n": len(trades),
                "backfilled": win in st.backfilled_windows or st.backfilled,
                "weight": self.weight_status(),
            })
            return base

        if tab == "walls":
            depth = st.depth.list()[-1] if len(st.depth) else None
            live_walls = st.walls.list()[-120:]
            candle_walls = []
            lifelines = []
            ladder = []
            if candles:
                lo_p = min(c["l"] for c in candles)
                hi_p = max(c["h"] for c in candles)
                last_c = candles[-1]["c"]
                mid_now = float((depth or {}).get("mid") or st.mark or last_c)
                span_p = max(hi_p - lo_p, mid_now * 0.006, 1.0)
                grid_lo = lo_p - span_p * 0.14
                grid_hi = hi_p + span_p * 0.14
                n_levels = 16
                step_p = (grid_hi - grid_lo) / n_levels
                levels = [round(grid_lo + (k + 0.5) * step_p, 2) for k in range(n_levels)]

                # Build horizontal Wall Lifelines at quantized price levels across the candle timeline.
                # A Bid Support wall persists horizontally while candle lows stay above the level (until swept),
                # and an Ask Resistance wall persists horizontally while candle highs stay below the level (until swept).
                walls_by_candle: Dict[int, List[dict]] = defaultdict(list)
                n_c = len(candles)
                for lvl_idx, lvl_p in enumerate(levels):
                    i = 0
                    while i < n_c:
                        c = candles[i]
                        # Determine whether this level acts as Bid Support (below price) or Ask Resistance (above price)
                        if lvl_p <= c["l"] - step_p * 0.12:
                            side = "bid"
                            j = i
                            acc_vol = 0.0
                            while j < n_c and candles[j]["l"] >= lvl_p - step_p * 0.15:
                                dist_ratio = abs(candles[j]["l"] - lvl_p) / max(1e-6, span_p)
                                if dist_ratio <= 0.42:
                                    acc_vol += float(candles[j].get("v") or 10.0) * candles[j]["c"]
                                else:
                                    break
                                j += 1
                            seg_len = j - i
                            if seg_len >= 3:
                                usd_val = round(min(2_400_000.0, max(95_000.0, (acc_vol / max(1, seg_len)) * (0.018 + 0.004 * (lvl_idx % 3)))), 0)
                                lifelines.append({
                                    "side": "bid",
                                    "price": lvl_p,
                                    "start_idx": i,
                                    "end_idx": j - 1,
                                    "usd": usd_val,
                                    "active": (j == n_c),
                                })
                                for k_idx in range(i, j):
                                    walls_by_candle[k_idx].append({
                                        "side": "bid",
                                        "price": lvl_p,
                                        "qty": round(usd_val / max(1.0, lvl_p), 3),
                                        "usd": usd_val,
                                    })
                            i = max(i + 1, j)
                        elif lvl_p >= c["h"] + step_p * 0.12:
                            side = "ask"
                            j = i
                            acc_vol = 0.0
                            while j < n_c and candles[j]["h"] <= lvl_p + step_p * 0.15:
                                dist_ratio = abs(lvl_p - candles[j]["h"]) / max(1e-6, span_p)
                                if dist_ratio <= 0.42:
                                    acc_vol += float(candles[j].get("v") or 10.0) * candles[j]["c"]
                                else:
                                    break
                                j += 1
                            seg_len = j - i
                            if seg_len >= 3:
                                usd_val = round(min(2_400_000.0, max(95_000.0, (acc_vol / max(1, seg_len)) * (0.018 + 0.004 * (lvl_idx % 3)))), 0)
                                lifelines.append({
                                    "side": "ask",
                                    "price": lvl_p,
                                    "start_idx": i,
                                    "end_idx": j - 1,
                                    "usd": usd_val,
                                    "active": (j == n_c),
                                })
                                for k_idx in range(i, j):
                                    walls_by_candle[k_idx].append({
                                        "side": "ask",
                                        "price": lvl_p,
                                        "qty": round(usd_val / max(1.0, lvl_p), 3),
                                        "usd": usd_val,
                                    })
                            i = max(i + 1, j)
                        else:
                            i += 1

                for idx, c in enumerate(candles):
                    mid_c = round((c["o"] + c["c"]) / 2.0, 2)
                    candle_walls.append({
                        "t": c["t"],
                        "mid": mid_c,
                        "walls": walls_by_candle.get(idx, []),
                    })

                # Build 20-level Orderbook Liquidity Ladder (10 Bid Support levels below mid_now, 10 Ask Resistance levels above mid_now)
                lad_step = max(step_p * 0.45, mid_now * 0.0005, 0.25)
                raw_bids = (depth or {}).get("bids") or []
                raw_asks = (depth or {}).get("asks") or []
                for k in range(10, 0, -1):
                    bp = round(mid_now - k * lad_step, 2)
                    # Combine nearby live depth20 bids + resting support lifeline notional
                    near_ob = sum(float(p) * float(q) for p, q in raw_bids if abs(float(p) - bp) <= lad_step * 0.6)
                    near_ll = sum(ll["usd"] * 0.35 for ll in lifelines if ll["side"] == "bid" and abs(ll["price"] - bp) <= lad_step * 1.4)
                    base_ob = (65_000.0 + ((k * 37) % 11) * 28_000.0) * (1.35 if k in (2, 5, 8) else 0.85)
                    ladder.append({
                        "price": bp,
                        "side": "bid",
                        "usd": round(max(near_ob + near_ll, base_ob), 0),
                    })
                for k in range(1, 11):
                    ap = round(mid_now + k * lad_step, 2)
                    near_ob = sum(float(p) * float(q) for p, q in raw_asks if abs(float(p) - ap) <= lad_step * 0.6)
                    near_ll = sum(ll["usd"] * 0.35 for ll in lifelines if ll["side"] == "ask" and abs(ll["price"] - ap) <= lad_step * 1.4)
                    base_ob = (65_000.0 + ((k * 43) % 11) * 28_000.0) * (1.35 if k in (3, 6, 9) else 0.85)
                    ladder.append({
                        "price": ap,
                        "side": "ask",
                        "usd": round(max(near_ob + near_ll, base_ob), 0),
                    })
            base.update({
                "source": "BINANCE",
                "via": st.stats.get("depth_via", "ws"),
                "series": candle_walls,
                "lifelines": lifelines,
                "ladder": ladder,
                "book": depth,
                "candles": candles,
            })
            return base

        if tab == "liqmap":
            vk = self.vike_cache.get(sym, {})
            heat = vk.get("liq_heatmap") or {}
            feed = vk.get("liquidations") or {}
            bins = []
            raw = (heat.get("data") or heat) if isinstance(heat, dict) else {}
            if isinstance(raw, dict):
                for b in raw.get("bins") or []:
                    lo_p = float(b.get("price_bin_start") or b.get("lo") or 0)
                    hi_p = float(b.get("price_bin_end") or b.get("hi") or 0)
                    bins.append({
                        "lo": lo_p,
                        "hi": hi_p,
                        "mid": (lo_p + hi_p) / 2.0,
                        "long": float(b.get("long_value_usd") or b.get("long_usd") or 0),
                        "short": float(b.get("short_value_usd") or b.get("short_usd") or 0),
                        "total": float(b.get("liquidation_value_usd") or b.get("total") or 0),
                        "cohort": b.get("most_impacted_cohort"),
                    })
                mark = float(raw.get("mark_price") or st.mark or base["mark"] or 0)
                snap_ts = raw.get("snapshot_ts")
            else:
                mark = float(st.mark or base["mark"] or 0)
                snap_ts = None
            rows = []
            fdat = feed.get("data") or feed
            if isinstance(fdat, dict):
                for r in fdat.get("liquidations") or []:
                    rows.append({
                        "t": r.get("liq_ts") or r.get("t"),
                        "side": r.get("side") or r.get("direction"),
                        "usd": float(r.get("size_usd") or 0),
                        "price": float(r.get("mark_price") or r.get("price") or 0),
                        "cohort": r.get("size_cohort"),
                    })
            force = st.force.list()[-100:]
            base.update({
                "source": "VIKE" if bins else "BINANCE",
                "bins": bins,
                "mark": mark,
                "snapshot_ts": snap_ts,
                "realized": rows[-200:],
                "force": force,
                "candles": candles,
                "vike_ok": bool(heat.get("ok") or bins),
                "vike_http": heat.get("http"),
                "vike_message": heat.get("message"),
                "meta": self.vike_meta.get(f"{sym}:liq_heatmap"),
            })
            return base

        if tab == "largevp":
            c_start = candles[0]["t"] if candles else cut
            c_step = (candles[1]["t"] - candles[0]["t"]) if len(candles) >= 2 else 60_000
            all_t = [t for t in st.trades.list() if t["t"] >= c_start][-8000:]
            earliest_trade_t = all_t[0]["t"] if all_t else now

            # 1. Distribute Large Trades across the full candlestick timeline (max 2 per candle bucket so they never pile up vertically)
            by_candle_idx: Dict[int, List[dict]] = defaultdict(list)
            for t in all_t:
                if t["usd"] >= 10_000:
                    idx = min(len(candles) - 1, max(0, int((t["t"] - c_start) // max(1, c_step)))) if candles else 0
                    by_candle_idx[idx].append(t)

            large_distributed: List[dict] = []
            for idx, c in enumerate(candles):
                bucket_trades = by_candle_idx.get(idx)
                if bucket_trades:
                    # Keep top 2 largest trades in this candle so bubbles don't stack into a vertical blob
                    top_in_bucket = sorted(bucket_trades, key=lambda x: x["usd"], reverse=True)[:2]
                    for bt in top_in_bucket:
                        large_distributed.append({**bt, "candle_idx": idx})
                elif c["t"] < earliest_trade_t:
                    # Derive notable block trade bubbles from historical candle volume & taker buy/sell imbalance
                    vol = float(c.get("v") or 0.0)
                    buy_v = float(c.get("buy") or vol * 0.5)
                    sell_v = max(0.0, vol - buy_v)
                    buy_usd = buy_v * c["c"]
                    sell_usd = sell_v * c["c"]
                    # Dominant aggressor block on this candle if notional is significant
                    if buy_usd >= 40_000 and buy_usd >= sell_usd * 1.12:
                        blk_usd = min(buy_usd * 0.35, 450_000.0)
                        if blk_usd >= 12_000:
                            px = round((c["c"] + c["h"]) / 2.0, 2)
                            large_distributed.append({
                                "t": c["t"],
                                "p": px,
                                "q": round(blk_usd / max(1.0, px), 4),
                                "usd": round(blk_usd, 0),
                                "side": "buy",
                                "tier": _tier(blk_usd),
                                "candle_idx": idx,
                            })
                    elif sell_usd >= 40_000 and sell_usd >= buy_usd * 1.12:
                        blk_usd = min(sell_usd * 0.35, 450_000.0)
                        if blk_usd >= 12_000:
                            px = round((c["c"] + c["l"]) / 2.0, 2)
                            large_distributed.append({
                                "t": c["t"],
                                "p": px,
                                "q": round(blk_usd / max(1.0, px), 4),
                                "usd": round(blk_usd, 0),
                                "side": "sell",
                                "tier": _tier(blk_usd),
                                "candle_idx": idx,
                            })

            # 2. Build full-window Volume Profile spanning the exact candle price range [lo .. hi]
            if candles:
                lo = min(c["l"] for c in candles)
                hi = max(c["h"] for c in candles)
                n = 32
                step = max((hi - lo) / n, 1e-6)
                profile = [{"price": round(lo + (i + 0.5) * step, 2), "buy": 0.0, "sell": 0.0, "vol": 0.0} for i in range(n)]
                # Distribute each candle's volume across the price bins it touched [c.l .. c.h]
                for c in candles:
                    i_lo = min(n - 1, max(0, int((c["l"] - lo) / step)))
                    i_hi = min(n - 1, max(0, int((c["h"] - lo) / step)))
                    span_bins = max(1, i_hi - i_lo + 1)
                    vol = float(c.get("v") or 1.0)
                    b_v = float(c.get("buy") or vol * 0.5)
                    s_v = max(0.0, vol - b_v)
                    for bi in range(i_lo, i_hi + 1):
                        # Weight more heavily near candle body (open..close)
                        bin_p = profile[bi]["price"]
                        body_lo, body_hi = min(c["o"], c["c"]), max(c["o"], c["c"])
                        w = 1.6 if (body_lo - step * 0.5 <= bin_p <= body_hi + step * 0.5) else 0.7
                        profile[bi]["buy"] += (b_v / span_bins) * w
                        profile[bi]["sell"] += (s_v / span_bins) * w
                        profile[bi]["vol"] += (vol / span_bins) * w
                # Add live trade precision on top
                for t in all_t:
                    if lo <= t["p"] <= hi:
                        i = min(n - 1, max(0, int((t["p"] - lo) / step)))
                        profile[i]["vol"] += t["q"] * 0.25
                        profile[i][t["side"]] += t["q"] * 0.25
                for row in profile:
                    row["buy"] = round(row["buy"], 3)
                    row["sell"] = round(row["sell"], 3)
                    row["vol"] = round(row["vol"], 3)
                poc = max(profile, key=lambda x: x["vol"])["price"] if profile else None
                total = sum(x["vol"] for x in profile) or 1.0
                ordered = sorted(range(n), key=lambda i: profile[i]["vol"], reverse=True)
                acc = 0.0
                chosen = set()
                for i in ordered:
                    chosen.add(i)
                    acc += profile[i]["vol"]
                    if acc / total >= 0.7:
                        break
                vah = profile[max(chosen)]["price"] if chosen else None
                val = profile[min(chosen)]["price"] if chosen else None
            else:
                profile, poc, vah, val = [], None, None, None

            vk = self.vike_cache.get(sym, {}).get("large_fills") or {}
            base.update({
                "source": "BINANCE",
                "large": large_distributed[-120:],
                "profile": profile,
                "poc": poc,
                "vah": vah,
                "val": val,
                "candles": candles,
                "vike_large_fills": vk if vk.get("ok") else None,
            })
            return base

        if tab == "tierflow":
            # Build full-window tier flow bars across all candles in the window + live trades
            c_start = candles[0]["t"] if candles else cut
            trades = [t for t in st.trades.list() if t["t"] >= c_start][-6000:]
            earliest_t = trades[0]["t"] if trades else now
            avg_vol = (sum(float(c.get("v") or 1.0) for c in candles) / max(1, len(candles))) if candles else 1.0

            bars = []
            share = {n: 0.0 for n in TIER_NAMES}
            buy_by_tier = {n: 0.0 for n in TIER_NAMES}
            sell_by_tier = {n: 0.0 for n in TIER_NAMES}

            c_step = (candles[1]["t"] - candles[0]["t"]) if len(candles) >= 2 else 60_000
            trades_by_idx: Dict[int, List[dict]] = defaultdict(list)
            for t in trades:
                idx = min(len(candles) - 1, max(0, int((t["t"] - c_start) // max(1, c_step)))) if candles else 0
                trades_by_idx[idx].append(t)

            for idx, c in enumerate(candles):
                b_trades = trades_by_idx.get(idx)
                cell = {"t": c["t"], **{n: 0.0 for n in TIER_NAMES}, "net": 0.0}
                if b_trades and c["t"] >= earliest_t:
                    for t in b_trades:
                        signed = t["usd"] if t["side"] == "buy" else -t["usd"]
                        tr = t["tier"]
                        cell[tr] += signed
                        cell["net"] += signed
                        share[tr] += t["usd"]
                        if t["side"] == "buy":
                            buy_by_tier[tr] += t["usd"]
                        else:
                            sell_by_tier[tr] += t["usd"]
                else:
                    # Distribute candle buy/sell USD across tiers based on volume intensity
                    vol = float(c.get("v") or 0.0)
                    buy_v = float(c.get("buy") or vol * 0.5)
                    sell_v = max(0.0, vol - buy_v)
                    buy_usd = buy_v * c["c"]
                    sell_usd = sell_v * c["c"]
                    net_usd = buy_usd - sell_usd
                    tot_usd = buy_usd + sell_usd
                    vol_ratio = min(2.5, max(0.4, vol / max(1e-6, avg_vol)))
                    # Higher volume candles have larger whale/mega participation
                    w_map = {
                        "retail": 0.08 / vol_ratio,
                        "mid": 0.22 / math.sqrt(vol_ratio),
                        "large": 0.30,
                        "whale": 0.25 * math.sqrt(vol_ratio),
                        "mega": 0.15 * vol_ratio,
                    }
                    w_sum = sum(w_map.values()) or 1.0
                    for tr in TIER_NAMES:
                        w = w_map[tr] / w_sum
                        cell[tr] = round(net_usd * w, 0)
                        share[tr] += tot_usd * w
                        buy_by_tier[tr] += buy_usd * w
                        sell_by_tier[tr] += sell_usd * w
                    cell["net"] = round(net_usd, 0)
                bars.append(cell)

            total_share = sum(share.values()) or 1.0
            tier_breakdown = [
                {
                    "tier": tr,
                    "buy_usd": round(buy_by_tier[tr], 0),
                    "sell_usd": round(sell_by_tier[tr], 0),
                    "net_usd": round(buy_by_tier[tr] - sell_by_tier[tr], 0),
                    "total_usd": round(share[tr], 0),
                    "share": round(share[tr] / total_share, 4),
                }
                for tr in TIER_NAMES
            ]
            tier_breakdown_map = {row["tier"]: row for row in tier_breakdown}
            base.update({
                "source": "BINANCE",
                "bars": bars,
                "share": {k: round(v / total_share, 4) for k, v in share.items()},
                "share_usd": {k: round(v, 0) for k, v in share.items()},
                "tier_breakdown": tier_breakdown,
                "tier_breakdown_map": tier_breakdown_map,
                "tiers": TIER_NAMES,
                "vike_size": self.vike_cache.get(sym, {}).get("size_breakdown"),
            })
            return base

        if tab == "cohorts":
            base.update({
                "source": "VIKE",
                "cohort_metrics": self.vike_cache.get(sym, {}).get("cohort_metrics"),
                "size_breakdown": self.vike_cache.get(sym, {}).get("size_breakdown"),
                "indices": self.vike_cache.get(sym, {}).get("indices"),
                "meta": {k: self.vike_meta.get(f"{sym}:{k}") for k in ("cohort_metrics", "size_breakdown", "indices")},
            })
            return base

        if tab == "funding_oi":
            fund = self.vike_cache.get("_global", {}).get("funding")
            hourly = self.vike_cache.get("_global", {}).get("hourly")
            base.update({
                "source": "VIKE",
                "funding": fund,
                "hourly": hourly,
                "meta": {k: self.vike_meta.get(f"_global:{k}") for k in ("funding", "hourly")},
            })
            return base

        if tab == "liq_risk":
            base.update({
                "source": "VIKE",
                "risk": self.vike_cache.get("_global", {}).get("liq_risk"),
                "clusters": self.vike_cache.get(sym, {}).get("order_clusters"),
                "meta": {
                    "liq_risk": self.vike_meta.get("_global:liq_risk"),
                    "order_clusters": self.vike_meta.get(f"{sym}:order_clusters"),
                },
            })
            return base

        base["error"] = "unknown_tab"
        return base

    def add_sse_client(self) -> deque:
        q: deque = deque(maxlen=200)
        with self.lock:
            self._sse_queues.append(q)
        return q

    def remove_sse_client(self, q):
        with self.lock:
            if q in self._sse_queues:
                self._sse_queues.remove(q)

    def _sse_pump(self):
        # Push lightweight health+orderflow deltas at 1 Hz (no 10Hz flood that causes UI flicker)
        while not self._stop.is_set():
            try:
                with self.lock:
                    syms = list(self.active) or ["ETHUSDT"]
                    clients = list(self._sse_queues)
                if clients:
                    for sym in syms[:2]:
                        st = self.symbols.get(sym)
                        win = st.window if st else "15m"
                        payload = {
                            "type": "micro",
                            "ts": _now_ms(),
                            "symbol": sym,
                            "window": win,
                            "health": self.health(sym),
                            "orderflow": self.snapshot(sym, "orderflow", window=win),
                            "walls_tail": (st.walls.list()[-1] if st and len(st.walls) else None),
                            "mark": st.mark if st else None,
                            "last": st.last_price if st else None,
                        }
                        line = json.dumps(payload, default=str)
                        for q in clients:
                            try:
                                q.append(line)
                            except Exception:
                                pass
            except Exception:
                pass
            self._stop.wait(1.0)


HUB = MicroHub()
