#!/usr/bin/env python3
"""Deprecated compatibility shim for the OpenClaw channel bridge.

Canonical location: `bridge/openclaw/channel_bridge.py`.
"""

from bridge.openclaw.channel_bridge import *  # noqa: F401,F403
from bridge.openclaw.channel_bridge import main


if __name__ == "__main__":
    raise SystemExit(main())

