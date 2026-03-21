# CLI Reference

## Command style

Default command style:

```bash
agent-ruler <subcommand> [args...]
```

Developer fallback from source checkout:

```bash
cargo run -- <subcommand> [args...]
```

Release-binary alternative:

```bash
./target/release/agent-ruler <subcommand> [args...]
```

If you use the CLI often, a shell helper is convenient:

```bash
ar() { agent-ruler "$@"; }
```

Cargo helper alternative:

```bash
ar() { cargo run -- "$@"; }
```

## Quick first-time flow

```bash
agent-ruler init
agent-ruler setup
agent-ruler run -- <runner> <runner-args...>
```

Control Panel is auto-started/maintained by runner launch. Use the URL printed
in terminal output.

## Core commands

- `agent-ruler init`
- `agent-ruler setup`
- `agent-ruler run -- <cmd...>`
- `agent-ruler run --background -- <cmd...>`
- `agent-ruler status [--json]`
- `agent-ruler tail [lines]`
- `agent-ruler approve --decision <list|approve|deny> [--id ...|--all]`
- `agent-ruler reset-exec --yes`
- `agent-ruler reset --yes [--keep-config]`
- `agent-ruler ui [--bind <host:port>]`
- `agent-ruler ui stop`
- `agent-ruler stop ui`
- `agent-ruler stop run -- <openclaw|claudecode|opencode>`
- `agent-ruler update --check [--json]`
- `agent-ruler update [--version vX.Y.Z] --yes [--json]`
- `agent-ruler runner remove <openclaw|claudecode|opencode> [--project <project-key>]`
- `agent-ruler purge --yes [--project <project-key>]`

## Runner lifecycle commands

- `agent-ruler setup`
  - interactive runner setup for OpenClaw, Claude Code, and OpenCode
  - provisions Ruler-managed runner home/workspace/runtime paths for this project
  - optional host import reads/copies selected config into managed home (OpenClaw path)
  - if runtime is not initialized yet, run `agent-ruler init` first
- `agent-ruler runner remove <openclaw|claudecode|opencode> [--project <project-key>]`
  - deletes only Ruler-managed runner data for that project runtime
  - removes runner association from project config
  - host runner homes are never touched
- `agent-ruler purge [--project <project-key>] --yes`
  - deletes the selected runtime directory
  - use this for full project runtime cleanup

Missing runner reminder behavior:
- checked on key project entry points (`run`, `status`, `ui`)
- non-interactive / JSON mode never blocks for input
- only runner-required operations fail fast when a selected runner binary is missing
- reminder includes resolution commands: `agent-ruler setup` and `agent-ruler runner remove <runner-id>`

## File movement commands

- `agent-ruler import <src> [dst] [--preview-only] [--force]`
- `agent-ruler export <src> <stage-dst> [--preview-only] [--force]`
- `agent-ruler deliver <stage-ref> [destination] [--preview-only] [--force] [--move-artifact]`

## Operator checks

- `agent-ruler smoke [--non-interactive]`
- `agent-ruler wait --id <approval-id> [--timeout <seconds>] [--json]`
- `agent-ruler ui [--bind 127.0.0.1:4622]`

## Native runner command support

Agent Ruler supports native runner commands. Use the same command you would run
outside the ruler, prefixed with:

```bash
agent-ruler run --
```

## OpenClaw command style

```bash
agent-ruler run -- openclaw gateway
agent-ruler run -- openclaw gateway stop
agent-ruler stop run -- openclaw
```

For OpenClaw gateway specifically, detached mode is the default behavior of that command.
Agent Ruler prints:
- managed gateway PID
- gateway log file location
- stop command

Gateway runtime files:
- log: `<runtime>/user_data/logs/openclaw-gateway.log`
- PID metadata: `<runtime>/user_data/logs/openclaw-gateway.pid.json`

`gateway stop` uses only recorded managed PID metadata and clears stale record files when the process is already gone.

New stop UX aliases:
- `agent-ruler stop ui` is equivalent to `agent-ruler ui stop`.
- `agent-ruler stop run -- openclaw` stops managed OpenClaw gateway + channel bridge.
- `agent-ruler stop run -- claudecode` stops managed Claude Code Telegram bridge.
- `agent-ruler stop run -- opencode` stops managed OpenCode Telegram bridge.

When the runner is configured, Agent Ruler injects `OPENCLAW_HOME` automatically for `run -- openclaw ...`.

## Claude Code command style

```bash
agent-ruler run -- claude
agent-ruler run -- claude remote-control
agent-ruler run -- claude remote-control stop
```

Use the normal Claude Code command style under `agent-ruler run -- ...`.
Agent Ruler keeps managed runtime paths and governance wiring in place.

## OpenCode command style

```bash
agent-ruler run -- opencode web
agent-ruler run -- opencode web stop
agent-ruler run -- opencode run "Summarize TODO.md"
```

Use the normal OpenCode command style under `agent-ruler run -- ...`.
Agent Ruler keeps managed runtime paths and governance wiring in place.

## Installer options

- Release install option A (recommended/fast script):
  - one-liner: `curl -fsSL "https://raw.githubusercontent.com/steadeepanda/agent-ruler/main/install/install.sh" | bash -s -- --release`
  - safer variant:
      * `curl -fsSLO "https://raw.githubusercontent.com/steadeepanda/agent-ruler/main/install/install.sh"`
      * `bash install.sh --release`
  - private repo/fork: set `GITHUB_TOKEN` and optional `AGENT_RULER_GITHUB_REPO=<owner>/<repo>`
- Release install option B (manual): download `agent-ruler-linux-x86_64.tar.gz` + `SHA256SUMS.txt`, run `sha256sum -c`, extract, install binary under `~/.local/share/agent-ruler/installs/<tag>/agent-ruler`, copy bundled `bridge/` + `docs-site/` into `~/.local/share/agent-ruler/installs/` when present, and link `~/.local/bin/agent-ruler`.
- Developer install (`--local`) once you have the source code:
  - `bash install/install.sh --local`
  - script handles command-path wiring automatically; if current shell is stale, open a new terminal once
- Uninstall symlink/file:
  - `bash install/install.sh --uninstall`
- Uninstall plus install-artifact cleanup:
  - `bash install/install.sh --uninstall --purge-installs`
- Optional explicit runtime-data cleanup:
  - `bash install/install.sh --uninstall --purge-data`

## Update command

- Check latest release from GitHub:
  - `agent-ruler update --check --json`
- Apply latest release in-place (no runtime data/config purge):
  - `agent-ruler update --yes`
- Pin a specific tag:
  - `agent-ruler update --version v0.1.8 --yes`
- For private repos/forks, the updater honors:
  - `GITHUB_TOKEN=<token>`
  - `AGENT_RULER_GITHUB_REPO=<owner>/<repo>`

## Reset behavior

- `agent-ruler reset-exec --yes`
  - Clears only `state/exec-layer`.
  - Keeps config, policy, receipts, approvals, staged exports.
- `agent-ruler reset --yes --keep-config`
  - Recreates runtime state while preserving existing config/policy wiring.
  - Useful when runtime state is noisy but your paths/toggles are correct.
- `agent-ruler reset --yes`
  - Full runtime reset to defaults.
  - Recreates default config/policy and clears runtime state artifacts.

## Version sync

Version source of truth is `Cargo.toml`.

After changing it, run:

```bash
scripts/sync-version.sh
```

That syncs docs/package/plugin version references to the same release number.
