# Getting Started

This guide gets a single project runtime running quickly, then points you to
runner-specific docs.

## Prerequisites

- Linux host
- `bubblewrap` (`bwrap --version`)
- At least one supported runner installed on host (`openclaw`, `claude`, or `opencode`)
- Rust toolchain only if you are doing developer local installs from source

Runner choice recommendation:
- `OpenClaw`, `Claude Code`, and `OpenCode` are all first-class runner paths.
- OpenCode follows the same Agent Ruler "rules of living" governance workflow as OpenClaw and Claude Code.

## Command style used in docs

Default:

```bash
agent-ruler <subcommand> [args...]
```

Developer fallback from source checkout:

```bash
cargo run -- <subcommand> [args...]
```

## Install (release, Linux)

Option A (recommended fast): installer script

```bash
curl -fsSL "https://raw.githubusercontent.com/steadeepanda/agent-ruler/main/install/install.sh" | bash -s -- --release
```

Safer script variant:

```bash
curl -fsSLO "https://raw.githubusercontent.com/steadeepanda/agent-ruler/main/install/install.sh"
bash install.sh --release
```

Option B (manual): download, verify, install

```bash
# 1) Download release asset + checksums
curl -fsSLO "https://github.com/steadeepanda/agent-ruler/releases/latest/download/agent-ruler-linux-x86_64.tar.gz"
curl -fsSLO "https://github.com/steadeepanda/agent-ruler/releases/latest/download/SHA256SUMS.txt"

# 2) Verify checksum
sha256sum -c SHA256SUMS.txt

# 3) Extract
tar -xzf agent-ruler-linux-x86_64.tar.gz

# 4) Install binary (+ bundled bridge/docs assets if present)
mkdir -p "$HOME/.local/share/agent-ruler/installs/vX.Y.Z" "$HOME/.local/bin"
install -m 755 agent-ruler "$HOME/.local/share/agent-ruler/installs/vX.Y.Z/agent-ruler"
if [[ -d bridge ]]; then
  mkdir -p "$HOME/.local/share/agent-ruler/installs/bridge"
  cp -a bridge/. "$HOME/.local/share/agent-ruler/installs/bridge/"
fi
if [[ -d docs-site ]]; then
  mkdir -p "$HOME/.local/share/agent-ruler/installs/docs-site"
  cp -a docs-site/. "$HOME/.local/share/agent-ruler/installs/docs-site/"
fi
ln -sfn "$HOME/.local/share/agent-ruler/installs/vX.Y.Z/agent-ruler" "$HOME/.local/bin/agent-ruler"
export PATH="$HOME/.local/bin:$PATH"
```

Private repos/forks are supported via:

- `GITHUB_TOKEN=<token>`
- `AGENT_RULER_GITHUB_REPO=<owner>/<repo>`

Release update:

```bash
agent-ruler update --check --json
agent-ruler update --yes
```

In WebUI: **Control Settings -> Ruler Version -> Check for Updates / Update Now**.

## Developer install (`--local`)

```bash
bash install/install.sh --local
```

The installer builds and installs a local runnable `agent-ruler` command, so a
separate `cargo build --release` step is not required for this flow.

## 1) Initialize runtime state

```bash
agent-ruler init
```

Runtime data is created under:

```text
~/.local/share/agent-ruler/projects/<project-key>/
```

Keep agent edits inside this managed workspace by default.

## 2) Setup runner wiring

```bash
agent-ruler setup
```

`setup` guides:

1. Runner selection (`OpenClaw`, `Claude Code`, or `OpenCode`)
2. Optional host import (OpenClaw only)
3. Optional integrations (when available)

Then it provisions project-local managed paths and prints a confined run command
for the selected runner.

## 3) Start and stop your configured runner under confinement

Native runner commands are supported: use the runner's normal command shape,
just prefixed with `agent-ruler run --`.

```bash
agent-ruler run -- <runner> <runner-args...>
```

Examples:

```bash
agent-ruler run -- openclaw gateway
agent-ruler run -- openclaw gateway stop

agent-ruler run -- claude

agent-ruler run -- opencode web
agent-ruler run -- opencode web stop
```

Notes:

- Runner web sessions are detached by default and include managed PID/log output.
- OpenCode supports explicit local bind flags (`--hostname`, `--port`) and Agent
  Ruler validates requested ports before launch.
- Claude Code uses native `remote-control` mode and does not expose local
  `--port` / `--hostname` bind flags in Agent Ruler.
- OpenClaw gateway is also detached by default.

## 4) Open the Control Panel

When you start a runner with `agent-ruler run -- ...`, Agent Ruler starts and
maintains the Control Panel automatically.

Open the URL shown in your terminal:
- if Tailscale IP is available, use that URL at port `4622`
- otherwise use `http://127.0.0.1:4622`

## 5) Enable Telegram Threaded Mode (optional, 1:1 chats)

If you want Claude Code or OpenCode in Telegram, use this flow.

### Quick setup

1. Open `@BotFather`, select your bot, and enable `Threaded Mode`.
2. In Agent Ruler Control Panel, open `Control Settings`.
3. In the `Claude Code` or `OpenCode` Telegram bridge panel:
   - enable the bridge
   - paste your bot token
   - keep `Stream answers in Telegram` enabled if you want progressive replies
4. In Telegram, message your bot: `/whoami`
5. Copy that sender ID into `allow_from` in Control Panel.
6. Start a fresh 1:1 thread with the bot and run: `/status`

### Day-to-day usage

Use these commands directly in the bot thread:

- `/whoami` -> returns your Telegram sender ID (works before allowlist is set)
- `/status` -> shows runner label, session id, and Telegram thread id
- `/continue` -> links this Telegram thread to a recent computer-started session when possible
- `/continue <session-id>` or `/continue <runner-session-key>` -> explicit bind to an existing session
- `/new [topic]` -> starts a fresh Telegram topic/session for a substantially different task

Then send plain text normally. The bridge forwards your message to the bound
runner session and posts the runner reply back to the same Telegram thread.

### What stays deterministic

- one Telegram thread maps to one Agent Ruler session
- a thread stays pinned to one runner kind (`Claude Code` or `OpenCode`)
- plain text is sent to the bound runner session and replies come back in the same thread
- media uploads (photos/videos/documents/voice/audio/animations/video notes/stickers) are staged into managed runner workspace and referenced in prompt
- manual `chat_ids` entry is not used for Claude Code/OpenCode bridge routing; session/thread bindings are learned from inbound Telegram commands
- approvals stay in the same thread with clear operator actions (`✅` / `🚫`)
- approval cards include a short reason summary and operator-focused formatting (`🚨`, `🔗`, `✅`, `🚫`)
- if explicit thread-target reply is rejected by Telegram, bridge falls back to reply-anchor routing
- sessions appear in `Monitoring -> Runners -> Recent Sessions`

Thread/session reuse policy:

- recurring or same-topic work should stay in the existing related thread/session
- create a new thread/session only when topic scope is substantially different, no suitable thread exists, or old thread is unavailable

Official references:

- [Telegram bots: natively integrate AI chatbots](https://core.telegram.org/bots#natively-integrate-ai-chatbots)
- [Telegram threaded chat demo video](https://core.telegram.org/file/400780400658/2/zyAsgGtzdvg.5107918.mp4/413b3825ef972abc2a)

If the bot replies that threaded mode is required, go back to BotFather, enable
`Threaded Mode`, and start a fresh thread before retrying.

## 6) Confirm status quickly

```bash
agent-ruler status --json
agent-ruler tail 40
```

## 7) Runner remove / re-setup flow

Remove configured runner mapping and project-local managed data:

```bash
agent-ruler runner remove openclaw
agent-ruler runner remove claudecode
agent-ruler runner remove opencode
```

Then wire again:

```bash
agent-ruler setup
```

Host runner homes remain untouched by default.

## 8) Continue with the right guide

- Architecture and rationale: [About Agent Ruler](/guides/about-agent-ruler)
- WebUI operations: [Control Panel Guide](/guides/control-panel)
- Integrations workflow: [Integrations Guide](/integrations/guide)
- API contracts: [Integrations API Reference](/integrations/api-reference)

## Note on unattended workflows

For long autonomous runs, keep at least one operator decision surface active
(WebUI and/or channel bridge). Otherwise pending approvals remain pending.

## Quick recovery commands

```bash
# Reset only ephemeral execution artifacts
agent-ruler reset-exec --yes

# Full runtime reset, keep current config/policy
agent-ruler reset --yes --keep-config

# Full runtime reset, restore defaults
agent-ruler reset --yes

# Purge a runtime directory (requires confirmation)
agent-ruler purge --yes
```
