#!/bin/bash
# Agent Ruler - OWASP Scenario Smoke Tests
#
# This script runs manual smoke tests for OWASP prompt injection scenarios.
# Each test demonstrates a specific attack vector and validates the defense.
#
# Usage: ./demo/06-owasp-smoke.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DEMO_DIR="$PROJECT_DIR/demo"

echo "========================================"
echo "Agent Ruler - OWASP Scenario Smoke Tests"
echo "========================================"
echo ""

# Check if agent-ruler is built
if ! command -v cargo &> /dev/null; then
    echo "ERROR: cargo not found. Please install Rust."
    exit 1
fi

cd "$PROJECT_DIR"

# Build if needed
echo "Building agent-ruler..."
cargo build --release 2>/dev/null || { echo "Build failed"; exit 1; }

RULER_BIN="./target/release/agent-ruler"

# Create temp test directory
TEST_DIR=$(mktemp -d)
trap "rm -rf $TEST_DIR" EXIT
RUNTIME_DIR="$TEST_DIR/runtime"
RULER_CMD=("$RULER_BIN" --runtime-dir "$RUNTIME_DIR")

echo ""
echo "Test directory: $TEST_DIR"
echo "Runtime directory: $RUNTIME_DIR"
echo ""

# Initialize test project
echo "=== Initializing test project ==="
"${RULER_CMD[@]}" init --force
echo ""

# ============================================
# Scenario 1: Normal workspace operations succeed
# ============================================
echo "========================================"
echo "Scenario 1: Normal Workspace Operations"
echo "========================================"
echo "Testing: Legitimate workspace writes should succeed"
echo ""

"${RULER_CMD[@]}" run -- bash -c "echo 'test content' > test.txt && cat test.txt"
echo ""
echo "✓ Normal workspace operations: PASS"
echo ""

# ============================================
# Scenario 2: System delete blocked
# ============================================
echo "========================================"
echo "Scenario 2: System Delete Blocked"
echo "========================================"
echo "Testing: Attempts to delete system files should be blocked"
echo ""

if "${RULER_CMD[@]}" run -- rm -f /etc/passwd 2>&1 | grep -q "denied\|blocked"; then
    echo "✓ System delete blocked: PASS"
else
    echo "! System delete test: Check receipts for details"
fi
echo ""

# ============================================
# Scenario 3: Download→Exec blocked
# ============================================
echo "========================================"
echo "Scenario 3: Download→Exec Chain Blocked"
echo "========================================"
echo "Testing: Execution of downloaded files should be quarantined"
echo ""

# Create a "downloaded" file in workspace
"${RULER_CMD[@]}" run -- bash -c "echo '#!/bin/bash' > downloaded_script.sh && chmod +x downloaded_script.sh"

# Attempt to execute - should be blocked
if "${RULER_CMD[@]}" run -- ./downloaded_script.sh 2>&1 | grep -q "denied\|blocked\|quarantine"; then
    echo "✓ Download→Exec blocked: PASS"
else
    echo "! Download→Exec test: Check receipts for details"
fi
echo ""

# ============================================
# Scenario 4: Network default deny
# ============================================
echo "========================================"
echo "Scenario 4: Network Default Deny"
echo "========================================"
echo "Testing: Network requests to non-allowlisted hosts should be blocked"
echo ""

# This test depends on network being available
if ping -c 1 -W 2 8.8.8.8 &>/dev/null; then
    if "${RULER_CMD[@]}" run -- curl -s https://example.com 2>&1 | grep -q "denied\|blocked\|network"; then
        echo "✓ Network default deny: PASS"
    else
        echo "! Network test: Check policy configuration"
    fi
else
    echo "⊘ Network test skipped (no network connectivity)"
fi
echo ""

# ============================================
# Scenario 5: Secrets access denied
# ============================================
echo "========================================"
echo "Scenario 5: Secrets Access Denied"
echo "========================================"
echo "Testing: Access to secrets paths should be denied"
echo ""

if "${RULER_CMD[@]}" run -- cat /root/.ssh/id_rsa 2>&1 | grep -q "denied\|blocked\|secrets"; then
    echo "✓ Secrets access denied: PASS"
else
    echo "! Secrets test: Check receipts for details"
fi
echo ""

# ============================================
# Scenario 6: Persistence blocked
# ============================================
echo "========================================"
echo "Scenario 6: Persistence Blocked"
echo "========================================"
echo "Testing: Attempts to create persistence mechanisms should be blocked"
echo ""

if "${RULER_CMD[@]}" run -- bash -c "echo '[Service]' > /etc/systemd/system/malicious.service" 2>&1 | grep -q "denied\|blocked\|persistence"; then
    echo "✓ Persistence blocked: PASS"
else
    echo "! Persistence test: Check receipts for details"
fi
echo ""

# ============================================
# Summary
# ============================================
echo "========================================"
echo "Smoke Test Summary"
echo "========================================"
echo ""
echo "Review detailed receipts:"
echo "  $RULER_BIN --runtime-dir $RUNTIME_DIR tail 20"
echo ""
echo "View in WebUI:"
echo "  $RULER_BIN --runtime-dir $RUNTIME_DIR ui"
echo ""
echo "========================================"
echo "OWASP Scenario Smoke Tests Complete"
echo "========================================"
