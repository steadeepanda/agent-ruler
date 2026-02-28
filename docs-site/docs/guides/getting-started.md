# Getting Started

You can get Agent Ruler running in a few minutes.
This page keeps setup simple, then points you to the right next page for UI operations, agent integration, and validation.

## Prerequisites

- Linux host
- `bubblewrap` (`bwrap --version`)
- OpenClaw installed on host (if you plan to run OpenClaw right away)
- Rust toolchain (`cargo`, `rustc`) only for developer local builds (`--local`)

## Command style used in docs

Default for this guide:

```bash
agent-ruler <subcommand> [args...]
```

Developer fallback from source checkout (no local install required):

```bash
cargo run -- <subcommand> [args...]
```

Release-binary alternative (after `cargo build --release`):

```bash
./target/release/agent-ruler <subcommand> [args...]
```

Both are valid. This docs site defaults to `agent-ruler`.

## Install (release, Linux)

Option A (safest recommended/manual): Download + verify + run

```bash
# 1) Download release asset + checksums
curl -fsSLO "https://github.com/steadeepanda/agent-ruler/releases/latest/download/agent-ruler-linux-x86_64.tar.gz"
curl -fsSLO "https://github.com/steadeepanda/agent-ruler/releases/latest/download/SHA256SUMS.txt"

# 2) Verify checksum
sha256sum -c SHA256SUMS.txt

# 3) Extract
tar -xzf agent-ruler-linux-x86_64.tar.gz

# 4) Install binary (+ bundled bridge/docs assets if present) and ensure PATH.
# Replace vX.Y.Z with the release tag you installed.
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

Option B (installer script): convenient release install

One-liner variant:

```bash
curl -fsSL "https://raw.githubusercontent.com/steadeepanda/agent-ruler/main/install/install.sh" | bash -s -- --release
```

Safer script variant (if you want to inspect before running):

```bash
curl -fsSLO "https://raw.githubusercontent.com/steadeepanda/agent-ruler/main/install/install.sh"
bash install.sh --release
```


Private GitHub repos/forks are supported via:
- `GITHUB_TOKEN=<token>`
- `AGENT_RULER_GITHUB_REPO=<owner>/<repo>`

Release update (keeps runtime data/config/files):

```bash
agent-ruler update --check --json
agent-ruler update --yes
```

In the WebUI, use **Control Settings → Ruler Version → Check for Updates / Update Now**.

## Developer install (`--local`)

```bash
bash install/install.sh --local
```

This installs `agent-ruler` under your local user paths and updates `~/.local/bin/agent-ruler`.
The installer also wires a runnable `agent-ruler` command into a writable PATH location automatically.

## 1) Build (repo workflow)

```bash
cargo build --release
```

## 2) Initialize runtime state

```bash
agent-ruler init
```

Runtime data is created under:

```text
~/.local/share/agent-ruler/projects/<project-key>/
```

By default, agent edits should stay in this managed workspace. Import/copy files into it instead of pointing the agent directly at unrelated source trees.

## 3) Setup runner wiring (OpenClaw now)

```bash
agent-ruler setup
```

`setup` walks you through:

1. Runner selection (`OpenClaw` today)
2. Optional import from Host OpenClaw home/workspace
3. Optional integrations

Then it provisions Ruler-managed OpenClaw paths inside runtime and prints the exact confined run command:

```bash
agent-ruler run -- openclaw gateway
```

Path terms used in docs:

- `Host OpenClaw home/workspace`: your existing host paths (for example `~/.openclaw`), untouched by default.
- `Ruler-managed OpenClaw home/workspace`: project-local runtime paths used under Agent Ruler confinement.

## 4) Start OpenClaw under confinement

Run the exact command printed by setup.

Example shape:

```bash
agent-ruler run -- openclaw gateway
```

When the project is configured for OpenClaw, Agent Ruler automatically injects `OPENCLAW_HOME` to the Ruler-managed OpenClaw home.

Gateway launch is detached by default. Agent Ruler prints managed PID/log/stop details and returns control to your terminal.

Logs:
- `<runtime>/user_data/logs/openclaw-gateway.log`

Stop:

```bash
agent-ruler run -- openclaw gateway stop
```

## 5) Start the Control Panel UI

```bash
agent-ruler ui
```

Open `http://127.0.0.1:4622`.
Control Panel routes are available only while this process is running.

Optional custom bind:

```bash
agent-ruler ui --bind 127.0.0.1:4633
```

## 6) Confirm status quickly

```bash
agent-ruler status --json
agent-ruler tail 40
```

## 7) Runner remove / re-setup flow

If you want to remove the OpenClaw runner mapping and its project-local managed data:

```bash
agent-ruler runner remove openclaw
```

Then wire it again:

```bash
agent-ruler setup
```

Host OpenClaw home/workspace remains untouched.

## 8) Continue with the right guide

- Why Agent Ruler is built this way:
  [About Agent Ruler](/guides/about-agent-ruler)
- Operating the WebUI end-to-end:
  [Control Panel Guide](/guides/control-panel)
- OpenClaw-focused integration and approvals workflow:
  [OpenClaw Guide](/integrations/openclaw-guide)
- Endpoint contracts and deterministic wait/resume behavior:
  [OpenClaw API Reference](/integrations/openclaw-api-reference)
- Running confidence checks before unattended runs:
  [Manual Tests](/guides/manual-tests)

## Note on unattended workflows

If you expect long autonomous runs, make sure at least one operator decision path is active (WebUI and/or a bridge channel).
Without it, pending approvals will remain pending.

## Quick recovery commands

If local state gets messy, you can reset without reinstalling:

```bash
# Reset only ephemeral execution artifacts
agent-ruler reset-exec --yes

# Full runtime reset, keep your current config/policy
agent-ruler reset --yes --keep-config

# Full runtime reset, restore defaults
agent-ruler reset --yes

# Remove only OpenClaw runner data for this project runtime
agent-ruler runner remove openclaw

# Purge a full runtime directory (requires confirmation)
agent-ruler purge --yes

# Remove local installer symlink/binary
bash install/install.sh --uninstall
```
