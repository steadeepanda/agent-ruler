#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

BIN="${AR_BIN:-./target/debug/agent-ruler}"
RUNTIME_DIR="${AR_RUNTIME_DIR:-/tmp/agent-ruler-smoke}"

if [[ ! -x "$BIN" ]]; then
  echo "building debug binary..."
  cargo build
fi

echo "running one-shot smoke checks"
"$BIN" --runtime-dir "$RUNTIME_DIR" smoke --non-interactive

echo
echo "to run interactive checks too:"
echo "  $BIN --runtime-dir $RUNTIME_DIR smoke"
