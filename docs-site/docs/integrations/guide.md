# Integrations Guide

This guide is the shared operator playbook for running runners under Agent Ruler
without bypassing policy, approvals, staging/import/export mediation, or
append-only receipts.

## Runner status (practical recommendation)

- **OpenClaw**: stable and production-oriented.
- **Claude Code**: stable and production-oriented.
- **OpenCode**: stable and follows the same Agent Ruler governance contract and "rules of living" workflow.

## Shared integration contract (all runners)

### 1) Setup and launch pattern

```bash
agent-ruler init
agent-ruler setup
agent-ruler run -- <runner-command>
```

Use `agent-ruler run -- ...` for runner execution so confinement/policy and
approval semantics are enforced.
Control Panel is auto-started/maintained by runner launch; use the URL shown
in terminal output.
Do not invoke bare runner CLIs (`openclaw`, `claude`, `opencode`) from agent
shell/exec tools; Agent Ruler now blocks that path and points operators back to
`agent-ruler run -- ...` so managed restarts and governance cannot escape.

### 2) Safe runtime + zones

All runners use the same runtime zones and transfer discipline:

- **Zone 0 (`workspace`)**: agent working files
- **Zone 2 (`shared-zone`)**: staged boundary area
- **Zone 1 (`delivery`)**: user destination after checks

Transfers stay explicit:

- import: external -> workspace
- stage: workspace -> shared-zone
- deliver: shared-zone -> delivery destination

### 3) Approval discipline

Handle these outcomes explicitly:

- success states (`completed`, `staged`, `delivered`)
- blocked states
- `pending_approval` with `approval_id`

On `pending_approval`, keep the same `approval_id` and wait/resume instead of
blind retry loops.

### 4) Tooling expectations

Runner governance uses preflight checks + Agent Ruler tools/endpoints.
When a guarded action is blocked, stay in runner-safe flow:

- wait on the same approval
- use import/stage/deliver tools for boundary crossings
- avoid direct writes/copies to delivery paths

### 5) Operator surfaces

- Local Control Panel (`/approvals`, `/receipts`, `/files`, `/runners`)
- Optional remote operator access via SSH tunnel or Tailscale
- Optional Telegram bridge per runner (separate chats/topics recommended)

## Runner-specific notes

### OpenClaw

- Supports baseline mode (Control Panel approvals) and seamless mode
  (OpenClaw tools adapter with wait/resume).
- Managed OpenClaw bridge runtime is generated under runtime data and editable
  from Control Settings.
- Chat approval bridge supports Telegram today (best validated path), with
  WhatsApp/Discord support available but less broadly validated.

### Claude Code

- Runner id: `claudecode`
- Managed settings auth is preferred when available; OAuth login is also
  supported when needed.
- Agent Ruler repairs managed Claude permissions profile so Agent Ruler remains
  the governing approval/policy boundary.
- Web mode uses native `claude remote-control` command shape under managed
  process lifecycle.
- Telegram bridge quick use:
  - Enable `Claude Code -> Telegram Bridge` in `Control Settings`
  - Add bot token + your sender ID (`/whoami`) in `allow_from`
  - In Telegram thread: `/status` -> optional `/continue` -> send plain text
  - Keep one thread per task scope; use `/new [topic]` for a new scope

### OpenCode

- Runner id: `opencode`
- Managed XDG/HOME overrides are enforced to keep runtime DB/state writable.
- Managed auth can be host-seeded or supplied through provider env config.
- Governance parity matches OpenClaw and Claude Code for approvals, preflight, and boundary workflow discipline.
- Telegram bridge quick use:
  - Enable `OpenCode -> Telegram Bridge` in `Control Settings`
  - Add bot token + your sender ID (`/whoami`) in `allow_from`
  - In Telegram thread: `/status` -> optional `/continue` -> send plain text
  - Keep one thread per task scope; use `/new [topic]` for a new scope

## Quick validation checklist

1. Confirm runner mapping:

```bash
agent-ruler status --json | jq '.runner'
```

2. Trigger a governed action and verify `pending_approval` appears.
3. Resolve approval in Control Panel and verify exact-step resume.
4. Verify timeline receipts and runner tags in `/receipts`.

## Next

- Endpoint and contract details: [Integrations API Reference](/integrations/api-reference)
- Runtime boundaries: [Workspace, Shared Zone, Deliver](/concepts/zones-and-flows)
- Operator workflows: [Control Panel Guide](/guides/control-panel)
