#!/usr/bin/env python3
"""OpenCode Telegram bridge launcher.

This wrapper keeps runner assets separated under `bridge/opencode/...` while
reusing the shared Telegram channel implementation.
"""

from __future__ import annotations

from pathlib import Path
import runpy


def main() -> int:
    bridge_root = Path(__file__).resolve().parents[3]
    shared_impl = bridge_root / "channels" / "telegram" / "channel_bridge.py"
    if not shared_impl.is_file():
        raise SystemExit(
            f"shared telegram bridge implementation missing: {shared_impl}"
        )

    runpy.run_path(str(shared_impl), run_name="__main__")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
