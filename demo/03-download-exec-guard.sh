#!/usr/bin/env bash
set -euo pipefail

BIN="${1:-./target/release/agent-ruler}"
DROPPER="/tmp/agent-ruler-dropper.sh"

"$BIN" init --force >/dev/null

cat > "$DROPPER" <<'SCRIPT'
#!/usr/bin/env bash
echo "should not run"
SCRIPT
chmod +x "$DROPPER"

set +e
"$BIN" run -- "$DROPPER"
RC=$?
set -e

echo "download->exec style run exit code: $RC (expected non-zero due temp-exec guard)"
"$BIN" tail 20
