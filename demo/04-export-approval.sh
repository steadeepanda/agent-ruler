#!/usr/bin/env bash
set -euo pipefail

BIN="${1:-./target/release/agent-ruler}"

"$BIN" init --force >/dev/null

STATUS_JSON=$("$BIN" status --json)
WORKSPACE=$(printf '%s' "$STATUS_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["workspace"])')
SHARED_ZONE=$(printf '%s' "$STATUS_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["shared_zone"])')

mkdir -p "$WORKSPACE"
echo "release-notes-v1" > "$WORKSPACE/report.txt"

OUT=$("$BIN" export report.txt report.txt || true)
echo "$OUT"

PENDING_ID=$(echo "$OUT" | awk '/pending id:/ {print $NF}' | tail -n 1)
if [[ -z "$PENDING_ID" ]]; then
  echo "no pending approval id detected"
  "$BIN" approve --decision list
  exit 1
fi

"$BIN" approve --decision approve --id "$PENDING_ID"

if [[ -f "$SHARED_ZONE/report.txt" ]]; then
  echo "export committed after approval: $SHARED_ZONE/report.txt"
else
  echo "export file missing after approval"
  exit 1
fi

"$BIN" tail 30
