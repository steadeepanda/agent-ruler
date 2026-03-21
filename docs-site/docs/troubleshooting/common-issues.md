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
- If `Runner` filter is set to a specific runner, switch it back to `All` to include historical records without `runner_id`.

## Approvals list looks empty after switching runners

- Approvals view now has a global runner filter (`All`, `OpenClaw`, `Claude Code`, `OpenCode`).
- If you were reviewing a specific runner and switch task context, reset the filter to `All`.
- Runner-specific rows show `runner_id`; older records created before runner tagging may show `unknown`.

## Confinement errors on Linux VM

Examples:
- `Operation not permitted`
- `setting up uid map`
- `Failed RTM_NEWADDR`

Actions:
1. Ensure `bubblewrap` is installed.
2. Enable unprivileged namespaces if your distro requires it.
3. OpenClaw gateway launches already use managed host mode (not bubblewrap-only), so they can still work on hosts that block user namespaces.
4. Claude Code and OpenCode runner commands fail closed when bubblewrap is unavailable (no degraded fallback for these runners).
5. If you still see raw `bwrap` errors, confirm you are using the same runtime root in that terminal:

```bash
agent-ruler status --json | jq -r '.runtime_root'
```

Environment note:
- VSCode Snap terminals usually set `XDG_DATA_HOME=~/snap/code/<rev>/.local/share`, while regular shells often use `~/.local/share`; this can point to different Agent Ruler runtimes/configs.

## OpenCode web error: `attempt to write a readonly database`

Typical cause:

- OpenCode command did not run with managed OpenCode env overrides
  (`XDG_DATA_HOME` / `XDG_STATE_HOME` / `XDG_CACHE_HOME` / `XDG_CONFIG_HOME`).

Current behavior:

- Agent Ruler enforces managed OpenCode XDG paths when runtime runner is
  configured as `opencode`.
- Runner mismatch now fails early (for example runtime is `claudecode` but
  command is `opencode`).

Checks:

```bash
agent-ruler status --json | jq '.runner'
agent-ruler run -- opencode debug paths
```

Expected:

- `runner.kind` is `opencode`
- `debug paths` shows managed runtime paths under
  `<runtime>/user_data/runners/opencode/home/...`

If mismatch is reported:

```bash
agent-ruler setup
```

Then select `OpenCode` for that runtime.

If OpenCode Web loads but shows:

`{"name":"UnknownError","data":{"message":"Error: Unable to connect. Is the computer able to access the url?"}}`

check:

```bash
agent-ruler run -- opencode web --hostname 127.0.0.1 --port 4096
ss -ltnp | rg ':4096'
curl -i http://127.0.0.1:4096/global/health
```

Expected:
- managed launch prints `web: http://127.0.0.1:4096/`
- listener exists on `127.0.0.1:4096`
- `/global/health` returns `200`

## Agent blocks waiting for approval

- Open `/approvals` in WebUI and resolve pending items.
- For autonomous usage, connect at least one operator channel path (for example bridge adapter to phone notifications).

## Telegram bridge for Claude Code / OpenCode is not sending approvals

When using `claudecode` or `opencode`, Agent Ruler can run a managed Telegram bridge (separate from OpenClaw hook flow).

Checks:

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
tail -n 120 "$RUNTIME_ROOT/user_data/logs/claudecode-telegram-channel-bridge.log"
tail -n 120 "$RUNTIME_ROOT/user_data/logs/opencode-telegram-channel-bridge.log"
cat "$RUNTIME_ROOT/user_data/bridge/claudecode-telegram-channel-bridge.generated.json"
cat "$RUNTIME_ROOT/user_data/bridge/opencode-telegram-channel-bridge.generated.json"
```

What to verify:
- `enabled=true`
- `bot_token` is present in generated bridge config
- `allow_from` contains your Telegram sender ID (or `*` for controlled local testing only)
- run `/whoami` in Telegram if you need to discover your sender ID before allowlist setup
- approval notifications should show the formatted card style (`🚨 Approval Required`, `🔗 Link`, and `Click the buttons or Reply with : ...`)

Network note:
- If network egress is isolated, allowlist Telegram API host (`api.telegram.org`) in policy.

Threading note:
- `chat_ids` are intentionally ignored for Claude Code/OpenCode runner bridge routing to prevent misconfiguration drift.
- thread/session mappings are learned when you send `/status`, `/continue`, or `/new`.

## Telegram shows `Message received` but no runner reply

This usually means the bridge accepted your message but runner execution failed.

Checks:
- confirm the bridge is enabled in Control Panel (`enabled=true` for the runner bridge config)
- run `/status` in the same thread and verify the runner/session binding is present
- inspect bridge logs for runner command failures:

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
tail -n 160 "$RUNTIME_ROOT/user_data/logs/claudecode-telegram-channel-bridge.log"
tail -n 160 "$RUNTIME_ROOT/user_data/logs/opencode-telegram-channel-bridge.log"
```

Behavior notes:
- plain text Telegram messages are forwarded through Agent Ruler `/api/run/command`
- Claude Code relay uses bound runner session key when available, otherwise the Agent Ruler session UUID
- OpenCode relay reuses the bound runner session key and auto-learns it for new Telegram-started sessions

## Telegram `/status` appears in `All` instead of the thread

In Telegram private threaded mode, `All` is an aggregate timeline. A reply that
is correctly bound to a topic can still appear there.

How to verify true routing:
- the bot response payload includes the same `message_thread_id` as your command
- `/status` response text includes the expected `Thread: <id>`
- Control Panel `Monitoring -> Runners -> Recent Sessions` shows the same thread id

Bridge behavior:
- Agent Ruler sends `/status` replies with both thread id and reply anchor first.
- If Telegram returns `Bad Request: message thread not found` for that message,
  Agent Ruler retries with reply-anchor-only to keep the response attached to
  the originating thread message.

Session continuation notes:
- If your active work started from terminal/web and you want to continue in Telegram, use `/continue` in a Telegram thread.
- For explicit binding, use `/continue <session-id>` or `/continue <runner-session-key>`.
- Use `/new [topic]` when the subject is substantially different and should not reuse the existing thread.

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

## Claude Code / OpenCode runner is configured but executable is missing

When a project is configured for `claudecode` or `opencode` and the executable is missing from `PATH`, Agent Ruler keeps the same missing-runner safety semantics.

Useful commands:
- `agent-ruler setup`
- `agent-ruler runner remove claudecode`
- `agent-ruler runner remove opencode`

Operator checks:
- Open `Monitoring -> Runners` and verify `installed`, `binary path`, and `health` fields.
- Confirm shell `PATH` resolves the configured executable (`claude` or `opencode`).

## Claude Code shows MCP connection failure for `agent_ruler`

Symptoms in older builds:
- Claude init metadata shows two MCP servers:
  - `plugin:agent-ruler-tools:agent_ruler` (`connected`)
  - `agent_ruler` (`failed`)

Cause:
- The Claude launcher injected both `--plugin-dir` and `--mcp-config`.
- Claude plugin wiring already provides the working MCP path, so the extra
  injected entry can show a redundant failed connection.

Recovery:
1. Upgrade to the fixed build (release `v0.1.8` and later private rebuilds).
2. Re-run `agent-ruler setup` for Claude Code runtime.

## Runner tool preflight path differs by runner integration

Tool preflight now has a canonical multi-runner endpoint:

- `POST /api/runners/:id/tool/preflight` where `id` is `openclaw`, `claudecode`, or `opencode`

Compatibility aliases still exist:

- `POST /api/openclaw/tool/preflight`
- `POST /api/claudecode/tool/preflight`
- `POST /api/opencode/tool/preflight`

If adapter wiring fails, check `/api/capabilities` and confirm `tool_mapping.before_tool_call` points to `/api/runners/:id/tool/preflight`.

## Claude Code / OpenCode structured output summary receipts

Agent Ruler now normalizes one-shot commands to structured output mode when possible:

- Claude Code print mode (`-p`/`--print`) auto-adds `--output-format json` when omitted.
- OpenCode one-shot `run` auto-adds `--format json` when omitted.

Each run records a summary receipt operation:

- `runner_structured_output_parse`

Use Timeline (Receipts mode) with the runner filter to inspect parser status and counts. If parser warnings appear, verify your runner version still supports the documented JSON flags.

## OpenCode error: `Model not found: zai-coding-plan/glm-4.5` under Agent Ruler

If direct `opencode` works but `agent-ruler run -- opencode ...` fails with provider/model not found, managed OpenCode auth may be missing in the project runtime.

Current behavior:
- On first `agent-ruler run -- opencode ...`, Agent Ruler seeds managed OpenCode auth from host auth when available.
- Seed source priority: `XDG_DATA_HOME/opencode/auth.json`, then `~/.local/share/opencode/auth.json`, then Snap Code paths.

Checks:

```bash
env -u SNAP -u XDG_DATA_HOME opencode auth list
env -u SNAP -u XDG_DATA_HOME agent-ruler status --json | jq -r '.runner.managed_home'
```

Then verify managed auth file exists:

```bash
MANAGED_HOME="$(env -u SNAP -u XDG_DATA_HOME agent-ruler status --json | jq -r '.runner.managed_home')"
ls -l "$MANAGED_HOME/.local/share/opencode/auth.json"
```

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
