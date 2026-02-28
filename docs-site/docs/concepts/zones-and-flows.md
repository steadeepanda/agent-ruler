# Workspace, Shared Zone, Deliver

Agent Ruler uses zones so everyday work can stay fast while risky transitions stay explicit.

## Zone model

- `workspace` (Zone 0)
  - Main agent working directory
  - Intended for normal project edits and build/test outputs
- `user_data` (Zone 1)
  - User documents and application data
  - May be allowed, denied, or approval-gated by policy
- `shared-zone` (Zone 2)
  - Staging boundary for export/deliver workflows
- `system_critical` (Zone 3)
  - Host-critical paths
  - Always denied by design
- `secrets` (Zone 4)
  - Credentials and sensitive materials
  - Denied or tightly restricted

## OpenClaw path terms

- **Host OpenClaw home/workspace**
  - User-managed OpenClaw paths on host (for example `~/.openclaw`)
  - Untouched by default and not mounted for normal Agent Ruler OpenClaw runs
- **Ruler-managed OpenClaw home/workspace**
  - Project-local runtime paths used when OpenClaw runs under Agent Ruler
  - Home: `<runtime>/user_data/openclaw_home/`
  - Workspace: `<runtime>/workspace/` (preferred), or `<runtime>/user_data/openclaw_workspace/` when needed

This keeps existing host OpenClaw installs safe while making confined runner state deterministic per project.

## Practical runner flow in zones

Typical OpenClaw setup/run sequence:

```bash
agent-ruler init
agent-ruler setup
agent-ruler run -- openclaw gateway
```

Developer fallback from source checkout:

```bash
cargo run -- init
cargo run -- setup
cargo run -- run -- openclaw gateway
```

Zone mapping in that flow:

- OpenClaw process runs under Agent Ruler confinement
- OpenClaw config/session data is in `user_data/openclaw_home` (Ruler-managed OpenClaw home)
- Agent work happens in `workspace` (Ruler-managed OpenClaw workspace)
- Host OpenClaw home/workspace is outside confinement and untouched by default

## Transfer flow

1. Import: external input to workspace
2. Stage: workspace output to shared-zone
3. Deliver: shared-zone artifact to destination

Each boundary crossing is evaluated deterministically. That keeps transfer behavior predictable and reviewable.
In the Control Panel Import/Export tab, `Workspace` and `Shared Zone` are shown first, with `Deliveries` below, and each zone browser is scrollable for deeper directory management.

Example command flow:

```bash
# import into workspace
agent-ruler import ./input.txt imported/input.txt

# stage workspace output to shared-zone
agent-ruler export report.txt report.txt

# deliver staged artifact to destination
agent-ruler deliver report.txt
```

## Why this structure works

- Local development stays smooth inside workspace.
- High-risk boundaries are explicit and reviewable.
- Receipts provide a deterministic trace for operator debugging and audits.

## Related concepts

- For API integration, approvals, and wait/resume behavior:
  [OpenClaw API Reference](/integrations/openclaw-api-reference)
- For OpenClaw setup and operator workflow:
  [OpenClaw Guide](/integrations/openclaw-guide)
- For architecture-level internals:
  [Architecture](/concepts/architecture)
- For UI operation of these flows:
  [Control Panel Guide](/guides/control-panel)
