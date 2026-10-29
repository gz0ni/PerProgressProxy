#!/usr/bin/env sh
# POSIX shim self-test: runs curl through a fake SOCKS5 exit.
# Usage: sh tests/posix_selftest.sh [path-to-shim]
set -eu

cd "$(dirname "$0")/.."
ROOT=$(pwd)
SHIM=${1:-}
if [ -z "$SHIM" ]; then
  for candidate in "$ROOT/bin/proxyinject.so" "$ROOT/proxyinject.so"; do
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
python3 tests/testsocks.py --port "$PORT" --fake-ip "$EXPECT" >/tmp/ppp-testsocks.log 2>&1 &
TS=$!
trap 'kill $TS 2>/dev/null || true' EXIT INT TERM
sleep 1

set +e
OUT=$(LD_PRELOAD="$SHIM" PROXYINJECT_DIR="$ROOT" PROXYINJECT_FORCE="socks5://127.0.0.1:$PORT" \
      PROXYINJECT_LOG=1 curl -sS --max-time 15 http://checkip.amazonaws.com)
RC=$?
set -e

echo "exit_ip=$OUT"
echo "curl_rc=$RC"
echo "--- state"
cat state/*.txt 2>/dev/null || echo "(no state files)"
echo "--- log"
cat logs/*.log 2>/dev/null || echo "(no logs)"

[ "$RC" -eq 0 ] && [ "$OUT" = "$EXPECT" ]
