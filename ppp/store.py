"""Owns config.ini and proxies.txt, plus the proxy/ intake folder."""

from __future__ import annotations

import re
from pathlib import Path

from .paths import CONFIG, PROXIES, PROXY_DIR
from .specs import Proxy, SpecError, parse_spec, split_tokens

HEADER = (
    "# One proxy per line. Formats accepted:\n"
    "#   socks5://user:pass@1.2.3.4:1080\n"
    "#   socks4://1.2.3.4:1080\n"
    "#   http://1.2.3.4:8080\n"
    "#   1.2.3.4:8080        1.2.3.4:8080:user:pass\n"
    "# Edits are picked up by running processes within ~1 second.\n"
    "# Files dropped into proxy/ are merged here automatically.\n\n"
)


def read_config(path: Path | None = None) -> dict[str, str]:
    path = path or CONFIG
    out: dict[str, str] = {}
    if not path.exists():
        return out
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if not line or line[0] in "#;=" or "=" not in line:
            continue
        key, _, val = line.partition("=")
        out[key.strip().lower()] = val.strip()
    return out


def set_config(**pairs: object) -> None:
    text = CONFIG.read_text(encoding="utf-8") if CONFIG.exists() else ""
    lines = text.splitlines()
    for key, val in pairs.items():
        pattern = re.compile(rf"^\s*{re.escape(key)}\s*=", re.IGNORECASE)
        for i, line in enumerate(lines):
            if pattern.match(line):
                lines[i] = f"{key} = {val}"
                break
        else:
            lines.append(f"{key} = {val}")
    CONFIG.write_text("\n".join(lines) + "\n", encoding="utf-8")


def read_proxy_lines(path: Path | None = None) -> list[tuple[int, str, Proxy | None, str]]:
    """(index, raw text, parsed or None, error) for every non-comment line."""
    path = path or PROXIES
    rows: list[tuple[int, str, Proxy | None, str]] = []
    if not path.exists():
        return rows
    default = read_config().get("default_scheme", "auto")
    for n, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines()):
        s = line.strip()
        if not s or s[0] in "#;":
            continue
        try:
            rows.append((n, s, parse_spec(s, default), ""))
        except SpecError as exc:
            rows.append((n, s, None, str(exc)))
    return rows


def write_proxy_lines(texts: list[str], path: Path | None = None) -> None:
    path = path or PROXIES
    body = HEADER + "\n".join(texts) + "\n" if texts else HEADER
    path.write_text(body, encoding="utf-8")


def proxies() -> list[tuple[int, str, Proxy]]:
    """Valid entries only, with their index in proxies.txt order."""
    out = []
    for idx, (_n, text, p, _err) in enumerate(read_proxy_lines()):
        if p is not None:
            out.append((idx, text, p))
    return out


def current_index() -> int:
    try:
        return int(read_config().get("current", "0"))
    except ValueError:
        return 0


def _existing_keys() -> tuple[list[str], set[str]]:
    texts = [t.strip() for _n, t, _p, _e in read_proxy_lines()]
    keys = set()
    default = read_config().get("default_scheme", "auto")
    for text in texts:
        try:
            keys.add(parse_spec(text, default).label)
        except SpecError:
            keys.add(text)
    return texts, keys


def add_specs(specs: list[str]) -> tuple[int, list[str]]:
    """Append specs, returns (added count, messages)."""
    default = read_config().get("default_scheme", "auto")
    texts, keys = _existing_keys()
    added = 0
    notes: list[str] = []
    for spec in specs:
        try:
            parsed = parse_spec(spec, default)
        except SpecError as exc:
            notes.append(f"skip {spec!r}: {exc}")
            continue
        if parsed.label in keys:
            notes.append(f"dup  {parsed.display}")
            continue
        texts.append(spec.strip())
        keys.add(parsed.label)
        notes.append(f"add  {parsed.display}")
        added += 1
    if added:
        write_proxy_lines(texts)
    return added, notes


def import_file(src: Path) -> tuple[int, int, int]:
    """Import a single file, returns (added, skipped, total)."""
    if not src.exists():
        raise FileNotFoundError(src)
    return _import_text(src.read_text(encoding="utf-8", errors="replace"))


def import_text(raw: str) -> tuple[int, int, int]:
    return _import_text(raw)


def _import_text(raw: str) -> tuple[int, int, int]:
    default = read_config().get("default_scheme", "auto")
    texts, keys = _existing_keys()
    added = skipped = 0
    for token in split_tokens(raw):
        try:
            parsed = parse_spec(token, default)
        except SpecError:
            skipped += 1
            continue
        if parsed.label in keys:
            skipped += 1
            continue
        texts.append(token)
        keys.add(parsed.label)
        added += 1
    if added:
        write_proxy_lines(texts)
    return added, skipped, len(texts)


def proxy_sources() -> list[Path]:
    if not PROXY_DIR.exists():
        return []
    return sorted(p for p in PROXY_DIR.iterdir() if p.is_file() and p.suffix.lower() in (".txt", ".csv", ".list"))


def sync_proxy_dir() -> dict[str, object]:
    """Merge every file in proxy/ into proxies.txt. Idempotent."""
    files = proxy_sources()
    default = read_config().get("default_scheme", "auto")
    texts, keys = _existing_keys()
    added = skipped = 0
    for src in files:
        for token in split_tokens(src.read_text(encoding="utf-8", errors="replace")):
            try:
                parsed = parse_spec(token, default)
            except SpecError:
                skipped += 1
                continue
            if parsed.label in keys:
                continue
            texts.append(token)
            keys.add(parsed.label)
            added += 1
    if added:
        write_proxy_lines(texts)
    return {"files": len(files), "added": added, "skipped": skipped, "total": len(texts)}


def remove_indices(indices: list[int]) -> int:
    rows = read_proxy_lines()
    drop = {i for i in indices}
    keep = [text for i, (_n, text, _p, _e) in enumerate(rows) if i not in drop]
    write_proxy_lines(keep)
    return len(rows) - len(keep)
