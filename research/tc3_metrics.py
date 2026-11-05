"""Summarize a completed pre-holdout replay without using holdout data."""
import collections
import datetime
import json
import pathlib
import sys

START = 1735689600000
VAL = 1751328000000  # 2025-07-01 UTC (Rules §4.3)
END = 1777593600000  # 2026-05-01 UTC


def segment(trades, start, end):
    return [t for t in trades if t["exit_kind"] != "EndOfData" and start <= t["signal_ms"] <= t["entry_ms"] <= t["exit_ms"] < end]


def metrics(trades):
    if not trades:
        return {"trades": 0, "win_rate": None, "profit_factor": None, "expectancy_r": None, "max_dd_usd": None, "max_dd_r": None}
    profits = [t["pnl_net"] for t in trades]
    gross_profit = sum(max(0, p) for p in profits)
    gross_loss = -sum(min(0, p) for p in profits)
    eq = peak = drawdown = eq_r = peak_r = dd_r = 0.0
    for trade in sorted(trades, key=lambda t: (t["exit_ms"], t["entry_ms"])):
        eq += trade["pnl_net"]
        eq_r += trade["r"]
        peak = max(peak, eq)
        peak_r = max(peak_r, eq_r)
        drawdown = max(drawdown, peak - eq)
        dd_r = max(dd_r, peak_r - eq_r)
    return {"trades": len(trades), "win_rate": sum(p > 0 for p in profits) / len(trades), "profit_factor": gross_profit / gross_loss if gross_loss else None, "expectancy_r": sum(t["r"] for t in trades) / len(trades), "max_dd_usd": drawdown, "max_dd_r": dd_r}


def summarize(report):
    if report.get("start") != "2025-01-01T00:00:00Z" or report.get("end") != "2026-05-01T00:00:00Z":
        raise ValueError("only the specified pre-holdout replay is supported")
    all_trades = segment(report["trades"], START, END)
    is_trades = segment(report["trades"], START, VAL)
    val_trades = segment(report["trades"], VAL, END)
    by_symbol, by_month = collections.defaultdict(list), collections.defaultdict(list)
    for trade in all_trades:
        by_symbol[trade["symbol"]].append(trade)
        month = datetime.datetime.fromtimestamp(trade["exit_ms"] / 1000, datetime.timezone.utc).strftime("%Y-%m")
        by_month[month].append(trade)
    groups = lambda g: {k: {"trades": len(v), "trade_share": len(v) / len(all_trades), "net_usd": sum(t["pnl_net"] for t in v)} for k, v in sorted(g.items())}
    return {"signals": report["signals"], "all": metrics(all_trades), "is": metrics(is_trades), "val": metrics(val_trades), "by_symbol": groups(by_symbol), "by_month": groups(by_month), "data_gaps": report["data_gaps"], "excluded_cross_boundary_or_forced": len(report["trades"]) - len(is_trades) - len(val_trades), "note": "VAL may be research-contaminated; PF null means no losses or no trades, not zero"}


if __name__ == "__main__":
    path = pathlib.Path(sys.argv[1])
    report = json.loads(path.read_text(encoding="utf-8"))
    print(json.dumps(summarize(report), indent=2))
