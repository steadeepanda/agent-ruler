# Control Panel Guide

This page is your practical tour of the WebUI.
If you are still on setup, start with [Getting Started](/guides/getting-started).

## Navigation map

- `Overview`: runtime snapshot and quick actions
- `Approvals`: pending queue, single/bulk resolve, deep-link support
- `Import / Export`: import, stage, and deliver operations
- `Policy`: profile selection and configurable controls
- `Runtime Paths`: shared-zone and default delivery locations
- `Control Settings`: UI/debug/runtime Control Panel settings
- `Execution Layer`: one-shot troubleshooting runner, exec-layer reset, and full runtime reset
- `Timeline`: receipts and filtering

You can collapse the left navigation to icon-only mode (or expand it back) from the sidebar/header toggle on any screen size.

`Control Settings` includes:
- `UI Bind Address`
- `Show debug tools in UI`
- `Allow degraded confinement fallback`
- `Default Approval Wait Timeout (seconds)` (safe default `120`, range `1..300`)

## Policy profile quick guide

- `Strict`: maximum guardrails, best for untrusted tasks.
- `Simple User`: safest everyday profile with minimal tuning required.
- `Balanced`: recommended quick-start profile for most operators.
- `Coding/Nerd`: more freedom for coding workflows while preserving core boundaries.
- `I DON'T CARE`: emergency low-friction mode ("you don't care, but I do lol").
- `Custom`: advanced personalization.

Baseline protections always stay active, including system-critical deny, secrets deny, and download->exec quarantine.

## Domain rules recommendation

- Keep POST domain entries limited to trusted endpoints you actually use.
- Domain lists start unchecked; invert toggles decide whether checked hosts block or allow traffic, and every request still flows through Agent Ruler's policy/confinement checks.
- GET can remain broad (whole internet) when `default_deny = false` and network posture allows it.
- POST remains more sensitive: allow only trusted domains and prefer manual human POST actions for sensitive write operations.

## Import / stage / deliver flow

The zone layout intentionally shows two cards on the first row (`Workspace` + `Shared Zone`) and `Deliveries` on the next row to reduce visual overload.

1. Import into workspace
   - Choose a local file from the browser picker.
   - Destination override is optional and hidden until enabled.

2. Stage from workspace to shared zone
   - Pick source manually or from the workspace hierarchy browser.
   - Destination override is optional and hidden until enabled.

3. Deliver from shared zone to destination
   - Pick stage reference manually or from the shared-zone hierarchy browser.
   - Destination override is optional and hidden until enabled.

The hierarchy browser is zone-scoped (`workspace` / `shared-zone`) so transfer actions stay aligned with confinement boundaries.
Use the zone browser scroll areas to inspect deeper folders before selecting stage/deliver targets.

## User mode vs agent mode

- User mode (`auto_approve=true`): manual Control Panel actions skip approval queue.
- Agent mode: same boundaries apply, but governed actions can enter approvals.
- Drag/drop transfers are user-only; the `Auto-approve drag/drop transfers` checkbox controls whether those drop actions auto-approve or enter queue.

## Timeline and detailed visibility

Timeline shows receipts across governed actions.

- Filters: date, verdict, text search
- `Clear`: resets filters and shows full timeline
- Default view: shows the details the agent can see (`decision.detail`)
- `Show operator-only debug details`: reveals hidden fields (full command + diff summary) for debugging
- `Use runtime path labels (recommended)`: shortens long paths in timeline entries (for example `WORKSPACE_PATH/...`)

Operator-only fields stay hidden by default so shared timeline views remain safer.

## Reset options

From the `Execution Layer` page:

- `Reset Execution Layer`
  - Clears only `state/exec-layer`.
  - Keeps config, policy, receipts, approvals, and staged exports.
- `Reset Runtime` + `Keep current config and policy` checked
  - Recreates runtime state while preserving your current config/policy wiring.
- `Reset Runtime` + `Keep current config and policy` unchecked
  - Restores default config/policy and recreates runtime state.

This makes recovery fast when something breaks, without forcing reinstall/reconfiguration every time.

## One-shot command runner

The `Execution Layer` page includes a one-shot command runner for deterministic troubleshooting.

- Runs through `/api/run/script`
- Returns exit code, confinement mode, stdout, stderr
- Writes receipts visible in Timeline

Use this for operator diagnostics, not as a replacement for normal agent task pipelines.

## Next pages

- OpenClaw-focused setup and approvals workflow:
  [OpenClaw Guide](/integrations/openclaw-guide)
- API-level integration and wait/resume flow:
  [OpenClaw API Reference](/integrations/openclaw-api-reference)
- Validation checklist before unattended runs:
  [Manual Tests](/guides/manual-tests)
