#!/usr/bin/env bash
# Build the core + CLI for the host and copy the shared library next to the
# Flutter desktop runner (Linux/Windows). On Windows run from Git Bash.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release
case "$(uname -s)" in
  Linux*)  LIB=target/release/libreis_core.so;  DEST=app/flutter/build/linux/x64/release/bundle/lib ;;
  MINGW*|MSYS*|CYGWIN*) LIB=target/release/reis_core.dll; DEST=app/flutter/build/windows/x64/runner/Release ;;
  Darwin*) LIB=target/release/libreis_core.dylib; DEST=app/flutter/build/macos ;;
esac
mkdir -p "$DEST" && cp "$LIB" "$DEST/"
echo "core: $LIB -> $DEST"
