#!/usr/bin/env sh
# POSIX shim self-test against the first credentialed proxy in proxies.db.
# Prints only the exit IP; the credential itself is never echoed.
set -eu

cd "$(dirname "$0")/.."
ROOT=$(pwd)
SHIM=${1:-}
if [ -z "$SHIM" ]; then
  for candidate in "$ROOT/bin/proxyinject.so" "$ROOT/proxyinject.so"; do
    if [ -f "$candidate" ]; then SHIM=$candidate; break; fi
  done
fi
if [ -z "$SHIM" ]; then
  echo "missing shim: bin/proxyinject.so (build it first, see README)" >&2
  exit 1
fi

ROT=$ROOT/bin/rot
[ "$(uname -m)" = "x86_64" ] || ROT=$ROOT/bin/rot-aarch64
LINE=$("$ROT" raw --credentialed || true)
if [ -z "$LINE" ]; then
  echo "no credentialed proxy in the database" >&2
  exit 1
fi

OUT=$(LD_PRELOAD="$SHIM" PROXYINJECT_DIR="$ROOT" PROXYINJECT_FORCE="$LINE" \
      PROXYINJECT_LOG=0 curl -sS --max-time 20 http://checkip.amazonaws.com)
echo "exit_ip=$OUT"
