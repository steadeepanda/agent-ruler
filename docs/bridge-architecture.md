# Bridge Architecture for Multi-Surface Approvals

Agent Ruler uses a hybrid approval model so operators can approve safely without babysitting a single browser tab.

## Approval Surfaces

1. **WebUI (canonical):** `http://127.0.0.1:<port>` is the source of truth.
2. **Remote operator access (no public exposure):** Use SSH tunnel or Tailscale to reach the same WebUI.
3. **Optional bridge sidecar:** A separate process that reads redacted approval status and posts notifications to external channels.

The bridge is optional and should run outside the agent sandbox.

## Agent + Channel Onboarding

For autonomous agent operation (for example OpenClaw), you must provide an operator approval path reachable from your phone.

Minimum recommended setup:
1. Run `cargo run -- ui` (defaults to `127.0.0.1:4622`; release-binary alternative: `./target/release/agent-ruler ui`).
2. Connect your agent so guarded actions execute through Agent Ruler.
3. Configure at least one bridge adapter/channel and validate notifications.
4. Ensure your phone receives approval notifications and can open deep links.

If channels are not connected:
- Agent Ruler still enforces approvals correctly.
- WebUI approvals still work.
- Autonomous agent flows can block indefinitely waiting for operator action.

## Operator Workflow

1. Agent triggers a guarded action.
2. Agent Ruler creates a pending approval.
3. Operator is notified (WebUI, browser notification, optional bridge channel message).
4. Operator approves/denies using verified operator path (WebUI and chat command path with allowlists + short-id references).
5. Agent/tool waits via `wait` endpoint or CLI command and resumes deterministically.

## Implemented Endpoints

### List Pending Approvals
`GET /api/approvals`

### Get One Approval (Deep Link Support)
`GET /api/approvals/:id`

### Apply Decision
- `POST /api/approvals/:id/approve`
- `POST /api/approvals/:id/deny`
- `POST /api/approvals/approve-all`
- `POST /api/approvals/deny-all`

Approval resolution now writes explicit receipt transitions (`pending -> approved/denied`) for better audit coherence.

### Wait for Decision (Long Poll)
`GET /api/approvals/:id/wait?timeout=<seconds>&poll_ms=500`

Query params:
- `timeout` (seconds, optional; defaults to Control Panel setting `approval_wait_timeout_secs`, initial safe default `90`, max `300`)
- `poll_ms` (milliseconds, default `500`, range `100..2000`)

Response shape:

```json
{
  "approval_id": "...",
  "verdict": "pending|approved|denied|expired",
  "reason_code": "approval_required_export",
  "category": "shared_zone_stage",
  "target_classification": "shared_zone",
  "guidance": "waiting for approval; open /approvals/<id> in WebUI",
  "open_in_webui": "/approvals/<id>",
  "updated_at": "2026-02-20T10:00:00Z",
  "resolved": false,
  "timeout": true
}
```

### Redacted Status Feed (Agent-Safe)
`GET /api/status/feed?include_resolved=true&limit=100`

Query params:
- `include_resolved` (default `true`)
- `limit` (default `100`, max `500`)

Returned fields are intentionally redacted:
- `approval_id`
- `verdict`
- `reason_code`
- `category`
- `target_classification` (zone-style classification, no raw paths)
- `guidance`
- `open_in_webui`
- `updated_at`

No policy files, raw file paths, diffs, receipts internals, or private approval-state internals are exposed by this feed.

## Deep Link Route

`/approvals/:id` opens approval detail directly in WebUI.

This is used by:
- Browser notifications
- Bridge notifications (Open in WebUI)
- Remote tunnel workflows

## OpenClaw Channel Bridge

Current bridge implementation:
- `bridge/openclaw/channel_bridge.py`
- Hook pack for inbound chat events: `bridge/openclaw/approvals-hook`

What it does now:
- Polls redacted feed (`include_resolved=false`) for pending approvals
- Sends pending approval notifications through OpenClaw channels (Telegram, WhatsApp, Discord)
- Emits Telegram typing keepalive during pending-approval windows to avoid stalled UX on long waits
- Includes deep link to WebUI approval detail
- Accepts chat decisions via:
  - Telegram callback payloads (`ar:approve:<short-id>`, `ar:deny:<short-id>`)
  - Text commands (`approve <short-id>`, `deny <short-id>`)
  - WhatsApp poll options (`approve <short-id>`, `deny <short-id>`)
- Enforces channel allowlists and short-id mapping to pending approvals (with TTL expiry)
- Handles inbound hook delivery asynchronously (`202 accepted`) for non-sync events to avoid blocking channel hook pipelines

Current validation status:
- Telegram: tested in this release
- WhatsApp: supported but not yet fully validated end-to-end in this public release
- Discord: supported but not yet fully validated end-to-end in this public release

## Remote Access Guidance (Operator)

### SSH Tunnel

```bash
ssh -L 4622:127.0.0.1:4622 user@host
```

Then open `http://127.0.0.1:4622` locally.

### Tailscale

Bind UI to Tailscale interface or host LAN with firewall restrictions, then access through tailnet ACLs.

## Security Notes

- WebUI remains canonical and local-first.
- Bridge remains optional and isolated from the agent sandbox.
- Redacted status feed is safe for agent/tool polling and does not expose secrets.
- Chat approvals are enforced by channel allowlists + pending short-id mapping at bridge layer.

## Quick Tests

```bash
# list pending
curl http://127.0.0.1:4622/api/approvals

# wait (uses runtime default timeout unless overridden)
curl "http://127.0.0.1:4622/api/approvals/<id>/wait"

# explicit override
curl "http://127.0.0.1:4622/api/approvals/<id>/wait?timeout=45&poll_ms=500"

# redacted feed
curl "http://127.0.0.1:4622/api/status/feed?include_resolved=true&limit=20"
```
