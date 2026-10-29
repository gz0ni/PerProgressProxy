#!/usr/bin/env sh
set -u

cd "$(dirname "$0")" || exit 1

if [ -n "${PYTHON:-}" ]; then
  PY="$PYTHON"
elif command -v python3 >/dev/null 2>&1; then
  PY=python3
elif command -v python >/dev/null 2>&1; then
  PY=python
else
  echo "[!] python3 not found in PATH. Install Python 3.8+ or set PYTHON." >&2
  exit 127
fi

exec "$PY" rot.py "$@"
