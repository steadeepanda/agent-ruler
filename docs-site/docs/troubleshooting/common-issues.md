# Common Issues

## WebUI looks stale after updates

- `agent-ruler ui` now checks docs freshness and rebuilds `docs-site/docs/.vitepress/dist` automatically when markdown/config changed.
- Restart UI after doc edits (`agent-ruler ui`) so the freshness check runs.
- If `npm` is unavailable on that host, build docs manually before restarting UI:
  `npm --prefix docs-site run docs:build`.

## Control Panel does not open

- Start the UI process first:

```bash
agent-ruler ui
```

- Then open `http://127.0.0.1:4622`.
- For non-default bind, use the exact host/port passed to `agent-ruler ui --bind ...`.

## Timeline appears empty

- Timeline now defaults to no date filter.
- If filters were applied, use `Clear` in Timeline filters.

## Confinement errors on Linux VM

Examples:
- `Operation not permitted`
- `setting up uid map`
- `Failed RTM_NEWADDR`

Actions:
1. Ensure `bubblewrap` is installed.
2. Enable unprivileged namespaces if your distro requires it.
3. If host policy still blocks namespaces, use explicit degraded mode only when necessary.

## Agent blocks waiting for approval

- Open `/approvals` in WebUI and resolve pending items.
- For autonomous usage, connect at least one operator channel path (for example bridge adapter to phone notifications).

## OpenClaw hook error: `ECONNREFUSED 127.0.0.1:4661`

Example symptom in gateway logs:
- `[agent-ruler-approvals-hook] failed to forward inbound message ... connect ECONNREFUSED 127.0.0.1:4661`

Meaning:
- OpenClaw approvals hook is running, but the Agent Ruler OpenClaw bridge sidecar is not reachable on its inbound bind.

Checks:

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
tail -n 120 "$RUNTIME_ROOT/user_data/logs/openclaw-channel-bridge.log"
tail -n 120 "$RUNTIME_ROOT/user_data/logs/openclaw-gateway.log"
agent-ruler run -- openclaw config get hooks.internal.entries.agent-ruler-approvals.env.AR_OPENCLAW_BRIDGE_URL
```

Recovery:
1. Stop and relaunch managed gateway so bridge + hook are re-provisioned together.
2. Confirm bridge log shows startup and inbound listener bind.

```bash
agent-ruler run -- openclaw gateway stop
agent-ruler run -- openclaw gateway
```

If still failing, verify nothing else owns the bridge port and that the configured bridge URL matches managed runtime logs.

## OpenClaw log: `typing TTL reached (2m)`

This log is emitted by OpenClaw when its internal typing indicator TTL expires during long operations.

Current behavior with Agent Ruler bridge:
- Bridge sends Telegram typing keepalive during pending-approval windows to reduce stalled perception.
- You may still see TTL logs from OpenClaw itself on long non-approval tasks; this does not mean the agent crashed.

Operator check:
- Watch for continued receipt/timeline updates or bridge/gateway log activity before treating it as stuck.

## I installed Agent Ruler but OpenClaw does not run under it yet

Usually this means runtime/setup was skipped.

Run:

```bash
agent-ruler init
agent-ruler setup
```

Then run the exact command printed by setup:

```bash
agent-ruler run -- openclaw gateway
```

This uses the Ruler-managed OpenClaw home/workspace. Host OpenClaw home/workspace remains untouched by default.

## `agent-ruler: command not found` after local install

`install/install.sh --local` now handles command-path wiring automatically:
- it links `agent-ruler` into a writable directory already on your current `PATH` when possible
- and writes profile fallback for future shells when needed

If your current shell still says `command not found`, open a new terminal and run:

```bash
agent-ruler --version
```

## How to test locally (fresh reinstall loop)

Use this exact sequence when validating OpenClaw + Agent Ruler lifecycle changes:

```bash
bash install/install.sh --uninstall --purge-installs --purge-data
bash install/install.sh --local
agent-ruler init
agent-ruler setup
agent-ruler run -- openclaw gateway
agent-ruler run -- openclaw gateway stop
```

What to confirm:
- setup succeeds only when managed import requirements are complete
- `run -- openclaw gateway` returns immediately (detached) and prints PID/log/stop
- `gateway stop` works in one call and clears PID metadata
- managed model/provider is correct (`agent-ruler run -- openclaw config get agents.defaults.model.primary`)

## OpenClaw runner is configured but executable is missing

When a project is configured for OpenClaw runner and `openclaw` is not in `PATH`, Agent Ruler shows a sticky prompt/reminder on relevant commands.

Choices:
- Keep Ruler-managed OpenClaw data for this project (default)
- Delete Ruler-managed OpenClaw data for this project
- Re-run setup / change runner

Useful commands:
- `agent-ruler setup`
- `agent-ruler runner remove openclaw`

Host OpenClaw home/workspace stays untouched.

Automation note:
- non-interactive and `--json` flows never block for input
- missing-runner reminders are emitted as warnings (stderr in JSON mode)

## `No API key found for provider 'anthropic'` after setup

Example symptoms:
- `No API key found for provider 'anthropic' ...`
- auth path points into managed runtime, for example:
  `<runtime>/user_data/openclaw_home/.openclaw/agents/main/agent/auth-profiles.json`
- gateway log may show lanes like `session:temp:slug-generator` when the session-memory hook triggers slug generation

Checks:

```bash
agent-ruler status --json
agent-ruler run -- openclaw config get gateway.mode
agent-ruler run -- openclaw config get agents.defaults.model.primary
agent-ruler run -- openclaw config get hooks.internal.entries.session-memory.enabled
```

Then verify managed auth files exist:

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
ls -l "$RUNTIME_ROOT/user_data/openclaw_home/.openclaw/agents/main/agent/auth-profiles.json"
ls -l "$RUNTIME_ROOT/user_data/openclaw_home/.openclaw/agents/main/agent/auth.json"
```

If import was skipped or incomplete, rerun setup and choose import:

```bash
agent-ruler setup
```

Agent Ruler setup now fails fast (instead of reporting configured) when managed auth/model requirements are incomplete.
For non-anthropic primary models, setup disables the managed `session-memory` hook (`hooks.internal.entries.session-memory.enabled=false`) to avoid anthropic fallback from slug-generation lanes.
Host OpenClaw home/workspace remains read/copy only and is never modified.

## Telegram `setMyCommands` / `deleteMyCommands` network failures

This means Telegram command sync could not reach Telegram API or token/config is missing.

Check:

```bash
agent-ruler status --json
agent-ruler run -- openclaw config get channels.telegram.enabled
agent-ruler run -- openclaw config get channels.telegram.botToken
agent-ruler run -- openclaw config get channels.telegram.token
```

If token/config is missing in managed home, rerun setup with import:

```bash
agent-ruler setup
```

If token exists but sync still fails, check whether confinement policy is in deny-all network namespace mode (outbound HTTPS blocked):

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
rg -n 'default_deny|allowlist_hosts|invert_allowlist|denylist_hosts|invert_denylist' "$RUNTIME_ROOT/state/policy.yaml"
```

When policy effectively isolates network egress, allowlist Telegram endpoints (for example `api.telegram.org`) or adjust policy for this workflow.

## Gateway port in use / wrong OPENCLAW_HOME process

If gateway start fails with `already listening` / `address already in use`:

1. Stop managed detached gateway first:

```bash
agent-ruler run -- openclaw gateway stop
```

2. Stop known host services:

```bash
openclaw gateway stop
systemctl --user stop openclaw-gateway.service
```

3. Confirm listener PID:

```bash
ss -ltnp | rg openclaw
```

4. Inspect that PID's OpenClaw home:

```bash
tr '\0' '\n' </proc/<pid>/environ | rg '^OPENCLAW_HOME='
```

If the PID still holds the port, stop it explicitly:

```bash
kill <pid>
```

Agent Ruler also prints this diagnostic automatically when gateway launch detects port-in-use style errors.

## Gateway blocks terminal

Gateway start is detached by default:

```bash
agent-ruler run -- openclaw gateway
```

Logs are written to:
- `<runtime>/user_data/logs/openclaw-gateway.log`
- PID metadata: `<runtime>/user_data/logs/openclaw-gateway.pid.json`

Stop managed detached gateway:
- `agent-ruler run -- openclaw gateway stop`

If the PID record is stale, stop clears it and reports the process is already gone.

## Docs search not opening

- Press `Ctrl+K` (or `Cmd+K` on macOS).
- Confirm local JavaScript is enabled in browser.
