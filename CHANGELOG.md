# Changelog

All notable user-facing changes are documented here.

## [v0.1.7] - 2026-02-28

### User update workflow
- Added `agent-ruler update` command with:
  - `--check` mode for release checks,
  - optional `--version vX.Y.Z` target pinning,
  - optional JSON output for UI integration.
- Added Control Panel update check/apply flow in **Settings** with:
  - update availability indicator in header/sidebar,
  - manual check button and in-app apply action,
  - update notifications when a newer release is detected.
- Added Timeline mode switch (`Receipts` / `Logs`) with persistent UI/operator event logging, including update-check and update-apply failures for retrace/debug workflows.
- Adjusted Control Settings update-check button styling to match the displayed version badge sizing.
- Update path preserves runtime data/config and only replaces release install artifacts.

### Installer/runtime coherence
- Embedded updater now reuses the release installer script so update behavior matches release install behavior.
- Added installer stop-skip guard (`AGENT_RULER_INSTALL_SKIP_STOP=1`) used by WebUI-triggered updates to avoid interrupting the update request itself.

### Agent skill and docs UX
- Rewrote Agent Ruler runtime skill guidance for clearer Zone 0/1/2 intent mapping, capability discovery, and anti-bypass expectations.
- Added release update docs to README/getting-started/CLI references.
- Updated docs-site top branding to match Control Panel styling (title with Beta badge + version tag presentation).

## [v0.1.6] - 2026-02-28

### Release and install reliability
- Fixed Linux release checksum packaging so `SHA256SUMS.txt` verifies with the documented manual flow (`sha256sum -c SHA256SUMS.txt`) without path rewrites.
- Retained release tarball layout (`agent-ruler` + `bridge/`) while making checksum output consistent for both scripted and manual installs.

### Approval and bridge robustness
- Made approval resolution idempotent across API and CLI surfaces to avoid duplicate-click/duplicate-callback failures (`approval ... is not pending (status: Approved)`).
- Updated OpenClaw channel bridge fallback handling to treat already-resolved approvals as terminal instead of surfacing noisy errors.

### Mediation hardening
- Expanded OpenClaw tool preflight coverage to run for all non-`agent_ruler_*` tool calls and normalize common alias tool names, reducing bypass risk from naming drift.

## [v0.1.5] - 2026-02-27

### Security and enforcement
- Blocked direct delivery-path bypass attempts from confined agents (`cp`/`mv`/shell-exec variants) so user destinations can only be modified through the stage + deliver flow.
- Removed writable user-data bind mounts from Linux confinement; workspace remains writable and shared-zone remains read-only in sandbox view.
- Hardened OpenClaw tool preflight to classify file-mutating shell commands deterministically and deny direct writes to delivery destinations.
- Preserved operator boundary by denying agent-side `agent-ruler` CLI execution attempts.

### Approvals and runtime reliability
- Improved approval state persistence robustness by ensuring approval-store parent directories exist before atomic replace.
- Strengthened OpenClaw hook/bridge routing and managed config guards to reduce wiring drift and callback failures.
- Added explicit regressions tests for delivery-bypass prevention in both confined runtime and OpenClaw preflight surfaces.

### UX and workflow
- Improved OpenClaw approval reliability and callback latency with API-first decision resolution and safe fallback behavior.
- Added Telegram typing keepalive support in the OpenClaw bridge while approvals are pending to avoid “stalled” operator perception on long tasks.
- Reduced default approval wait timeout from 120s to 90s across runtime, UI, adapter config, and docs for faster feedback loops.

### Docs and structure
- Refined Control Panel/bridge docs and architecture references to match current runtime behavior.
- Reorganized UI/page module structure for clearer ownership and easier maintenance.

## [0.1.3] - 2026-02-21

### Policy and confinement hardening
- Added live persistence preflight interception for cron/systemd/autostart style behaviors.
- Reconciled network allowlist/denylist semantics with confinement behavior for deterministic mediation.
- Improved docs-code audit coverage and platform confinement module visibility.

## [0.1.2] - 2026-02-21

### Policy usability
- Added profile lock semantics and baseline safety guards in policy toggles.
- Improved timeline readability and policy panel guidance.

## [0.1.1] - 2026-02-21

### Release consistency
- Synced runtime/docs/plugin versioning to a single source-of-truth flow.
