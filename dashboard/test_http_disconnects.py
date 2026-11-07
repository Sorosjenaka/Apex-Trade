"""Regression contract for normal browser disconnects on the local dashboard."""
from __future__ import annotations

import sys
from pathlib import Path
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parent))
import server


def test_safe_json_write_ignores_aborted_client():
    class FakeStream:
        def write(self, body):
            raise ConnectionAbortedError(10053, "client closed")

    class FakeHandler(server.H):
        wfile = FakeStream()

    # Calling the low-level writer must not raise when a browser aborts a request.
    assert FakeHandler._safe_write(FakeHandler, b"x") is False


def test_server_root_and_health_are_live():
    for path in ("/", "/api/micro/health?symbol=ETHUSDT"):
        with urllib.request.urlopen("http://127.0.0.1:8787" + path, timeout=15) as response:
            assert response.status == 200
            assert response.read()
