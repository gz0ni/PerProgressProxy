"""Proxy reachability checks: connect through a proxy from this machine."""

from __future__ import annotations

import base64
import concurrent.futures
import re
import socket
import struct
import time
from dataclasses import dataclass
from pathlib import Path

from .specs import Proxy

ECHO_HOST = "checkip.amazonaws.com"
ECHO_PORT = 80
ECHO_PATH = "/"

IP_RE = re.compile(r"^\d{1,3}(?:\.\d{1,3}){3}$")


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("closed")
        buf += chunk
    return buf


def _open_tunnel(proxy: Proxy, host: str, port: int, timeout: float) -> socket.socket:
    s = socket.create_connection((proxy.host, proxy.port), timeout)
    s.settimeout(timeout)
    try:
        if proxy.scheme == "socks5":
            methods = b"\x00\x02" if proxy.user else b"\x00"
            s.sendall(b"\x05" + bytes([len(methods)]) + methods)
            ver, method = _recv_exact(s, 2)
            if ver != 5 or method == 0xFF:
                raise ConnectionError("no acceptable auth method")
            if method == 2:
                u = proxy.user.encode()[:255]
                p = proxy.pass_.encode()[:255]
                s.sendall(b"\x01" + bytes([len(u)]) + u + bytes([len(p)]) + p)
                _v, status = _recv_exact(s, 2)
                if status != 0:
                    raise PermissionError("auth rejected")
            name = host.encode()
            s.sendall(b"\x05\x01\x00\x03" + bytes([len(name)]) + name + struct.pack("!H", port))
            _ver, rep, _rsv, atyp = _recv_exact(s, 4)
            if _ver != 5 or rep != 0:
                raise ConnectionError(f"socks5 reply {rep}")
            if atyp == 1:
                _recv_exact(s, 4)
            elif atyp == 3:
                _recv_exact(s, _recv_exact(s, 1)[0])
            elif atyp == 4:
                _recv_exact(s, 16)
            _recv_exact(s, 2)
        elif proxy.scheme == "socks4":
            name = host.encode()
            req = struct.pack("!BBH", 4, 1, port) + b"\x00\x00\x00\x01" + b"\x00" + name + b"\x00"
            s.sendall(req)
            reply = _recv_exact(s, 8)
            if reply[1] != 0x5A:
                raise ConnectionError(f"socks4 reply {reply[1]}")
        else:
            auth = ""
            if proxy.user:
                token = base64.b64encode(f"{proxy.user}:{proxy.pass_}".encode()).decode()
                auth = f"Proxy-Authorization: Basic {token}\r\n"
            target = f"{host}:{port}"
            s.sendall(f"CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{auth}\r\n".encode())
            head = b""
            while b"\r\n\r\n" not in head:
                chunk = s.recv(1)
                if not chunk:
                    raise ConnectionError("proxy closed on CONNECT")
                head += chunk
            if b" 200" not in head.split(b"\r\n", 1)[0]:
                raise ConnectionError(head.split(b"\r\n", 1)[0].decode("latin1"))
        return s
    except BaseException:
        s.close()
        raise


def fetch_via(proxy: Proxy, host: str = ECHO_HOST, port: int = ECHO_PORT, timeout: float = 12.0) -> str:
    s = _open_tunnel(proxy, host, port, timeout)
    try:
        s.sendall(
            f"GET {ECHO_PATH} HTTP/1.0\r\nHost: {host}\r\n"
            f"User-Agent: ppp\r\nConnection: close\r\n\r\n".encode()
        )
        chunks = []
        while True:
            chunk = s.recv(4096)
            if not chunk:
                break
            chunks.append(chunk)
        body = b"".join(chunks)
        head, _, payload = body.partition(b"\r\n\r\n")
        status = head.split(b"\r\n", 1)[0] if head else b""
        if b" 200" not in status:
            raise ConnectionError(status.decode("latin1", "replace") or "empty reply")
        return payload.decode("latin1", "replace").strip()
    finally:
        s.close()


def direct_ip(timeout: float = 6.0) -> str | None:
    try:
        s = socket.create_connection((ECHO_HOST, ECHO_PORT), timeout)
        s.settimeout(timeout)
        try:
            s.sendall(f"GET / HTTP/1.0\r\nHost: {ECHO_HOST}\r\nConnection: close\r\n\r\n".encode())
            data = b""
            while True:
                chunk = s.recv(4096)
                if not chunk:
                    break
                data += chunk
        finally:
            s.close()
        body = data.split(b"\r\n\r\n", 1)[-1].decode("latin1", "replace").strip()
        return body if IP_RE.match(body) else (body[:40] or None)
    except OSError:
        return None


@dataclass
class TestResult:
    idx: int
    proxy: Proxy
    ok: bool
    exit_ip: str = ""
    ms: int = 0
    note: str = ""


def test_one(idx: int, proxy: Proxy, timeout: float) -> TestResult:
    started = time.monotonic()
    try:
        ip = fetch_via(proxy, timeout=timeout)
        ms = int((time.monotonic() - started) * 1000)
        if not IP_RE.match(ip):
            return TestResult(idx, proxy, False, ip[:32], ms, "bad answer")
        return TestResult(idx, proxy, True, ip, ms)
    except PermissionError as exc:
        return TestResult(idx, proxy, False, "", int((time.monotonic() - started) * 1000), str(exc))
    except (OSError, ConnectionError) as exc:
        return TestResult(idx, proxy, False, "", int((time.monotonic() - started) * 1000), str(exc) or "failed")


def _health_path() -> Path:
    from .paths import STATE_DIR

    return STATE_DIR / "health.json"


def load_health() -> dict[str, dict]:
    import json

    path = _health_path()
    if not path.exists():
        return {}
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def save_health(results: list[TestResult]) -> None:
    import json
    import time as _time

    from .paths import STATE_DIR

    data = load_health()
    now = int(_time.time())
    for r in results:
        data[r.proxy.label] = {
            "ok": r.ok,
            "ms": r.ms,
            "ip": r.exit_ip,
            "note": r.note,
            "ts": now,
        }
    try:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        _health_path().write_text(json.dumps(data, indent=1), encoding="utf-8")
    except OSError:
        pass


def test_all(entries: list[tuple[int, str, Proxy]], concurrency: int = 24, timeout: float = 12.0) -> list[TestResult]:
    results: list[TestResult] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, concurrency)) as pool:
        futures = [pool.submit(test_one, idx, p, timeout) for idx, _t, p in entries]
        for fut in concurrent.futures.as_completed(futures):
            results.append(fut.result())
    results.sort(key=lambda r: r.idx)
    return results
