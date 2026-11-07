
"""Minimal RFC6455 WebSocket client (stdlib only) + Binance combined-stream helper."""
from __future__ import annotations
import base64, hashlib, json, os, socket, ssl, struct, threading, time
from typing import Callable, Optional, Set

class WSClient:
    def __init__(self, host: str, path: str, on_message: Callable[[str], None], on_status: Callable[[str], None] | None = None):
        self.host = host
        self.path = path
        self.on_message = on_message
        self.on_status = on_status or (lambda s: None)
        self._sock = None
        self._stop = threading.Event()
        self._thread = None
        self._send_lock = threading.Lock()
        self._backoff = 1.0

    def start(self):
        if self._thread and self._thread.is_alive():
            return
        self._stop.clear()
        self._thread = threading.Thread(target=self._run, name="wsclient", daemon=True)
        self._thread.start()

    def stop(self):
        self._stop.set()
        try:
            if self._sock:
                self._sock.close()
        except Exception:
            pass

    def send_text(self, text: str):
        data = text.encode("utf-8")
        # client frames MUST be masked
        header = bytearray()
        header.append(0x81)
        n = len(data)
        if n < 126:
            header.append(0x80 | n)
        elif n < 65536:
            header.append(0x80 | 126)
            header += struct.pack(">H", n)
        else:
            header.append(0x80 | 127)
            header += struct.pack(">Q", n)
        mask = os.urandom(4)
        header += mask
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        with self._send_lock:
            if self._sock:
                self._sock.sendall(header + masked)

    def _handshake(self, sock):
        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET {self.path} HTTP/1.1\r\n"
            f"Host: {self.host}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "\r\n"
        ).encode()
        sock.sendall(req)
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(4096)
            if not chunk:
                raise ConnectionError("handshake closed")
            buf += chunk
        head, rest = buf.split(b"\r\n\r\n", 1)
        status = head.split(b"\r\n", 1)[0]
        if b"101" not in status:
            raise ConnectionError("handshake failed: " + status.decode("utf-8", "replace"))
        return rest

    def _run(self):
        while not self._stop.is_set():
            try:
                self.on_status("connecting")
                raw = socket.create_connection((self.host, 443), timeout=10)
                ctx = ssl.create_default_context()
                sock = ctx.wrap_socket(raw, server_hostname=self.host)
                sock.settimeout(30)
                leftover = self._handshake(sock)
                self._sock = sock
                self._backoff = 1.0
                self.on_status("open")
                self._read_loop(sock, leftover)
            except Exception as e:
                self.on_status(f"error:{e}")
            finally:
                try:
                    if self._sock:
                        self._sock.close()
                except Exception:
                    pass
                self._sock = None
            if self._stop.is_set():
                break
            self.on_status(f"reconnect_in_{self._backoff:.0f}s")
            self._stop.wait(self._backoff)
            self._backoff = min(30.0, self._backoff * 1.7)

    def _pong(self, sock, payload: bytes):
        header = bytearray([0x8A, 0x80 | len(payload)])
        mask = os.urandom(4)
        header += mask
        body = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        with self._send_lock:
            sock.sendall(header + body)

    def _read_loop(self, sock, data: bytes):
        while not self._stop.is_set():
            while len(data) < 2:
                chunk = sock.recv(65536)
                if not chunk:
                    raise ConnectionError("eof")
                data += chunk
            b1, b2 = data[0], data[1]
            opcode = b1 & 0x0F
            masked = b2 & 0x80
            ln = b2 & 0x7F
            off = 2
            if ln == 126:
                while len(data) < 4:
                    data += sock.recv(65536)
                ln = struct.unpack(">H", data[2:4])[0]
                off = 4
            elif ln == 127:
                while len(data) < 10:
                    data += sock.recv(65536)
                ln = struct.unpack(">Q", data[2:10])[0]
                off = 10
            if masked:
                while len(data) < off + 4:
                    data += sock.recv(65536)
                mask = data[off:off+4]
                off += 4
            else:
                mask = b""
            while len(data) < off + ln:
                data += sock.recv(65536)
            payload = bytearray(data[off:off+ln])
            data = data[off+ln:]
            if masked:
                for i in range(len(payload)):
                    payload[i] ^= mask[i % 4]
            if opcode == 0x8:  # close
                raise ConnectionError("closed by peer")
            if opcode == 0x9:
                self._pong(sock, bytes(payload))
                continue
            if opcode == 0xA:
                continue
            if opcode in (0x1, 0x2):
                try:
                    self.on_message(payload.decode("utf-8", "replace") if opcode == 0x1 else payload.hex())
                except Exception:
                    pass


class BinanceStreamHub:
    """Shared upstream WS; fans out JSON lines to SSE subscribers. Dynamic SUBSCRIBE."""
    def __init__(self):
        self._subs: Set[str] = set()
        self._clients = []  # list of queues (list append)
        self._lock = threading.RLock()
        self._status = "idle"
        self._client: Optional[WSClient] = None
        self._last_msg = 0.0

    @property
    def status(self):
        return {"status": self._status, "streams": sorted(self._subs), "last_msg_age_ms": int((time.time()-self._last_msg)*1000) if self._last_msg else None}

    def _ensure(self):
        if self._client:
            return
        # start with empty combined endpoint that accepts SUBSCRIBE
        self._client = WSClient(
            "fstream.binance.com",
            "/stream?streams=ethusdt@markPrice@1s",
            on_message=self._on_msg,
            on_status=self._on_status,
        )
        self._client.start()

    def _on_status(self, s: str):
        self._status = s
        if s == "open":
            # re-subscribe desired
            with self._lock:
                streams = list(self._subs)
            if streams:
                self._client.send_text(json.dumps({"method": "SUBSCRIBE", "params": streams, "id": int(time.time())}))

    def _on_msg(self, text: str):
        self._last_msg = time.time()
        with self._lock:
            clients = list(self._clients)
        for q in clients:
            try:
                q.append(text)
            except Exception:
                pass

    def set_streams(self, streams: Set[str]):
        streams = {s.lower() for s in streams if s}
        self._ensure()
        with self._lock:
            add = streams - self._subs
            rem = self._subs - streams
            self._subs = streams
        if self._status == "open" and self._client:
            if add:
                self._client.send_text(json.dumps({"method": "SUBSCRIBE", "params": sorted(add), "id": int(time.time()*1000)}))
            if rem:
                self._client.send_text(json.dumps({"method": "UNSUBSCRIBE", "params": sorted(rem), "id": int(time.time()*1000)+1}))

    def add_client(self):
        q = []
        with self._lock:
            self._clients.append(q)
        self._ensure()
        return q

    def remove_client(self, q):
        with self._lock:
            if q in self._clients:
                self._clients.remove(q)

HUB = BinanceStreamHub()
