# OpenClaw Guide

This guide is for operators who want OpenClaw autonomy with Agent Ruler safety gates, without babysitting.

## Two integration modes (both still run inside Agent Ruler)

Both modes require confinement. In baseline and seamless mode, you still run OpenClaw through Agent Ruler with `agent-ruler run -- ...`.

- `Baseline`: wrapper run only. Approvals are handled in the Control Panel.
- `Seamless (recommended)`: same wrapper run, plus the OpenClaw tools adapter so the agent can wait/resume deterministically and report approval state clearly.

Seamless mode does **not** replace confinement. It adds better runtime behavior on top of confinement.

For endpoint contracts and response behavior, use [OpenClaw API Reference](/integrations/openclaw-api-reference).

## Command style used here

Default:

```bash
agent-ruler <subcommand> [args...]
```

Developer fallback from source checkout:

```bash
cargo run -- <subcommand> [args...]
```

## Path terms used here

- **Host OpenClaw home/workspace**
  - Your existing host OpenClaw paths (for example `~/.openclaw`)
  - Agent Ruler does not move or restore these paths
  - Not mounted by default for confined OpenClaw runs
- **Ruler-managed OpenClaw home/workspace**
  - Project-local runtime paths used by OpenClaw under Agent Ruler
  - Home: `<runtime>/user_data/openclaw_home/`
  - Workspace: `<runtime>/workspace/` (or `<runtime>/user_data/openclaw_workspace/` if needed)

## Fast path: you already have OpenClaw installed

If OpenClaw is already installed on your host, this is the shortest path:

```bash
# optional local binary install for Agent Ruler
bash install/install.sh --local

# initialize runtime for this project
agent-ruler init

# wire OpenClaw runner for this project
agent-ruler setup

# run the command printed by setup
agent-ruler run -- openclaw gateway

# optional checks
agent-ruler status --json
agent-ruler ui
```

`setup` keeps Host OpenClaw home/workspace untouched unless you explicitly choose import.
When you choose import, Agent Ruler copies host OpenClaw auth/config material into the managed home (read/copy only, host files unchanged).
Installer path wiring is automatic; if your current shell is stale, open a new terminal once.

## Setup flow (once per project)

```bash
# initialize runtime if needed
agent-ruler init

# interactive runner setup
agent-ruler setup
```

`setup` prompts through:

1. Runner selection (`OpenClaw` today)
2. Optional import from host OpenClaw
3. Optional integrations

Then it creates Ruler-managed OpenClaw home/workspace and prints the exact confined run command.
For project-local OpenClaw state, use wrapper calls in this guide (`agent-ruler run -- openclaw ...`).
Agent Ruler injects `OPENCLAW_HOME` automatically for those calls.
`setup` now validates key managed files before printing success, including imported auth profiles/store, Telegram token (when Telegram token/config was present on host), and selected model/provider resolution from managed config.

Quick verification after setup:

```bash
agent-ruler status --json
agent-ruler run -- openclaw config get gateway.mode
```

Expected:
- `runner.managed_home` is set in status output
- `gateway.mode` prints `local`

## OpenClaw references used here

- Channels overview: <https://docs.openclaw.ai/channels>
- Telegram channel: <https://docs.openclaw.ai/channels/telegram>
- WhatsApp channel: <https://docs.openclaw.ai/channels/whatsapp>
- Channels CLI: <https://docs.openclaw.ai/cli/channels>
- Message CLI (buttons/polls): <https://docs.openclaw.ai/cli/message>
- Poll automation: <https://docs.openclaw.ai/automation/poll>
- OpenClaw getting started: <https://docs.openclaw.ai/start/getting-started>

## A) Baseline integration (no OpenClaw changes)

### 1) Start Agent Ruler Control Panel

```bash
agent-ruler ui
```

Open `http://127.0.0.1:4622`.
If `agent-ruler ui` is not running, the Control Panel is not available.

### 2) Run OpenClaw inside Agent Ruler confinement

Quick proof command:

```bash
agent-ruler run -- openclaw --help
```

Normal run (still through Agent Ruler):

```bash
agent-ruler run -- openclaw gateway
```

`agent-ruler run -- openclaw gateway` is detached by default and returns control immediately.
Agent Ruler writes gateway stdout/stderr to:
- `<runtime>/user_data/logs/openclaw-gateway.log`
- and records managed gateway PID metadata at `<runtime>/user_data/logs/openclaw-gateway.pid.json`

To stop the managed detached gateway cleanly:

```bash
agent-ruler run -- openclaw gateway stop
```

`gateway stop` uses only the recorded managed PID and clears stale PID metadata when process is already gone.

### 3) Verify OpenClaw is running under Agent Ruler

```bash
agent-ruler status --json
agent-ruler tail 20
```

Confirm all three:

- `status --json` shows runtime/workspace/shared-zone and approval counts
- `tail` shows governed actions/verdicts
- Control Panel timeline (`/receipts`) shows matching events

### 4) Handle approvals in baseline mode

1. Open `http://127.0.0.1:4622/approvals`
2. Review details
3. Approve or deny
4. Agent flow resumes only after resolution

## B) Seamless integration (recommended)

Agent Ruler includes an OpenClaw adapter plugin:

```text
bridge/openclaw/openclaw-agent-ruler-tools
```

Use wrapper commands below so Agent Ruler keeps OpenClaw on the managed home/workspace.

### 1) Configure OpenClaw plugin loading

```bash
AR_DIR="$HOME/agent-ruler"
agent-ruler run -- openclaw config set plugins.load.paths "[\"$AR_DIR/bridge/openclaw/openclaw-agent-ruler-tools\"]" --json
agent-ruler run -- openclaw config set plugins.entries.openclaw-agent-ruler-tools.enabled true
agent-ruler run -- openclaw config set plugins.entries.openclaw-agent-ruler-tools.config.baseUrl "http://127.0.0.1:4622"
agent-ruler run -- openclaw config set agents.list[0].tools.allow "[\"openclaw-agent-ruler-tools\"]" --json
```

Optional environment override:

```bash
export AGENT_RULER_BASE_URL="http://127.0.0.1:4622"
```

Optional local sanity check:

```bash
node "$AR_DIR/bridge/openclaw/openclaw-agent-ruler-tools/sanity-check.mjs"
```

### 2) Restart OpenClaw gateway/runtime

Restart OpenClaw after plugin configuration changes.

### 3) Keep OpenClaw inside Agent Ruler confinement

Seamless mode still uses the wrapper run:

```bash
agent-ruler run -- openclaw gateway
```

### 4) Wait/resume behavior when approval is required

When an action returns `pending_approval`:

1. Persist `approval_id`
2. Notify the user (include `open_in_webui` when present)
3. Wait with the approval wait tool/endpoint
4. On `approved`, resume the blocked step
5. On `denied` or `expired`, stop that branch and report why
6. On timeout, keep `waiting_for_operator` state and retry later

The bundled OpenClaw tools adapter now does this automatically by default (`autoWaitForApprovals=true`), including preflight-gated core tools and transfer request tools.

You can tune the default wait duration in **Control Settings** via
`Default Approval Wait Timeout (seconds)` (safe initial default `90`, range `1..300`).

## C) Approvals without babysitting (multi-surface)

### Local Control Panel + browser notifications

1. Open `http://127.0.0.1:4622/approvals`
2. Allow browser notifications
3. Keep the page open during long runs
4. Approve/deny as requests arrive

### Remote approvals via SSH tunnel (no public exposure)

#### Host (machine running Agent Ruler + OpenClaw)

```bash
agent-ruler ui --bind 127.0.0.1:4622
```

This keeps Control Panel private to host loopback.

#### Viewer (your laptop/phone)

`REMOTE_HOST` is the hostname or IP of the Host machine. `REMOTE_USER` is the Linux username you SSH into on that Host.

Example values:

```bash
REMOTE_HOST="192.168.1.50"
REMOTE_USER="operator"
ssh -N -L 4622:127.0.0.1:4622 "${REMOTE_USER}@${REMOTE_HOST}"
```

This forwards Viewer `127.0.0.1:4622` to Host `127.0.0.1:4622`.

Open on Viewer: `http://127.0.0.1:4622/approvals`

### Remote approvals via Tailscale (no public exposure)

#### Host (machine running Agent Ruler + OpenClaw)

```bash
HOST_TAIL_IP="$(tailscale ip -4 | head -n1)"
agent-ruler ui --bind "${HOST_TAIL_IP}:4622"
```

This binds Control Panel to the Host tailnet address.

#### Viewer (laptop/phone in same tailnet)

Open `http://<HOST_TAIL_IP>:4622/approvals` from the Viewer device.

## D) Chat approvals (optional, recommended)

This is optional, but recommended for faster approvals without living in the web UI.

Use wrapper commands in this section so channel/hook config stays project-local for this runtime.

- WebUI remains the canonical fallback
- Telegram/WhatsApp/Discord messages are sent through OpenClaw channels
- Approvals are accepted only from allowed sender identities on each route

### Why this flow (and not generic webhooks)

- Discord can accept generic webhook POSTs for one-way notifications.
- Telegram and WhatsApp in this project use OpenClaw channels + inbound hooks for interactive approvals (verified sender identity + deterministic parsing).
- For deep links, no domain rental is required. Use localhost + SSH tunnel or a Tailscale IP base URL.

### 1) Prerequisite: channels already configured in OpenClaw

Telegram bot setup:

```bash
agent-ruler run -- openclaw channels add --channel telegram --token "$TELEGRAM_BOT_TOKEN"
```

WhatsApp link flow:

```bash
agent-ruler run -- openclaw channels login --channel whatsapp
```

Checks:

```bash
agent-ruler run -- openclaw channels list
agent-ruler run -- openclaw channels status
agent-ruler run -- openclaw channels capabilities
```

### 2) Enable button/poll capabilities for approvals UX

Enable Telegram inline buttons for trusted accounts (`allowlist`):

```bash
agent-ruler run -- openclaw config set channels.telegram.capabilities.inlineButtons allowlist
```

Enable WhatsApp action polls for trusted accounts (`allowlist`):

```bash
agent-ruler run -- openclaw config set channels.whatsapp.capabilities.actions.polls allowlist
```

### 3) Configure the Agent Ruler OpenClaw channel bridge

No manual bridge JSON is required for the normal workflow.
`agent-ruler run -- openclaw gateway` generates a runtime-local bridge config automatically.
You can edit generated bridge runtime values from **Control Settings** (same page/section as `ui_bind` and approval wait timeout).

If you already have routes in an existing bridge config (`bridge/openclaw/channel-bridge.json` or legacy `openclaw-channel-bridge*.json` files), `agent-ruler run -- openclaw gateway` auto-seeds managed OpenClaw `approvalBridgeRoutes` from that file.

If you do not have existing routes yet, set per-channel routes in OpenClaw (preferred source of truth):

```bash
agent-ruler run -- openclaw config set plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes '[{"channel":"telegram","target":"@your_telegram_chat","allow_from":["123456789"],"account":"default","telegram_inline_buttons":true},{"channel":"whatsapp","target":"+15555550123","allow_from":["+15555550123"],"account":"default","whatsapp_use_poll":true},{"channel":"discord","target":"channel:123456789012345678","allow_from":["user:123456789012345678"],"account":"default"}]' --json
```

Verify:

```bash
agent-ruler run -- openclaw config get plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes --json
```

You should only need the manual `config set ...approvalBridgeRoutes` call once for a fresh setup with no prior bridge route data.
If `approvalBridgeRoutes` is still missing, the bridge also attempts channel-default autodiscovery from managed OpenClaw channel settings and `allowFrom` credentials.

### 4) Inbound message hook is auto-managed

On `agent-ruler run -- openclaw gateway`, Agent Ruler now:

- syncs `bridge/openclaw/approvals-hook` into managed OpenClaw hooks,
- enables `agent-ruler-approvals`,
- sets `hooks.internal.entries.agent-ruler-approvals.env.AR_OPENCLAW_BRIDGE_URL` to the generated inbound URL (`http://<inbound_bind>/inbound`).

Manual fallback (recovery only):

```bash
agent-ruler run -- openclaw hooks install "$AR_DIR/bridge/openclaw/approvals-hook"
agent-ruler run -- openclaw hooks enable agent-ruler-approvals
```

### 5) Run OpenClaw under Agent Ruler (bridge auto-start)

```bash
agent-ruler run -- openclaw gateway
```

`agent-ruler run -- openclaw gateway` starts the managed channel bridge automatically when bridge assets/config are present.
Bridge config now keeps `ruler_url` on loopback (`127.0.0.1:<ui_port>`) for local reliability.
`public_base_url` auto-detects a Tailscale IPv4 when available, and falls back to loopback when not configured.
OpenClaw tools-adapter API calls use loopback (`127.0.0.1:<ui_port>`) for low-latency preflight and wait/resume.
When UI bind is a concrete interface (for example a Tailscale IP), Agent Ruler also starts a local loopback API mirror on the same port so these calls remain reachable.
To confirm delivery flow is active:

```bash
agent-ruler status --json
tail -f "$(agent-ruler status --json | jq -r '.runtime_root')/user_data/logs/openclaw-channel-bridge.log"
```

The bridge log includes redacted milestones: `approval detected`, `message queued`, `message sent`, plus inbound decision timing (`inbound decision latency`) for button-click troubleshooting.
Inbound hook requests run in sync mode for approval callbacks (`sync=true`).
For callback decisions, the bridge now sends direct channel confirmation first (immediate visible feedback), and falls back to hook-level feedback text if channel send fails.
Telegram approval cards include a leading `/stop` control marker line (internal sequencing hint); operators can ignore it.
If a tool preflight call cannot reach Agent Ruler, the OpenClaw tools hook blocks the tool call (fail-closed) instead of bypassing policy checks.
Agent runtime policy denies `agent-ruler` CLI execution from agent tool calls; agent flows must use Agent Ruler API tools/endpoints.

### 6) Receive approvals in Telegram

Status: tested in this release.

What you see:

- approval summary
- deep link to Control Panel approval detail
- inline Approve/Deny callback buttons when enabled
- no free-text approval command requirement

### 7) Receive approvals in WhatsApp

Status: bridge support exists, but end-to-end validation is still pending for this public release.

What you see:

- approval summary
- deep link to Control Panel approval detail
- poll options first
- no free-text approval command requirement
- WebUI fallback always available

### 8) Keep Discord working

Status: bridge support exists, but end-to-end validation is still pending for this public release.

Discord uses the same bridge route model with formatted approval alerts and deep links.

## E) What the agent can see

Default posture:

- agent can work in `workspace`
- agent can stage through `shared-zone`
- agent does not directly read policy, approvals queue, receipts store, or runtime internals
- if enabled, agent can read only redacted status feed (`/api/status/feed`)

Why this matters: work artifacts stay available while operator-control state stays outside agent context.

## Runner removed or missing

If the project is configured for OpenClaw runner and `openclaw` is later missing from `PATH`, Agent Ruler shows a sticky prompt/reminder on relevant commands.

Prompt options:

1. Keep Ruler-managed OpenClaw data for this project (default)
2. Delete Ruler-managed OpenClaw data for this project
3. Re-run setup / change runner

Useful commands:

- `agent-ruler setup`
- `agent-ruler runner remove openclaw`
- `agent-ruler purge --yes` (full project runtime cleanup)

Host OpenClaw home/workspace is never modified by this cleanup flow.

## Gateway port already in use

If `openclaw gateway` fails with `already listening` / `address already in use`, Agent Ruler prints extra diagnostics:

- listener PID from `ss -ltnp`
- `OPENCLAW_HOME` for that PID when visible via `/proc/<pid>/environ`
- remediation commands

Use these in order:

```bash
agent-ruler run -- openclaw gateway stop
openclaw gateway stop
systemctl --user stop openclaw-gateway.service
```

If still listening, kill the exact PID printed by diagnostics:

```bash
kill <pid>
```

Then verify which home that PID is serving:

```bash
tr '\0' '\n' </proc/<pid>/environ | rg '^OPENCLAW_HOME='
```

Runner lifecycle examples:

```bash
# remove OpenClaw runner mapping + managed OpenClaw data for current project runtime
agent-ruler runner remove openclaw

# wire OpenClaw again
agent-ruler setup

# full runtime cleanup (stronger than runner remove)
agent-ruler purge --yes
```

## Integration Checklist (copy/paste)

### 1) Run OpenClaw inside Agent Ruler

```bash
agent-ruler run -- openclaw --help
agent-ruler run -- openclaw gateway
```

### 2) Trigger an approval

```bash
agent-ruler run -- bash -lc 'printf "integration-check\n" > integration-check.txt'
agent-ruler export integration-check.txt integration-check.txt
```

### 3) Approve via Control Panel

Open `http://127.0.0.1:4622/approvals` and resolve the request.

### 4) Optional: approve remotely via SSH tunnel

```bash
REMOTE_HOST="192.168.1.50"
REMOTE_USER="operator"
ssh -N -L 4622:127.0.0.1:4622 "${REMOTE_USER}@${REMOTE_HOST}"
```

Then open `http://127.0.0.1:4622/approvals` on Viewer device.

### 5) Optional: approve remotely via Tailscale

```bash
HOST_TAIL_IP="$(tailscale ip -4 | head -n1)"
agent-ruler ui --bind "${HOST_TAIL_IP}:4622"
```

Then open `http://<HOST_TAIL_IP>:4622/approvals` on another tailnet device.

### 6) Optional: Telegram/WhatsApp/Discord chat approvals

Manual bridge start is still available for debugging:

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
AR_DIR="$HOME/agent-ruler"
python3 "$AR_DIR/bridge/openclaw/channel_bridge.py" --config "$RUNTIME_ROOT/user_data/bridge/openclaw-channel-bridge.generated.json"
```

### 7) Optional: replay local smoke without real channels

```bash
AR_DIR="$HOME/agent-ruler"
python3 "$AR_DIR/bridge/openclaw/smoke_replay.py"
```

### 8) Optional: agent wait/resume confirmation

```bash
PENDING_ID="$(agent-ruler approve --decision list | head -n1 | cut -d'|' -f1 | xargs)"
agent-ruler wait --id "${PENDING_ID}" --timeout 90 --json
```
