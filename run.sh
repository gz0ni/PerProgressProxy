#!/usr/bin/env sh
set -u

cd "$(dirname "$0")" || exit 1

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) PREBUILT=bin/rot ;;
  Linux-aarch64|Linux-arm64) PREBUILT=bin/rot-aarch64 ;;
  *) PREBUILT= ;;
esac

for bin in $PREBUILT rust/target/release/rot; do
  if [ -n "$bin" ] && [ -f "$bin" ]; then
    [ -x "$bin" ] || chmod +x "$bin" 2>/dev/null
    exec "$bin" "$@"
  fi
done

echo "[!] no rot binary for $(uname -s)/$(uname -m). Build it once: cd rust && cargo build --release" >&2
exit 127
