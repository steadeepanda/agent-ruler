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

For a project where OpenClaw is already installed on host:

```bash
agent-ruler init
agent-ruler setup
agent-ruler run -- openclaw gateway
agent-ruler ui
```

For source-checkout workflows, replace `agent-ruler` with `cargo run --` in the same commands.

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
- `agent-ruler update --check [--json]`
- `agent-ruler update [--version vX.Y.Z] --yes [--json]`
- `agent-ruler runner remove openclaw [--project <project-key>]`
- `agent-ruler purge --yes [--project <project-key>]`

## Runner lifecycle commands

- `agent-ruler setup`
  - interactive runner setup (OpenClaw today, scaffolded for more runners later)
  - provisions Ruler-managed OpenClaw home/workspace for this project runtime
  - optional host import reads/copies selected config into managed home
  - if runtime is not initialized yet, run `agent-ruler init` first
- `agent-ruler runner remove openclaw [--project <project-key>]`
  - deletes only Ruler-managed OpenClaw home/workspace for that project runtime
  - removes runner association from project config
  - host OpenClaw home/workspace is never touched
- `agent-ruler purge [--project <project-key>] --yes`
  - deletes the selected runtime directory
  - use this for full project runtime cleanup

Missing runner reminder behavior:
- checked on key project entry points (`run`, `status`, `ui`)
- non-interactive / JSON mode never blocks for input
- only runner-required operations fail fast (for example `run -- openclaw ...` when `openclaw` is missing)
- reminder includes resolution commands: `agent-ruler setup` and `agent-ruler runner remove openclaw`

## File movement commands

- `agent-ruler import <src> [dst] [--preview-only] [--force]`
- `agent-ruler export <src> <stage-dst> [--preview-only] [--force]`
- `agent-ruler deliver <stage-ref> [destination] [--preview-only] [--force] [--move-artifact]`

## Operator checks

- `agent-ruler smoke [--non-interactive]`
- `agent-ruler wait --id <approval-id> [--timeout <seconds>] [--json]`
- `agent-ruler ui [--bind 127.0.0.1:4622]`

## OpenClaw runner command style

After `setup`, run OpenClaw under confinement with the managed home path printed by setup:

```bash
agent-ruler run -- openclaw gateway
```

For OpenClaw gateway specifically, detached mode is the default behavior of that command.
Agent Ruler prints:
- managed gateway PID
- gateway log file location
- stop command

Gateway runtime files:
- log: `<runtime>/user_data/logs/openclaw-gateway.log`
- PID metadata: `<runtime>/user_data/logs/openclaw-gateway.pid.json`

Stop command:

```bash
agent-ruler run -- openclaw gateway stop
```

`gateway stop` uses only recorded managed PID metadata and clears stale record files when the process is already gone.

If the `openclaw` executable goes missing later, Agent Ruler shows a sticky reminder and asks whether to keep or delete Ruler-managed runner data.
When the runner is configured, Agent Ruler injects `OPENCLAW_HOME` automatically for `run -- openclaw ...`.

## Installer options

- Release install option A (recommended/manual): download `agent-ruler-linux-x86_64.tar.gz` + `SHA256SUMS.txt`, run `sha256sum -c`, extract, install binary under `~/.local/share/agent-ruler/installs/<tag>/agent-ruler`, copy bundled `bridge/` + `docs-site/` into `~/.local/share/agent-ruler/installs/` when present, and link `~/.local/bin/agent-ruler`.
- Release install option B (script):
  - one-liner: `curl -fsSL "https://raw.githubusercontent.com/steadeepanda/agent-ruler/main/install/install.sh" | bash -s -- --release`
  - safer variant: 
      * `curl -fsSLO "https://raw.githubusercontent.com/steadeepanda/agent-ruler/main/install/install.sh"`
      * `bash install.sh --release`
  - private repo/fork: set `GITHUB_TOKEN` and optional `AGENT_RULER_GITHUB_REPO=<owner>/<repo>`
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
  - `agent-ruler update --version v0.1.7 --yes`
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
