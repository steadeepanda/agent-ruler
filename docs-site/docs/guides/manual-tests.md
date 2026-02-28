# Manual Tests

Use this page when you want confidence before connecting a real autonomous workflow.

If setup is not complete yet, start with [Getting Started](/guides/getting-started).

## Quick pass (non-interactive)

```bash
agent-ruler smoke --non-interactive
```

Expected summary pattern:

```text
[SMOKE] Agent Ruler one-shot checks
[PASS] init --force
[PASS] confined workspace write
[PASS] system delete denied with reason code
[PASS] download/temp execution blocked
[INFO] non-interactive summary: pass=... fail=... skip=...
```

## Interactive pass (approval flow)

```bash
agent-ruler smoke
```

This mode asks you to complete approval steps so you can confirm wait/resume behavior end-to-end.

## How To Test Locally (OpenClaw lifecycle)

For OpenClaw runner lifecycle verification:

```bash
bash install/install.sh --uninstall --purge-installs --purge-data
bash install/install.sh --local
agent-ruler init
agent-ruler setup
agent-ruler run -- openclaw gateway
agent-ruler run -- openclaw gateway stop
```

Confirm:
- setup validates imported auth/model requirements before success
- gateway start is detached and prints PID/log/stop info
- stop works once and clears PID metadata
- model/provider stays correct:
  `agent-ruler run -- openclaw config get agents.defaults.model.primary`
- no anthropic fallback error appears in managed gateway log:
  `rg -n "No API key found for provider \\\"anthropic\\\"" "$(agent-ruler status --json | jq -r '.runtime_root')/user_data/logs/openclaw-gateway.log"`
- Telegram command-sync behavior is correct:
  - if token/config exists and egress is allowed, `setMyCommands`/`deleteMyCommands` succeeds
  - if it fails, diagnostics mention missing token or network egress policy hints (`api.telegram.org` allowlist)

Developer fallback:

```bash
cargo run -- <same commands as above>
```

## Release-binary alternative

If you prefer release binaries:

```bash
./target/release/agent-ruler smoke --non-interactive
```

## VM caveat

Some VM/CI environments block unprivileged namespaces.
When that happens, confinement checks can report `SKIP` with explicit hints.
