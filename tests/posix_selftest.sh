#!/usr/bin/env sh
# POSIX shim self-test: fetches the exit IP through a fake SOCKS5 exit.
# macOS strips DYLD_* from SIP-protected binaries (/usr/bin/curl, /usr/bin/python3),
# so there the client is a non-system python3 (Homebrew), set PPP_PYTHON to override.
# Usage: sh tests/posix_selftest.sh [path-to-shim]
set -eu

cd "$(dirname "$0")/.."
ROOT=$(pwd)
SHIM=${1:-}
if [ "$(uname -s)" = Darwin ]; then
  PRELOAD=DYLD_INSERT_LIBRARIES
  SHIMS="$ROOT/bin/proxyinject.dylib $ROOT/proxyinject.dylib"
  PY=${PPP_PYTHON:-}
  if [ -z "$PY" ]; then
    for candidate in /opt/homebrew/bin/python3 /usr/local/bin/python3; do
      if [ -x "$candidate" ]; then PY=$candidate; break; fi
    done
  fi
  if [ -z "$PY" ]; then
    echo "need a non-system python3 (brew install python) or PPP_PYTHON=..." >&2
    exit 1
  fi
else
  PRELOAD=LD_PRELOAD
  SHIMS="$ROOT/bin/proxyinject.so $ROOT/proxyinject.so"
  PY=${PPP_PYTHON:-python3}
fi
if [ -z "$SHIM" ]; then
  for candidate in $SHIMS; do
    if [ -f "$candidate" ]; then SHIM=$candidate; break; fi
  done
fi
PORT=${PPP_SELFTEST_PORT:-11080}
EXPECT=${PPP_SELFTEST_IP:-10.9.9.9}

if [ ! -f "$SHIM" ]; then
  echo "missing shim: $SHIM (build it first, see README)" >&2
  exit 1
fi

rm -rf state logs
"$PY" tests/testsocks.py --port "$PORT" --fake-ip "$EXPECT" >/tmp/ppp-testsocks.log 2>&1 &
TS=$!
trap 'kill $TS 2>/dev/null || true' EXIT INT TERM
sleep 1

set +e
if [ "$PRELOAD" = DYLD_INSERT_LIBRARIES ]; then
  OUT=$(env DYLD_INSERT_LIBRARIES="$SHIM" PROXYINJECT_DIR="$ROOT" \
        PROXYINJECT_FORCE="socks5://127.0.0.1:$PORT" PROXYINJECT_LOG=1 "$PY" -c \
        "import urllib.request as u; print(u.build_opener(u.ProxyHandler({})).open('http://checkip.amazonaws.com', timeout=15).read().decode().strip())")
else
  OUT=$(LD_PRELOAD="$SHIM" PROXYINJECT_DIR="$ROOT" PROXYINJECT_FORCE="socks5://127.0.0.1:$PORT" \
        PROXYINJECT_LOG=1 curl -sS --max-time 15 http://checkip.amazonaws.com)
fi
RC=$?
set -e

echo "exit_ip=$OUT"
echo "curl_rc=$RC"
echo "--- state"
cat state/*.txt 2>/dev/null || echo "(no state files)"
echo "--- log"
cat logs/*.log 2>/dev/null || echo "(no logs)"

[ "$RC" -eq 0 ] && [ "$OUT" = "$EXPECT" ]
