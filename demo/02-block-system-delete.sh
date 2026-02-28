#!/usr/bin/env bash
set -euo pipefail

BIN="${1:-./target/release/agent-ruler}"

"$BIN" init --force >/dev/null

set +e
"$BIN" run -- rm /etc/passwd
RC=$?
set -e

echo "run exit code: $RC (expected non-zero)"
"$BIN" tail 20
