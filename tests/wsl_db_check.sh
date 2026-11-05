#!/usr/bin/env sh
# Shim end-to-end with the proxy list only in proxies.db: reads it and follows a
# live rotation. Needs python3, curl and bin/rot + bin/proxyinject.so.
set -u

cd "$(dirname "$0")/.." || exit 1
ROOT=$(pwd)
T=$(mktemp -d)
export PPP_ROOT=$T
ROT=$ROOT/bin/rot
printf 'enabled = 1\ncurrent = 0\nfail_closed = 1\nlog = 1\n' > "$T/config.ini"

python3 tests/testsocks.py --port 11080 --fake-ip 10.9.9.9 >/dev/null 2>&1 &
A=$!
python3 tests/testsocks.py --port 11081 --fake-ip 10.9.9.10 >/dev/null 2>&1 &
B=$!
trap 'kill $A $B 2>/dev/null; rm -rf "$T"' EXIT
sleep 1

"$ROT" add socks5://127.0.0.1:11080 socks5://127.0.0.1:11081 >/dev/null
ls "$T" | tr '\n' ' '; echo

ask() {
  LD_PRELOAD="$ROOT/bin/proxyinject.so" PROXYINJECT_DIR="$T" curl -sS --max-time 10 http://checkip.amazonaws.com
}

echo "before: $(ask)"
"$ROT" use 1 >/dev/null
sleep 1
echo "after use 1: $(ask)"
cat "$T"/state/*.txt 2>/dev/null | grep -E "proxies|current" | tr '\n' ' '; echo
