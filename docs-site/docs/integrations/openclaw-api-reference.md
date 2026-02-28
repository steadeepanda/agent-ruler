# OpenClaw API Reference

Use this page when wiring OpenClaw tooling directly to Agent Ruler endpoints.
For operator workflow and channel setup, start with [OpenClaw Guide](/integrations/openclaw-guide).

## Runner setup prerequisite

Before API wiring, run:

```bash
agent-ruler init
agent-ruler setup
```

Developer fallback from source checkout:

```bash
cargo run -- init
cargo run -- setup
```

Then run OpenClaw under confinement with the setup-provided managed home path:

```bash
agent-ruler run -- openclaw gateway
```

Agent Ruler injects `OPENCLAW_HOME` automatically for `run -- openclaw ...` when the runner is configured.

## Deterministic response contract

Handle these outcomes explicitly:

- `completed` / `staged` / `delivered`
- `blocked`
- `pending_approval` with `approval_id`

On `pending_approval`, switch to wait mode instead of blind retries.
The bundled OpenClaw adapter defaults to automatic wait/resume (`autoWaitForApprovals=true`).

## Runtime and status endpoints

- `GET /api/status`
  - profile, counts, runtime summary
- `GET /api/runtime`
  - runtime paths/files
- `GET /api/status/feed`
  - redacted approval events for safe polling
  - query: `include_resolved`, `limit`
- `POST /api/openclaw/tool/preflight`
  - plugin hook endpoint used by Agent Ruler OpenClaw adapter to evaluate native tool calls before execution
  - appends timeline receipts for tool preflight decisions (`allow`, `denied`, `pending_approval`, `quarantined`)

## Approval endpoints

- `GET /api/approvals`
- `GET /api/approvals/<id>`
- `GET /api/approvals/<id>/wait?timeout=<seconds>&poll_ms=<ms>`
- `POST /api/approvals/<id>/approve`
- `POST /api/approvals/<id>/deny`
- `POST /api/approvals/approve-all`
- `POST /api/approvals/deny-all`

`timeout` is optional. If omitted, Agent Ruler uses the runtime default from Control Panel settings (`approval_wait_timeout_secs`, initial safe default `90`, max `300`).

## Transfer endpoints

- `POST /api/import/upload`
- `POST /api/import/request`
- `POST /api/export/preview`
- `POST /api/export/request`
- `POST /api/export/deliver/preview`
- `POST /api/export/deliver/request`
- `GET /api/exports/staged`

## Wait/resume pattern

1. Submit action request.
2. If `pending_approval`, persist `approval_id`.
3. Wait with bounded timeout.
4. On timeout, keep `waiting_for_operator` state.
5. On approved, resume exactly from blocked step.
6. On denied/expired, stop and report reason.

Adapter behavior (default):
- waits on `/api/approvals/<id>/wait` with bounded timeout
- resumes exactly one blocked step after approval
- avoids blind retry loops that can drift to a new approval id

### Wait timeout tuning (Control Panel)

Use **Control Settings** to set `Default Approval Wait Timeout (seconds)`.
- Safe initial default: `90`
- Allowed range: `1..300`
- Applies to adapter wait calls when no explicit timeout override is passed

## Redacted status boundary

`/api/status/feed` is intentionally redacted.

Included:

- `approval_id`
- verdict/reason/category labels
- guidance text
- `open_in_webui` deep link path
- update timestamp

Excluded:

- policy files
- approval queue internals
- receipt internals
- runtime secrets or private operator-only state

## OpenClaw plugin tool mapping

Plugin path:

```text
bridge/openclaw/openclaw-agent-ruler-tools
```

Current mapping:

- `agent_ruler_status_feed` -> `GET /api/status/feed`
- `agent_ruler_wait_for_approval` -> `GET /api/approvals/<id>/wait`
- `agent_ruler_request_export_stage` -> `POST /api/export/request`
- `agent_ruler_request_delivery` -> `POST /api/export/deliver/request`
- `before_tool_call` hook for core tools (`write/edit/delete/move/read/exec`) -> `POST /api/openclaw/tool/preflight`

## OpenClaw channel bridge components

For chat approvals in Telegram/WhatsApp/Discord:

- bridge runtime: `bridge/openclaw/channel_bridge.py`
- inbound hook pack: `bridge/openclaw/approvals-hook`

Validation status in this release:
- Telegram: tested
- WhatsApp: supported, pending broader end-to-end validation
- Discord: supported, pending broader end-to-end validation

Preferred route source:

- `plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes` in managed OpenClaw config

Auto-seed behavior:

- On `agent-ruler run -- openclaw gateway`, Agent Ruler checks existing bridge config routes and auto-seeds `approvalBridgeRoutes` when missing.
- This keeps existing user route setups working without extra migration steps.
- Bridge runtime config is generated automatically under `<runtime_root>/user_data/bridge/`; manual bridge JSON editing is not required for normal setup.
- Control Panel exposes these generated bridge runtime fields under **Settings → Control Settings** for in-place edits.
- If `approvalBridgeRoutes` is not set, the bridge falls back to channel-default autodiscovery from OpenClaw `channels.*` and `*-allowFrom.json` credentials.

### Outbound UX contracts

- Telegram:
  - deep link + summary text
  - callback buttons when `telegram_inline_buttons=true` and channel capability allows
  - button decisions resolve through Agent Ruler operator CLI path (approve/deny)
  - redacted bridge log milestones: `approval detected` -> `message queued` -> `message sent`
- WhatsApp:
  - poll first (`openclaw message poll`) when `whatsapp_use_poll=true`
  - alert includes deep link and poll guidance (no free-text command reliance)
- Discord:
  - formatted alert + deep link (approve/deny in WebUI)

### Wait/Resume and Safety

- `pending_approval` responses trigger bounded wait in the tools adapter and automatic resume on approve.
- Wait requests use timeout-aware HTTP deadlines (no premature 10s abort for long waits).
- Preflight hook failures fail closed (tool call blocked) to preserve policy semantics.

### Inbound parser contracts

Accepted command forms:

- Telegram callback payload: `ar:approve:<short-id>` / `ar:deny:<short-id>`
- (Compatibility only) text commands: `approve <short-id>` / `deny <short-id>`
- (Compatibility only) full id commands: `approve <approval-id>` / `deny <approval-id>`

Short IDs are generated by the bridge and mapped to pending approval IDs. Cards now emphasize buttons/polls/WebUI link instead of free-text commands.

## Related pages

- [OpenClaw Guide](/integrations/openclaw-guide)
- [Control Panel Guide](/guides/control-panel)
- [Workspace, Shared Zone, Deliver](/concepts/zones-and-flows)
