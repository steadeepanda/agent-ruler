# Changelog

All notable user-facing changes are documented here.

## [v0.1.9-1] - 2026-03-29

### OpenClaw bridge startup hardening
- Fixed the managed OpenClaw approvals bridge startup path so the inbound listener binds before slow OpenClaw route discovery finishes.
- Added bounded timeouts around bridge-side OpenClaw config reads and writes to prevent indefinite startup hangs on slower machines.
- Increased the outer managed OpenClaw bridge startup wait window to better tolerate cold-start config latency across different Linux hosts.
- Added regression coverage for the deferred route-resolution startup path and post-bind route auto-sync.

### Troubleshooting cleanup
- Reworked the troubleshooting docs into explicit `[User]` and `[Developer]` sections.
- Added bridge-timeout diagnostics and recovery guidance for empty bridge logs / slow OpenClaw config discovery.
- Added Bubblewrap `uid map` / AppArmor remediation guidance for Linux hosts that block user namespaces.
- Restored high-value historical troubleshooting entries for real runtime failures such as Anthropic auth import issues, gateway port conflicts, OpenCode model lookup failures, and Claude Code MCP duplication errors.

### Versioning and update UX
- Added a shared `config/app.json` manifest for release-level constants and used it to drive version/repo sync into Cargo, docs, install metadata, and OpenClaw plugin metadata.
- Kept the version sync helper as a minimal tracked-file updater so release metadata stays consistent without touching showcase asset filenames or ignored files.
- Added post-update release notes to the Control Settings update flow so successful WebUI updates now open a popup showing what changed plus any follow-up steps.

## [v0.1.9] - 2026-03-25

### Control Panel WebUI redesign
- Completed the structural and responsive Control Panel redesign across the main dashboard, policy, runners, import/export, and settings surfaces.
- Fixed the Runners Fleet tab initial layout compaction so the first render stays readable on narrower screens.
- Standardized the Import / Export refresh button style across the WebUI.
- Reworked the Policy tabs into a consistent two-column boundary layout.
- Fixed mobile header wrapping and sidebar toggle cutoff behavior on small screens.
- Locked the Control Panel shield logo to the docs-site source of truth so the branding stays consistent.
- Updated UI tests to match the new DOM structure after the redesign.


## [v0.1.8] - 2026-03-14

### Multi-runner parity (OpenClaw + Claude Code + OpenCode)
- Added first-class runner support for `claudecode` and `opencode` across setup, run, API, and Control Panel flows.
- Added runner-specific managed runtime paths, introspection, and preflight mediation parity across runners.
- Added structured output normalization and receipt parsing for Claude Code and OpenCode runs.
- Clarified and aligned runner guidance: OpenCode now follows the same Agent Ruler "rules of living" governance workflow as OpenClaw and Claude Code.

### Telegram continuity and operator UX
- Added session-aware Telegram continuation commands for threaded chats:
  - `/continue`
  - `/continue <session-id>`
  - `/continue <runner-session-key>`
  - `/new [topic]`
- Added Telegram bridge support for Claude Code and OpenCode with runtime-generated per-runner config.
- Claude Code/OpenCode runner bridges now relay plain-text Telegram messages to bound runner sessions and post assistant replies back in-thread.
- Claude Code/OpenCode bridges now keep approvals flowing during active runner requests (including pending-approval waits), instead of delaying notifications behind polling-loop work.
- Added Telegram attachment relay for Claude Code/OpenCode conversations by staging supported media into managed runner workspaces and referencing staged paths in forwarded prompts.
- Added `/whoami` for Claude Code/OpenCode Telegram bridges to simplify sender allowlist onboarding.
- Improved thread bootstrap reliability for private threaded chats by accepting native Telegram `createForumTopic` response envelopes.
- Made runner Telegram bridge startup tolerant of empty `allow_from` so token-only onboarding can begin immediately.
- Routing now ignores static `chat_ids` for Claude Code/OpenCode bridges and prefers learned active session/thread bindings to prevent drift.

### Session model and runtime reliability
- Added runner-aware session discovery APIs:
  - `GET /api/sessions`
  - `GET /api/sessions/:id`
  - `POST /api/sessions/telegram/resolve`
- Added `Monitoring -> Runners -> Recent Sessions` explorer with filtering, search, pagination, and session details.
- Session records now persist Telegram thread bindings plus runner session keys for cleaner continuity between terminal/web and Telegram workflows.
- Fixed `/api/run/command` so runner commands issued by Telegram bridge paths run with the same managed home, governance wiring, and normalization behavior as `agent-ruler run -- ...`.
- Re-ensured safe UI bind preference ordering (Tailscale when available, localhost fallback).
- Added one global runtime-path label toggle in Control Settings and wired it consistently across Overview, Approvals, Import/Export, Receipts, and Runners pages.

### Security and governance hardening
- Tightened session binding guardrails:
  - reject runner-kind switching on an existing Telegram thread/session mapping
  - reject conflicting rebind attempts to unrelated Telegram threads
- Fixed false internal-deny classification for runner tool preflight writes that are inside the active runner workspace root.
- Fixed Claude Code governance launcher behavior to avoid duplicate MCP server injection that could surface a spurious `agent_ruler` connection failure while plugin MCP wiring was healthy.

### Docs and release coherence
- Consolidated integration docs into:
  - `Integrations Guide` (`/integrations/guide`)
  - `Integrations API Reference` (`/integrations/api-reference`)
- Updated getting-started, troubleshooting, and integrations docs for threaded Telegram continuity behavior, MCP wiring expectations, and runner parity guidance.
- Refined safe-runtime guidance so thread reuse/new-thread decisions are explicit and beginner-friendly.
- Reworked detailed security coverage docs to be explicitly multi-runner and renamed the sidebar entry from `Prompt Injection (Detailed)` to `Security Testing (Detailed)`.
- Rebuilt and shipped updated docs bundle output for runtime-served help.

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
- Changed automatic update-check cadence to every 2 hours, and applied the same 2-hour backoff after failed checks to avoid 15-second retry noise.
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
