#!/usr/bin/env sh
# POSIX shim self-test against the first credentialed proxy in proxies.txt.
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

LINE=$(grep -v -E '^[[:space:]]*[#;]' proxies.txt | awk -F: 'NF>=4' | head -n 1 || true)
if [ -z "$LINE" ]; then
  echo "no credentialed proxy in proxies.txt" >&2
  exit 1
fi

OUT=$(LD_PRELOAD="$SHIM" PROXYINJECT_DIR="$ROOT" PROXYINJECT_FORCE="$LINE" \
      PROXYINJECT_LOG=0 curl -sS --max-time 20 http://checkip.amazonaws.com)
echo "exit_ip=$OUT"
