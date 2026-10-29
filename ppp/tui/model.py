"""Screen state for the interactive console: files, filters, selection, tests.

All file access goes through ppp.store / ppp.processes / ppp.netcheck so the
interactive UI never writes anything the DLL does not already understand.
"""

from __future__ import annotations

import concurrent.futures
import queue
import threading
import time
import urllib.request

from .. import netcheck, processes, store
from ..specs import Proxy

REFRESH_SEC = 0.5
FILTERS = ("all", "working", "untested", "failed", "same")
URL_PREFIXES = ("http://", "https://")
MAX_DOWNLOAD = 5 * 1024 * 1024
USER_AGENT = "ppp-tui/1"


def is_url(text: str) -> bool:
    return text.strip().lower().startswith(URL_PREFIXES)


def download(url: str, timeout: float = 20.0) -> str:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        raw = response.read(MAX_DOWNLOAD + 1)
    if len(raw) > MAX_DOWNLOAD:
        raise ValueError("list is larger than 5 MB")
    return raw.decode("utf-8", "replace")


class TestRun:
    """Runs netcheck.test_all in a background thread and streams results."""

    def __init__(self, rows: list[tuple[int, str, Proxy]], concurrency: int = 24, timeout: float = 12.0) -> None:
        self.total = len(rows)
        self.done = 0
        self.good = 0
        self.direct = ""
        self.finished = False
        self._rows = rows
        self._concurrency = concurrency
        self._timeout = timeout
        self._queue: queue.Queue = queue.Queue()
        self._thread = threading.Thread(target=self._work, daemon=True)
        self._thread.start()

    def _work(self) -> None:
        try:
            self.direct = netcheck.direct_ip() or ""
            self._queue.put(("direct", self.direct))
        except Exception:
            pass
        collected: list[netcheck.TestResult] = []
        try:
            with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, self._concurrency)) as pool:
                futures = {
                    pool.submit(netcheck.test_one, idx, proxy, self._timeout): idx
                    for idx, _text, proxy in self._rows
                }
                for future in concurrent.futures.as_completed(futures):
                    try:
                        result = future.result()
                    except Exception:
                        continue
                    collected.append(result)
                    self._queue.put(("result", result))
        finally:
            if collected:
                netcheck.save_health(collected)
            self._queue.put(("done", None))

    def drain(self) -> list[netcheck.TestResult]:
        out: list[netcheck.TestResult] = []
        while True:
            try:
                kind, payload = self._queue.get_nowait()
            except queue.Empty:
                return out
            if kind == "done":
                self.finished = True
            elif kind == "direct":
                self.direct = payload or ""
            else:
                self.done += 1
                if payload.ok and not (self.direct and payload.exit_ip == self.direct):
                    self.good += 1
                out.append(payload)


class Screen:
    """Live snapshot of the file contract plus filter/selection/scroll."""

    def __init__(self) -> None:
        self.cfg: dict[str, str] = {}
        self.rows: list[tuple[int, str, Proxy]] = []
        self.health: dict[str, dict] = {}
        self.states: list[processes.ProcessState] = []
        self.current = 0
        self.direct = ""
        self.filter = "all"
        self.query = ""
        self.selected = 0
        self.scroll = 0
        self.message = ""
        self.notice = ""
        self.test: TestRun | None = None
        self._last = 0.0

    # -- files ------------------------------------------------------------

    def reload(self, force: bool = False) -> None:
        now = time.monotonic()
        if not force and now - self._last < REFRESH_SEC:
            return
        self._last = now
        self.cfg = store.read_config()
        self.rows = store.proxies()
        if self.test is None:
            self.health = netcheck.load_health()
        self.states = processes.read_states()
        self.current = _int(self.cfg.get("current", "0"))
        if self.notice:
            self.message = self.notice
            self.notice = ""

    @property
    def enabled(self) -> bool:
        return self.cfg.get("enabled", "1") == "1"

    @property
    def rotate_sec(self) -> str:
        return self.cfg.get("rotate_sec", "0")

    def current_display(self) -> str:
        for index, _text, proxy in self.rows:
            if index == self.current:
                return proxy.display
        return self.rows[self.current][2].display if self.rows else ""

    # -- filtering --------------------------------------------------------

    def _matches(self, health: dict | None) -> bool:
        if self.filter == "working":
            return bool(health and health.get("ok") and not (self.direct and health.get("ip") == self.direct))
        if self.filter == "untested":
            return health is None
        if self.filter == "failed":
            return bool(health and not health.get("ok"))
        if self.filter == "same":
            return bool(health and health.get("ok") and self.direct and health.get("ip") == self.direct)
        return True

    def visible(self) -> list[tuple[int, str, Proxy]]:
        needle = self.query.lower()
        out = []
        for index, text, proxy in self.rows:
            if needle and needle not in (proxy.display + " " + text).lower():
                continue
            if not self._matches(self.health.get(proxy.label)):
                continue
            out.append((index, text, proxy))
        return out

    def cycle_filter(self) -> None:
        nxt = (FILTERS.index(self.filter) + 1) % len(FILTERS) if self.filter in FILTERS else 0
        self.filter = FILTERS[nxt]
        self.selected = 0
        self.scroll = 0

    def set_query(self, query: str) -> None:
        self.query = query
        self.selected = 0
        self.scroll = 0

    # -- selection --------------------------------------------------------

    def move(self, delta: int) -> None:
        rows = self.visible()
        if not rows:
            self.selected = 0
            return
        self.selected = max(0, min(len(rows) - 1, self.selected + delta))

    def jump(self, where: str) -> None:
        rows = self.visible()
        if not rows:
            return
        self.selected = 0 if where == "home" else len(rows) - 1

    def clamp(self, page: int) -> None:
        page = max(1, page)
        count = len(self.visible())
        self.selected = max(0, min(count - 1, self.selected)) if count else 0
        if self.selected < self.scroll:
            self.scroll = self.selected
        elif self.selected >= self.scroll + page:
            self.scroll = self.selected - page + 1
        self.scroll = max(0, min(max(0, count - page), self.scroll))

    def selected_row(self) -> tuple[int, str, Proxy] | None:
        rows = self.visible()
        if not rows:
            return None
        return rows[max(0, min(len(rows) - 1, self.selected))]

    def use_selected(self) -> str:
        row = self.selected_row()
        if row is None:
            return "no proxy selected"
        store.set_config(current=row[0])
        self.current = row[0]
        return f"current #{row[0]}  {row[2].display}"

    def step(self, delta: int) -> str:
        rows = self.visible()
        if not rows:
            return "no proxy selected"
        pos = (self.selected + delta) % len(rows)
        self.selected = pos
        return self.use_selected()

    # -- actions ----------------------------------------------------------

    def toggle_power(self) -> str:
        enabled = not self.enabled
        store.set_config(enabled=1 if enabled else 0)
        self.cfg["enabled"] = "1" if enabled else "0"
        return "proxying ON" if enabled else "proxying OFF - traffic passes straight through"

    def set_rotate(self, seconds: int) -> str:
        store.set_config(rotate_sec=max(0, seconds))
        self.cfg["rotate_sec"] = str(max(0, seconds))
        if seconds:
            return f"rotating every {seconds}s"
        return "automatic rotation off"

    def start_test(self) -> str:
        if self.test is not None:
            return "a test is already running"
        if not self.rows:
            return "no proxies to test"
        self.test = TestRun(list(self.rows))
        return f"testing {len(self.rows)} proxies in the background"

    def poll_test(self) -> None:
        run = self.test
        if run is None:
            return
        for result in run.drain():
            self.health[result.proxy.label] = {
                "ok": result.ok,
                "ms": result.ms,
                "ip": result.exit_ip,
                "note": result.note,
            }
        if run.direct:
            self.direct = run.direct
        if run.finished:
            self.test = None
            self.message = f"test finished: {run.good} usable / {run.total} checked"

    def clean(self) -> str:
        removed = processes.clean_dead()
        return f"removed {removed} stale state file(s)"


def _int(value: str) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0
