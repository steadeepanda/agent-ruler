#!/usr/bin/env bash
# Agent Ruler - Mediated Elevation Smoke Test (Ubuntu/Debian)
#
# Default behavior uses mock auth/helper so it is safe for CI/dev:
#   AR_ELEVATION_SMOKE_MOCK=1 ./demo/07-elevation-smoke.sh
#
# Real host flow (requires operator auth + apt execution):
#   AR_ELEVATION_SMOKE_MOCK=0 ./demo/07-elevation-smoke.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

echo "========================================"
echo "Agent Ruler - Mediated Elevation Smoke"
echo "========================================"

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo not found."
  exit 1
fi

cargo build --release >/dev/null
RULER_BIN="./target/release/agent-ruler"

TEST_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_DIR"' EXIT

echo "[1/6] init runtime"
"$RULER_BIN" --runtime-dir "$TEST_DIR/runtime" init --force

echo "[2/6] trigger mediated elevation request"
set +e
TRIGGER_OUTPUT="$("$RULER_BIN" --runtime-dir "$TEST_DIR/runtime" run -- sudo apt install git 2>&1)"
TRIGGER_EXIT=$?
set -e
echo "$TRIGGER_OUTPUT"
if [[ $TRIGGER_EXIT -eq 0 ]]; then
  echo "ERROR: expected sudo request to be approval-gated"
  exit 1
fi

PENDING_ID="$(echo "$TRIGGER_OUTPUT" | sed -n 's/.*pending id: \([a-f0-9-]\+\).*/\1/p' | tail -n1)"
if [[ -z "$PENDING_ID" ]]; then
  echo "ERROR: could not parse pending id from output"
  exit 1
fi
echo "pending id: $PENDING_ID"

echo "[3/6] verify wait endpoint reports pending/timeout"
"$RULER_BIN" --runtime-dir "$TEST_DIR/runtime" wait --id "$PENDING_ID" --timeout 1 --json

echo "[4/6] approve request"
if [[ "${AR_ELEVATION_SMOKE_MOCK:-1}" == "1" ]]; then
  echo "Using mock auth/helper (no host package install)."
  AR_ELEVATION_AUTH_MODE=mock AR_ELEVATION_HELPER_MODE=mock \
    "$RULER_BIN" --runtime-dir "$TEST_DIR/runtime" approve --decision approve --id "$PENDING_ID"
else
  echo "Using real auth/helper (may prompt for sudo password)."
  "$RULER_BIN" --runtime-dir "$TEST_DIR/runtime" approve --decision approve --id "$PENDING_ID"
fi

echo "[5/6] wait for resolved decision"
"$RULER_BIN" --runtime-dir "$TEST_DIR/runtime" wait --id "$PENDING_ID" --timeout 5 --json

echo "[6/6] inspect receipts for full trace"
"$RULER_BIN" --runtime-dir "$TEST_DIR/runtime" tail 50

echo "========================================"
echo "Elevation smoke completed"
echo "========================================"
