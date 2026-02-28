#!/usr/bin/env bash
set -euo pipefail

BIN="${1:-./target/release/agent-ruler}"

"$BIN" init --force >/dev/null

STATUS_JSON=$("$BIN" status --json)
WORKSPACE=$(printf '%s' "$STATUS_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["workspace"])')
STATE_DIR=$(printf '%s' "$STATUS_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["state_dir"])')

# Avoid VM-specific net namespace failures during the basic workspace success demo.
sed -i 's/default_deny: true/default_deny: false/' "$STATE_DIR/policy.yaml"

"$BIN" run -- bash -lc 'echo "artifact" > normal-output.txt'

echo "workspace write succeeded: $WORKSPACE/normal-output.txt"
cat "$WORKSPACE/normal-output.txt"
"$BIN" tail 5
