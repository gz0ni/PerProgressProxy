"""Proxy specification parsing.

Mirrors ``parse_proxy_spec()`` in ``src/proxyinject.c``: the shapes accepted here
must be the shapes the DLL accepts, with the same scheme guessing.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

SCHEMES = ("socks5", "socks4", "http")
SOCKS_PORTS = {1080, 1081, 1082, 9050, 9150}

_CRED_RE = re.compile(r"^(?P<host>[^:]+):(?P<port>\d{1,5}):(?P<user>[^:]*):(?P<pw>.+)$")


class SpecError(ValueError):
    pass


@dataclass
class Proxy:
    scheme: str
    host: str
    port: int
    user: str = ""
    pass_: str = ""

    @property
    def label(self) -> str:
        if self.user:
            return f"{self.scheme}://{self.user}:{self.pass_}@{self.host}:{self.port}"
        return f"{self.scheme}://{self.host}:{self.port}"

    @property
    def display(self) -> str:
        if self.user:
            return f"{self.scheme}://{self.user}:***@{self.host}:{self.port}"
        return self.label


def parse_spec(text: str, default_scheme: str = "auto") -> Proxy:
    raw = (text or "").strip()
    if not raw or raw[0] in "#;":
        raise SpecError("empty or comment")

    scheme = ""
    rest = raw
    if "://" in raw:
        scheme, rest = raw.split("://", 1)
        scheme = scheme.lower()
        if scheme not in SCHEMES:
            raise SpecError(f"unknown scheme {scheme!r}")
    elif raw.lower().startswith(("socks5:", "socks4:", "http:")):
        scheme, rest = raw.split(":", 1)
        scheme = scheme.lower()

    user = pw = ""
    if "@" in rest:
        cred, rest = rest.rsplit("@", 1)
        if ":" in cred:
            user, pw = cred.split(":", 1)
        else:
            user = cred

    if rest.startswith("["):
        host, _, tail = rest[1:].partition("]")
        port = tail.lstrip(":")
    else:
        match = _CRED_RE.match(rest)
        if match and rest.count(":") >= 3:
            host = match.group("host")
            port = match.group("port")
            if not user:
                user, pw = match.group("user"), match.group("pw")
        elif rest.count(":") == 1:
            host, port = rest.split(":")
        else:
            raise SpecError("expected host:port or host:port:user:pass")

    host = host.strip()
    port = port.strip()
    if not host or not port.isdigit():
        raise SpecError("bad host/port")
    port_i = int(port)
    if not 0 < port_i < 65536:
        raise SpecError("port out of range")

    if not scheme:
        if default_scheme in SCHEMES:
            scheme = default_scheme
        elif port_i in SOCKS_PORTS:
            scheme = "socks5"
        else:
            scheme = "http"
    if scheme == "socks4" and (user or pw):
        raise SpecError("socks4 cannot carry a username/password")
    return Proxy(scheme, host, port_i, user, pw)


def split_tokens(raw: str) -> list[str]:
    """Split an arbitrary list file into candidate proxy tokens."""
    out: list[str] = []
    for line in re.split(r"[\r\n]+", raw):
        stripped = line.strip()
        if not stripped or stripped[0] in "#;":
            continue
        for token in re.split(r"[\s,;]+", stripped):
            token = token.strip().strip('"\'')
            if not token or token.startswith("#"):
                continue
            out.append(token)
    return out
