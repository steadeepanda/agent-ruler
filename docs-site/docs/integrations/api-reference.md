# Integrations API Reference

This page defines the shared API contract for all runners, then calls out
runner-specific details where behavior differs.

## Shared contract (all runners)

### Deterministic outcomes

Integration handlers must handle these states explicitly:

- success (`completed`, `staged`, `delivered`)
- blocked
- `pending_approval` with `approval_id`

Use wait/resume against the same approval id; avoid blind retries.

### Core endpoints

- `GET /api/status`
- `GET /api/runtime`
- `GET /api/status/feed` (redacted agent-safe feed)
- `GET /api/approvals`
- `GET /api/approvals/:id`
- `GET /api/approvals/:id/wait?timeout=<s>&poll_ms=<ms>`
- `POST /api/approvals/:id/approve`
- `POST /api/approvals/:id/deny`
- `POST /api/approvals/approve-all`
- `POST /api/approvals/deny-all`

### Transfer endpoints

- `POST /api/import/upload`
- `POST /api/import/request`
- `POST /api/export/preview`
- `POST /api/export/request`
- `POST /api/export/deliver/preview`
- `POST /api/export/deliver/request`
- `GET /api/exports/staged`

### Runner introspection + preflight

- `GET /api/runners`
- `GET /api/runners?force=true`
- `GET /api/runners/:id`
- Canonical preflight: `POST /api/runners/:id/tool/preflight`

### Session discovery

These endpoints back the Control Panel session explorer and runner-aware session
discovery for Claude Code and OpenCode:

- `GET /api/sessions?runner=claudecode&channel=telegram&status=active&activity=recent&limit=10&cursor=0`
- `GET /api/sessions/:id`

Response shape highlights:

- `runner_kind` + `runner_label`
- `display_label`
- `created_at` + `last_active_at`
- `channels`
- Telegram bindings when present (`telegram_chat_id`, `telegram_thread_id`, `telegram_message_anchor_id`)

For the Telegram bridge runtime, the shared backend also exposes:

- `POST /api/sessions/telegram/resolve`

That endpoint creates/reuses thread bindings and supports continuation from
computer-started sessions:

- default: create/reuse by `chat_id + thread_id`
- explicit bind: `bind_session_id` or `bind_runner_session_key`
- auto-continue hint: `prefer_existing_runner_session=true`

Runner switches for an already-bound Telegram thread are still rejected.

Compatibility aliases are also available for runner-specific paths where
already wired.

### Approval wait timeout

`/api/approvals/:id/wait` uses the runtime default timeout when not explicitly
provided. Configure this in Control Settings (`Default Approval Wait Timeout`,
range `1..300`, safe default `90`).

### Redaction boundary

`/api/status/feed` is intentionally redacted and excludes policy internals,
receipt internals, runtime secrets, and private operator-only state.

## Runner-specific notes

### OpenClaw

- Compatibility preflight endpoint: `POST /api/openclaw/tool/preflight`
- Tools adapter mapping includes status feed, wait, stage, and deliver helpers.
- OpenClaw bridge runtime + inbound parser contracts support channel-driven
  approval actions (Telegram best validated).

### Claude Code

- Compatibility preflight endpoint: `POST /api/claudecode/tool/preflight`
- Runner mapping must be `claudecode`.
- Managed settings auth/permissions profile is preferred and repaired as needed
  to keep Agent Ruler as governing boundary.
- Telegram threaded sessions map one Telegram thread to one Claude Code session.

### OpenCode

- Compatibility preflight endpoint: `POST /api/opencode/tool/preflight`
- Runner mapping must be `opencode`.
- Managed XDG/HOME overrides are enforced.
- OpenCode uses the same governed runner contract and "rules of living"
  discipline as OpenClaw and Claude Code.
- Telegram threaded sessions map one Telegram thread to one OpenCode session.

## Related pages

- Shared operational model: [Integrations Guide](/integrations/guide)
- Runtime boundary model: [Workspace, Shared Zone, Deliver](/concepts/zones-and-flows)
- UI operator workflows: [Control Panel Guide](/guides/control-panel)
