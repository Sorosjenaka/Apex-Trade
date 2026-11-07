"""Red contract test for the Vike-style market monitor composition."""
from __future__ import annotations

import json
import urllib.request


BASE = "http://127.0.0.1:8787"


def test_command_snapshot_exposes_all_terminal_panels():
    url = BASE + "/api/micro/snapshot?symbol=ETHUSDT&tab=command&window=15m"
    with urllib.request.urlopen(url, timeout=30) as response:
        payload = json.loads(response.read())

    for field in (
        "volume_profile",
        "orderbook",
        "market_stats",
        "liquidation_map",
        "cohort_board",
    ):
        assert field in payload, f"missing Vike terminal panel: {field}"

    assert payload["analysis"]["execution"] == "DISABLED_READ_ONLY"
    assert len(payload.get("candles") or []) >= 10
    assert len(payload["orderbook"].get("asks") or []) >= 1
    assert len(payload["orderbook"].get("bids") or []) >= 1
