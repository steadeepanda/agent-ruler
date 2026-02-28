#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AR_DIR="${AR_DIR:-$REPO_ROOT}"

cd "$AR_DIR"
python3 bridge/openclaw/smoke_replay.py
