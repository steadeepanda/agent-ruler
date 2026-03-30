---
name: agent-ruler-approvals
version: 0.1.9-1
description: "Forward inbound channel interactions to the local Agent Ruler approvals bridge"
homepage: "https://github.com/steadeepanda/agent-ruler"
metadata:
  openclaw:
    emoji: "✅"
    events:
      - message
    install:
      - id: path
        kind: path
        label: Path install
---

# Agent Ruler Approvals Hook

For each inbound message/callback event, this hook forwards a normalized payload (including callback metadata when present) to a local bridge endpoint.

Configure endpoint with environment variable:

- `AR_OPENCLAW_BRIDGE_URL` (default: `http://127.0.0.1:4661/inbound`)
