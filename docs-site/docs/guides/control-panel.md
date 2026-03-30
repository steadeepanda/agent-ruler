# Control Panel Guide

This page is your practical tour of the WebUI.
If you are still on setup, start with [Getting Started](/guides/getting-started).

## Control Panel showcase

<details>
<summary><strong>Open WebUI showcase video</strong></summary>

This recording shows the redesigned Control Panel flow for `v0.1.9-1`.
The clip itself still shows `v0.1.8` in the UI because the version label was not updated before export.

<p align="center">
  <a href="/videos/agent-ruler_showcase_v0.1.9_preview_20260325_224404.mp4">
    <img src="/videos/agent-ruler_showcase_v0.1.9_preview_20260325_224404-poster.png" alt="Agent Ruler Control Panel showcase preview" width="900" />
  </a>
</p>

</details>

## Navigation map

- `Overview`: runtime snapshot and quick actions
- `Approvals`: pending queue, single/bulk resolve, deep-link support
- `Import / Export`: import, stage, and deliver operations
- `Policy`: profile selection and configurable controls
- `Runtime Paths`: shared-zone and default delivery locations
- `Control Settings`: UI/debug/runtime Control Panel settings
- `Execution Layer`: one-shot troubleshooting runner, exec-layer reset, and full runtime reset
- `Timeline`: receipts and filtering
- `Runners`: per-runner install/health/mode/capability/config visibility

You can collapse the left navigation to icon-only mode (or expand it back) from the sidebar/header toggle on any screen size.

`Control Settings` includes:
- `UI Bind Address`
- `Allow degraded confinement fallback`
- `Default Approval Wait Timeout (seconds)` (safe default `90`, range `1..300`)
- Per-runner Telegram bridge controls for `Claude Code` and `OpenCode`, including:
  - bridge enable/disable
  - bot token update field
  - allowed Telegram sender IDs
  - `Stream answers in Telegram` checkbox
  - poll interval / decision TTL / short-id length / state file

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

- Filters: date, verdict, text search, runner (`All | OpenClaw | Claude Code | OpenCode`)
- `Clear`: resets filters and shows full timeline
- Default view: shows the details the agent can see (`decision.detail`)
- `Show operator-only debug details`: reveals hidden fields (full command + diff summary) for debugging
- `Use runtime path labels (recommended)`: shortens long paths in timeline entries (for example `WORKSPACE_PATH/...`)
- Each row now includes `runner_id` in chips/details for cross-runner troubleshooting.

On the `Runtime Paths` page, path-label aliases are intentionally not applied by default so you can inspect exact absolute paths. A local `Hide paths display` toggle is available on that page if you need to mask paths while sharing your screen.

Operator-only fields stay hidden by default so shared timeline views remain safer.

## Runner Fleet View

Use `Monitoring -> Runners` for runner-specific facts without changing enforcement semantics:

- Installed status, binary path, and detected version
- Health and handshake status
- Mode (`one_shot` / `service`) and supported modes
- Managed capability profile per runner kind plus version probe
- Managed config paths and masked env-related keys
- Zone visibility snapshot (`Zone 0 workspace`, `Zone 2 shared`, `Zone 1 delivery`)
- Factual warnings (for example missing executable, service persistence cautions)

The same page now includes a compact `Recent Sessions` explorer for runner-bound
sessions:

- default view shows only a recent slice instead of the full history
- search box matches session id, label/title, runner session key, and Telegram thread id
- filters keep the list breathable: runner tab, `Status`, `Activity`, and `Channel`
- each row shows runner label, channel badges, last activity, and Telegram thread id when present
- `Details` opens the minimal session metadata view without leaving the page

For Telegram 1:1 threaded mode, this is the fastest place to confirm that:

- the thread was bound to the expected runner
- the session stayed isolated to that runner
- the stored Telegram thread id matches what the bot is replying from

For Claude Code/OpenCode Telegram threads, Control Settings also lets you decide
whether replies stream progressively in Telegram or only appear once the runner
finishes. Approval requests still use the same Telegram thread binding while the
runner is waiting on governance.

Performance notes:
- Runner metadata is preloaded on UI bootstrap and cached server-side with short TTL.
- The UI keeps a client-side warm cache and falls back to cached metadata on transient fetch failures.
- `Refresh` on the Runners page forces a live refresh when needed.

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

- Shared + runner-specific integration workflow:
  [Integrations Guide](/integrations/guide)
- API-level integration and wait/resume flow:
  [Integrations API Reference](/integrations/api-reference)
