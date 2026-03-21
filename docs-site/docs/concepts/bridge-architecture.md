---
title: Bridge Architecture
---

# Bridge Architecture for Multi-Surface Approvals

Agent Ruler keeps approval enforcement centralized in the runtime API.
Bridges are optional transport adapters that notify operators and relay
approve/deny intent back to Agent Ruler.

## Approval surfaces

1. **Control Panel (canonical):** source of truth for queue state and decision history.
2. **Remote Control Panel access:** same UI surface over SSH tunnel or Tailscale.
3. **Optional channel bridges:** per-runner channel adapters for operator notifications and quick decisions.

If channel bridges are disabled, approvals still work through the Control Panel.

## Runtime behavior

- Runner launch (`agent-ruler run -- <runner ...>`) starts/maintains the Control Panel runtime.
- `agent-ruler ui` remains available for explicit UI lifecycle control.
- Bridge configs are runtime-managed under `runtime_root/user_data/bridge`.
- Operator-edited bridge settings are handled in **Control Settings**.

## End-to-end decision flow

1. Runner triggers a governed action.
2. Policy/preflight creates an approval (`pending_approval`).
3. Operator is notified (Control Panel and optionally channel bridge).
4. Operator approves/denies in Control Panel or supported channel action.
5. Agent wait/resume flow continues with the same approval id.
6. Receipt trail records the transition and final verdict.

## API contracts used by bridges

### Approval queue + decision endpoints

- `GET /api/approvals`
- `GET /api/approvals/:id`
- `POST /api/approvals/:id/approve`
- `POST /api/approvals/:id/deny`
- `POST /api/approvals/approve-all`
- `POST /api/approvals/deny-all`

### Wait/resume endpoint

`GET /api/approvals/:id/wait?timeout=<seconds>&poll_ms=<ms>`

- `timeout`: defaults to runtime setting (`Default Approval Wait Timeout`, max `300`)
- `poll_ms`: default `500`, range `100..2000`

### Agent-safe status feed

`GET /api/status/feed?include_resolved=true&limit=100`

This feed is intentionally redacted for safe polling:
- includes ids/verdict/reason/guidance/deep-link metadata
- excludes policy internals, raw private state, and sensitive runtime internals

### Runner preflight endpoints

- Canonical: `POST /api/runners/:id/tool/preflight`
- Compatibility aliases:
  - `POST /api/openclaw/tool/preflight`
  - `POST /api/claudecode/tool/preflight`
  - `POST /api/opencode/tool/preflight`

## Runner + channel bridge status

- **OpenClaw + Telegram:** most validated bridge path today.
- **Claude Code + Telegram:** supported runtime path with current release validation.
- **OpenCode + Telegram:** supported runtime path with current release validation and parity governance behavior.

Use separate chats/topics per runner to reduce operator confusion in mixed-runner deployments.

## Security boundaries

- Channel bridges do not bypass policy or approval semantics.
- Final decision state is written by Agent Ruler approval APIs, not by channel transport logic.
- Bridges should run outside confined agent process context.
- Decision mappings use short-id + TTL behavior to limit replay and stale callbacks.

## Remote access guidance

### SSH tunnel

```bash
ssh -L 4622:127.0.0.1:4622 user@host
```

Then open the tunneled local URL.

### Tailscale

Use the Control Panel URL shown by Agent Ruler runtime output (Tailscale IP when available).
Keep ACL/firewall restrictions in place.

## Ops checklist

1. Confirm Control Panel reachability.
2. Trigger a test governed action and confirm `pending_approval` appears.
3. Verify operator notification path (UI and optional channel).
4. Resolve approval and confirm exact-step resume.
5. Verify receipt trail for pending -> approved/denied transition.
