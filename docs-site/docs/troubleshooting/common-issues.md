# Common Issues

Tag key:
- `[User]`: operator/runtime issues on a target machine
- `[Developer]`: local validation, docs, or release-workflow issues

## [User] `agent-ruler: command not found` after local install

Open a new shell first, then check:

```bash
agent-ruler --version
```

If the command still is not found, rerun the local installer:

```bash
bash install/install.sh --local
```

## [User] Control Panel does not open

Start the UI process first:

```bash
agent-ruler ui
```

Then open `http://127.0.0.1:4622`.

If you used a custom bind, use that exact host and port instead.

## [User] Timeline or approvals list looks empty

Before assuming data is missing, check the active filters in Control Panel:
- clear any date filter on Timeline
- reset runner filters back to `All`
- older records may not have `runner_id`, so filtering to a specific runner can hide them

## [User] OpenClaw does not run under Agent Ruler yet

This usually means runtime setup was skipped or points at a different runtime than the one you expect.

Run:

```bash
agent-ruler init
agent-ruler setup
agent-ruler status --json | jq '.runner'
```

Then launch the managed gateway:

```bash
agent-ruler run -- openclaw gateway
```

## [User] Quick diagnostics with `agent-ruler doctor`

Use Doctor first when runtime behavior and logs disagree:

```bash
agent-ruler doctor
agent-ruler doctor --repair
agent-ruler doctor --repair all
agent-ruler doctor --repair 4
agent-ruler doctor --repair 4,7
agent-ruler doctor --json
```

Interpretation notes:
- Doctor numbers every check. Use those numbers with `--repair`.
- Runner-specific checks are scoped to the active runner. For non-OpenClaw runtimes, OpenClaw checks are reported as not-applicable instead of warn/fail.
- Missing OpenClaw `approvalBridgeRoutes` can be valid when channel-default autodiscovery is active.
- If the bridge log reports `source=openclaw_unconfigured routes=0`, Doctor now reports that bridge startup is working but approval delivery is still deferred until route candidates are present.
- `--repair` only performs explicit safe local changes for the selected checks and reports each repair action.

## [User] Claude Code auth missing or not logged in

This usually means the managed Claude Code runtime does not have usable auth yet.

Recovery:

```bash
agent-ruler setup
agent-ruler run -- claude auth login
```

Use `setup` if you want Agent Ruler to refresh the managed Claude settings from your host profile. Use `claude auth login` if you want to complete the OAuth flow inside the managed runtime for this project.

If you rely on API-token/base-URL auth instead of OAuth, make sure the managed Claude settings were actually refreshed before retrying.

## [User] OpenClaw error: `No API key found for provider 'anthropic'` after setup

Example symptoms:
- `No API key found for provider 'anthropic' ...`
- auth path points into the managed runtime
- gateway log may show lanes like `session:temp:slug-generator` when the session-memory hook triggers slug generation

Checks:

```bash
agent-ruler status --json
agent-ruler run -- openclaw config get gateway.mode
agent-ruler run -- openclaw config get agents.defaults.model.primary
agent-ruler run -- openclaw config get hooks.internal.entries.session-memory.enabled
```

Then verify the managed auth files exist:

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
ls -l "$RUNTIME_ROOT/user_data/openclaw_home/.openclaw/agents/main/agent/auth-profiles.json"
ls -l "$RUNTIME_ROOT/user_data/openclaw_home/.openclaw/agents/main/agent/auth.json"
```

If import was skipped or incomplete, rerun setup and choose import:

```bash
agent-ruler setup
```

Behavior note:
- setup now fails fast when managed auth or model requirements are incomplete
- Agent Ruler now repairs managed provider/auth compatibility at the auth-profile layer instead of disabling the `session-memory` hook
- `agent-ruler run -- openclaw ...` now enforces the same provider/auth compatibility guard before command execution so hook-driven background lanes do not drift to stale Anthropic defaults

## [User] Agent blocks waiting for approval

Open `/approvals` in Control Panel and resolve the pending item.

If you expect approvals to arrive through Telegram/OpenClaw, verify at least one operator channel is configured and healthy before retrying the task.

## [User] OpenClaw hook error: `ECONNREFUSED 127.0.0.1:4661`

Meaning:
- the OpenClaw approvals hook is active
- the Agent Ruler OpenClaw bridge sidecar is not reachable on its inbound bind

Checks:

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
tail -n 120 "$RUNTIME_ROOT/user_data/logs/openclaw-channel-bridge.log"
tail -n 120 "$RUNTIME_ROOT/user_data/logs/openclaw-gateway.log"
agent-ruler run -- openclaw config get hooks.internal.entries.agent-ruler-approvals.env.AR_OPENCLAW_BRIDGE_URL
```

Recovery:

```bash
agent-ruler run -- openclaw gateway stop
agent-ruler run -- openclaw gateway
```

If the bridge log stays empty and Agent Ruler reports a startup timeout, continue with the next issue.

## [User] OpenClaw log: `typing TTL reached (2m)`

This log is emitted by OpenClaw when its internal typing indicator TTL expires during long operations.

Current behavior with Agent Ruler bridge:
- the bridge sends Telegram typing keepalive during pending-approval windows to reduce stalled perception
- you may still see TTL logs from OpenClaw itself on long non-approval tasks; that does not mean the agent crashed

Operator check:
- look for continued receipt, timeline, bridge-log, or gateway-log activity before treating the run as stuck

## [User] OpenClaw gateway startup says the port is already in use

This usually means another OpenClaw listener is already bound to the gateway port, sometimes under the wrong `OPENCLAW_HOME`.

Checks:

```bash
agent-ruler run -- openclaw gateway stop
# only if you intentionally need to clean up an external/non-Agent-Ruler OpenClaw
openclaw gateway stop
systemctl --user stop openclaw-gateway.service
ss -ltnp | rg openclaw
```

If you find a listener PID, inspect its OpenClaw home:

```bash
tr '\0' '\n' </proc/<pid>/environ | rg '^OPENCLAW_HOME='
```

If the PID is still holding the port and belongs to the wrong runtime, stop it explicitly:

```bash
kill <pid>
```

Agent Ruler also prints port-owner diagnostics automatically when gateway launch detects a port-in-use conflict.

## [User] Gateway blocks the terminal

Gateway start is detached by default:

```bash
agent-ruler run -- openclaw gateway
```

Logs are written to:
- `<runtime>/user_data/logs/openclaw-gateway.log`
- `<runtime>/user_data/logs/openclaw-gateway.pid.json`

Stop the managed detached gateway with:

```bash
agent-ruler run -- openclaw gateway stop
```

If the PID record is stale, stop clears it and reports that the process is already gone.

## [User] OpenClaw bridge startup timeout or empty bridge log

Typical error:

```text
managed bridge did not open inbound listener 127.0.0.1:4661 within startup timeout
```

Meaning:
- the bridge process started
- it did not bind the inbound HTTP listener before the startup window expired
- an empty bridge log usually means it stalled before the first `config loaded` log line

The most common cause is slow OpenClaw config discovery on a cold machine. Check the managed OpenClaw home and time the same reads the bridge uses:

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
MANAGED_HOME="$(agent-ruler status --json | jq -r '.runner.managed_home')"

time OPENCLAW_HOME="$MANAGED_HOME" \
  openclaw config get plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes --json

time OPENCLAW_HOME="$MANAGED_HOME" \
  openclaw config get channels --json
```

If each command takes multiple seconds, the host is hitting the slow-start path.

Doctor interpretation for this case:
- if `approvalBridgeRoutes` is missing but `channels` read is healthy, Doctor should not mark this as a hard failure
- route-pointer missing is compatible with bridge channel-default autodiscovery
- if the active bridge log shows `openclaw_unconfigured`, startup is considered healthy, but approvals remain deferred until a sender is paired or `allowFrom` data exists
- `agent-ruler doctor --repair 4` can only persist routes after OpenClaw has actual route candidates in `channels.*.allowFrom` or `credentials/*-allowFrom.json`

Workarounds on affected machines:
- Seed the bridge routes once so startup does not need to auto-discover them.
- Re-run the gateway after the managed routes exist.
- Keep using the same managed runtime instead of switching between different terminals with different runtime roots.

Useful files:

```bash
echo "$RUNTIME_ROOT/user_data/logs/openclaw-channel-bridge.log"
echo "$RUNTIME_ROOT/user_data/bridge/openclaw-channel-bridge.generated.json"
```

Environment note:
- Snap-based terminals and regular shells can point at different runtime/config roots because `XDG_*` values differ.

## [User] Linux confinement error: `bwrap: setting up uid map: Permission denied`

Common symptoms:
- `bwrap: setting up uid map: Permission denied`
- `Operation not permitted`
- `Failed RTM_NEWADDR`

Checks:

```bash
bwrap --version
agent-ruler status --json | jq -r '.runtime_root'
```

Fix path:
1. Ensure `bubblewrap` is installed.
2. Enable unprivileged user namespaces if your distro requires it.
3. On Ubuntu-family hosts using AppArmor user-namespace restriction, install a Bubblewrap-specific AppArmor profile for `/usr/bin/bwrap` instead of disabling AppArmor globally.

Ubuntu/AppArmor remediation target:

```bash
sudo tee /etc/apparmor.d/bwrap >/dev/null <<'EOF'
abi <abi/4.0>, include <tunables/global>
profile bwrap /usr/bin/bwrap flags=(unconfined) {
  userns,
  include if exists <local/bwrap>
}
EOF
sudo apparmor_parser -r /etc/apparmor.d/bwrap
```

Doctor targets that same file/reload path when check `1` is truly auto-repairable in the current session. If passwordless `sudo` or AppArmor tooling is unavailable, Doctor stays manual-only and says so explicitly. Do not disable AppArmor system-wide just to make Agent Ruler run.

Behavior note:
- OpenClaw gateway launches can still fall back to managed host mode on some blocked hosts.
- Claude Code and OpenCode runner commands fail closed when Bubblewrap is unavailable.
- Snap-confined shells and regular terminals can report different current-launcher results. Doctor now surfaces both the current launcher and a host-like launcher probe so the mismatch is explicit instead of silently flipping status between UI and terminal.
- Doctor only advertises `--repair 1` for this class of failure when it can actually install `/etc/apparmor.d/bwrap` and reload AppArmor in the current session. If operator-auth root or AppArmor tooling is missing, the check remains manual-only instead of pretending to auto-repair.
- The Control Panel Doctor view shows the same summary line and remediation guidance as the CLI, so mismatches between UI and terminal should now reflect real launcher-context differences rather than stale formatting or probe noise.

## [User] OpenCode web error: `attempt to write a readonly database`

Typical cause:
- OpenCode did not launch with the managed XDG paths that Agent Ruler expects

Checks:

```bash
agent-ruler status --json | jq '.runner'
agent-ruler run -- opencode debug paths
```

Expected:
- `runner.kind` is `opencode`
- reported XDG paths live under the active runtime root

If the runtime is wrong, run:

```bash
agent-ruler setup
```

Then select `OpenCode` for that runtime and retry.

If OpenCode Web loads but shows:

```text
{"name":"UnknownError","data":{"message":"Error: Unable to connect. Is the computer able to access the url?"}}
```

check:

```bash
agent-ruler run -- opencode web --hostname 127.0.0.1 --port 4096
ss -ltnp | rg ':4096'
curl -i http://127.0.0.1:4096/global/health
```

## [User] OpenCode error: `Model not found: zai-coding-plan/glm-4.5` under Agent Ruler

If direct `opencode` works but `agent-ruler run -- opencode ...` fails with provider/model not found, managed OpenCode auth may be missing in the project runtime.

Current behavior:
- on first `agent-ruler run -- opencode ...`, Agent Ruler seeds managed OpenCode auth from host auth when available
- seed source priority is `XDG_DATA_HOME/opencode/auth.json`, then `~/.local/share/opencode/auth.json`, then Snap Code paths

Checks:

```bash
env -u SNAP -u XDG_DATA_HOME opencode auth list
env -u SNAP -u XDG_DATA_HOME agent-ruler status --json | jq -r '.runner.managed_home'
```

Then verify the managed auth file exists:

```bash
MANAGED_HOME="$(env -u SNAP -u XDG_DATA_HOME agent-ruler status --json | jq -r '.runner.managed_home')"
ls -l "$MANAGED_HOME/.local/share/opencode/auth.json"
```

If only one machine fails while another works, compare the provider/model and managed auth state on both machines instead of assuming the Agent Ruler version is the cause.

## [User] Claude Code MCP connection failure

Symptoms seen in older builds:
- Claude init metadata shows two MCP servers
- `plugin:agent-ruler-tools:agent_ruler` is `connected`
- `agent_ruler` is `failed`

Cause:
- the Claude launcher injected both `--plugin-dir` and `--mcp-config`
- Claude plugin wiring already provided the working MCP path, so the extra injected entry showed a redundant failed connection

Recovery:

```bash
bash install/install.sh --local
agent-ruler setup
```

If you are on an older build, upgrade to a build that includes the Claude MCP duplication fix, then rerun setup for the Claude runtime.

## [User] Telegram bridge for Claude Code or OpenCode is not sending approvals

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
- `bot_token` is present
- `allow_from` contains your Telegram sender ID
- if you need your sender ID, run `/whoami` in Telegram first

If the host is network-restricted, allow `api.telegram.org` in policy.

## [User] Telegram `setMyCommands` or `deleteMyCommands` network failures

This usually means Telegram command sync could not reach Telegram API at that moment, or the managed token/config is missing.
It can be transient and non-fatal when Telegram messaging/approvals still work.

Checks:

```bash
agent-ruler status --json
agent-ruler run -- openclaw config get channels.telegram.enabled
agent-ruler run -- openclaw config get channels.telegram.botToken
agent-ruler run -- openclaw config get channels.telegram.token
```

If token/config is missing in the managed home, rerun setup with import:

```bash
agent-ruler setup
```

If token exists but sync still fails, check whether policy is isolating network egress and blocking `api.telegram.org`.

Doctor interpretation:
- command-sync network failures are advisory by themselves
- Doctor now checks both the OpenClaw bridge log and gateway log, because the signal can exist in the bridge log even when the gateway log is absent
- if Telegram delivery is healthy, treat this as retryable noise rather than a hard runtime failure

## [User] Telegram shows `Message received` but no runner reply

This usually means the bridge accepted the Telegram message but runner execution failed after that.

Check:

```bash
RUNTIME_ROOT="$(agent-ruler status --json | jq -r '.runtime_root')"
tail -n 160 "$RUNTIME_ROOT/user_data/logs/claudecode-telegram-channel-bridge.log"
tail -n 160 "$RUNTIME_ROOT/user_data/logs/opencode-telegram-channel-bridge.log"
```

Also confirm the session/thread binding is correct by sending `/status` in the same Telegram thread.

## [User] Telegram `/status` appears in `All` instead of the thread

In Telegram private threaded mode, `All` is an aggregate timeline. A reply can be correctly bound to a topic and still appear there.

How to verify true routing:
- the bot response payload includes the same `message_thread_id` as your command
- `/status` response text includes the expected `Thread: <id>`
- Control Panel `Monitoring -> Runners -> Recent Sessions` shows the same thread id

Bridge behavior:
- Agent Ruler sends `/status` replies with both thread id and reply anchor first
- if Telegram returns `Bad Request: message thread not found` for that message, Agent Ruler retries with reply-anchor-only to keep the response attached to the originating thread message

Session continuation notes:
- if your active work started from terminal or web and you want to continue in Telegram, use `/continue` in a Telegram thread
- for explicit binding, use `/continue <session-id>` or `/continue <runner-session-key>`
- use `/new [topic]` when the subject is substantially different and should not reuse the existing thread

## [User] Runner executable is missing

If a project is configured for a runner and the executable is no longer in `PATH`, Agent Ruler keeps the runtime association but warns on relevant commands.

Useful commands:

```bash
agent-ruler setup
agent-ruler runner remove openclaw
agent-ruler runner remove claudecode
agent-ruler runner remove opencode
```

Checks:
- in Control Panel, open `Monitoring -> Runners` and inspect `installed`, `binary path`, and `health`
- in the shell, confirm `PATH` resolves the configured executable (`openclaw`, `claude`, or `opencode`)

Behavior note:
- non-interactive and `--json` flows do not block for input
- missing-runner reminders are emitted as warnings

## [Developer] Runner tool preflight path differs by runner integration

Canonical endpoint:
- `POST /api/runners/:id/tool/preflight` where `id` is `openclaw`, `claudecode`, or `opencode`

Compatibility aliases still exist:
- `POST /api/openclaw/tool/preflight`
- `POST /api/claudecode/tool/preflight`
- `POST /api/opencode/tool/preflight`

If adapter wiring fails, check `/api/capabilities` and confirm `tool_mapping.before_tool_call` points to `/api/runners/:id/tool/preflight`.

## [Developer] Claude Code and OpenCode structured output summary receipts

Agent Ruler normalizes one-shot commands to structured output mode when possible:
- Claude Code print mode (`-p` or `--print`) auto-adds `--output-format json` when omitted
- OpenCode one-shot `run` auto-adds `--format json` when omitted

Each run records a summary receipt operation:
- `runner_structured_output_parse`

Use Timeline in Receipts mode with the runner filter to inspect parser status and counts. If parser warnings appear, verify your runner version still supports the documented JSON flags.

## [Developer] Docs or WebUI content looks stale after docs edits

Use this sequence after changing docs markdown, docs config, or docs assets:

```bash
rm -rf docs-site/docs/.vitepress/dist docs-site/docs/.vitepress/.temp
npm --prefix docs-site run docs:build
bash install/install.sh --local
agent-ruler ui stop || true
agent-ruler ui
```

## [Developer] Fresh reinstall / regression loop

Use this exact baseline when validating installer, runtime, or bridge changes:

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
- `run -- openclaw gateway` returns immediately, detached, and prints PID/log guidance
- `gateway stop` works in one call and clears PID metadata
- managed model/provider is correct:

```bash
agent-ruler run -- openclaw config get agents.defaults.model.primary
```
