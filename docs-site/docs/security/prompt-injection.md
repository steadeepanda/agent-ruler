---
title: Security Testing Coverage (Detailed)
---
# Security Testing Coverage Matrix (OWASP + Multi-Runner E2E)

This document tracks prompt-injection defenses, confinement boundaries, approval gates,
and related abuse-path controls across OpenClaw, Claude Code, and OpenCode.

Primary evidence sources:
- `tests/owasp_scenarios.rs` (OWASP-aligned control tests)
- `tests/linux_runtime_integration.rs` and `tests/ui_api_flow.rs` (runtime + API regressions)
- `tests/runner_expansion_flow.rs` and `tests/runner_structured_output_flow.rs` (Claude/OpenCode runner mediation)
- Local multi-runner live validation ledger:
  - `local/live-validation/live-results-2026-03-12.md`

Reference:
- https://cheatsheetseries.owasp.org/cheatsheets/LLM_Prompt_Injection_Prevention_Cheat_Sheet.html

Result legend used in tables: `PASS` / `N/A` / `FAIL`

## Enforcement model

Agent Ruler does not trust model output for enforcement decisions. Decisions are deterministic from:
- action kind
- zone/path/host classification
- explicit metadata markers (`downloaded`, `stream_exec`, `upload_pattern`, etc.)
- policy toggles

## OWASP coverage matrix

### 1) Input validation and allowlisting

| OWASP Guidance | Result | Implementation | Reason Code(s) | Primary Toggle(s) | Automated Coverage |
|---|---|---|---|---|---|
| Validate untrusted external content | PASS | Untrusted-origin actions always pass deterministic policy gates | N/A (architectural) | N/A | `tests/owasp_scenarios.rs::test_indirect_injection_web_content` |
| Use allowlists for egress | PASS | Host mediation via allowlist/denylist + invert semantics | `DenyNetworkDefault`, `DenyNetworkNotAllowlisted` | `rules.network.default_deny`, host lists | `tests/owasp_scenarios.rs::test_network_default_deny` |
| Bound high-risk upload paths | PASS | Upload-style POST/PUT requires approval | `ApprovalRequiredNetworkUpload` | `rules.network.require_approval_for_post` | `tests/owasp_scenarios.rs::test_data_exfiltration_upload_approval` |

### 2) Prompt/instruction hierarchy defense

| OWASP Guidance | Result | Implementation | Reason Code(s) | Primary Toggle(s) | Automated Coverage |
|---|---|---|---|---|---|
| Untrusted instructions cannot override policy | PASS | Zone/rule evaluation is prompt-intent agnostic | `DenySystemCritical` | filesystem zone dispositions | `tests/owasp_scenarios.rs::test_instruction_hierarchy_enforcement` |
| Separate trusted and untrusted capabilities | PASS | Zone model + action-kind constraints | `DenySecrets`, `DenySystemCritical` | `zones.*`, `rules.filesystem.*` | `tests/owasp_scenarios.rs::test_zone_classification_secrets` |

### 3) Execution-chain containment

| OWASP Guidance | Result | Implementation | Reason Code(s) | Primary Toggle(s) | Automated Coverage |
|---|---|---|---|---|---|
| Block download -> exec chains | PASS | Download markers trigger deny/quarantine | `QuarantineDownloadExecChain`, `DenyExecutionDownloaded` | `rules.execution.quarantine_on_download_exec_chain` | `tests/owasp_scenarios.rs::test_download_exec_chain` |
| Block interpreter streaming exec (`curl | bash`) | PASS | Stream-exec markers trigger deterministic deny | `DenyInterpreterStreamExec` | execution rules | `tests/owasp_scenarios.rs::test_interpreter_stream_exec` |
| Prevent workspace script execution | PASS | Workspace exec denied by policy | `DenyExecutionFromWorkspace` | `rules.execution.deny_workspace_exec` | `tests/owasp_scenarios.rs::test_multistep_chain_exec_persist` |

### 4) Tool misuse and destructive operations

| OWASP Guidance | Result | Implementation | Reason Code(s) | Primary Toggle(s) | Automated Coverage |
|---|---|---|---|---|---|
| Limit destructive mass actions | PASS | Mass delete threshold escalates to approval | `ApprovalRequiredMassDelete` | `safeguards.mass_delete_threshold` | `tests/owasp_scenarios.rs::test_tool_misuse_mass_delete` |
| Protect system/secret/operator paths | PASS | Critical paths denied independent of prompt intent | `DenySystemCritical`, `DenySecrets`, `DenyPersistence` | filesystem/persistence rules | `tests/owasp_scenarios.rs::test_tool_misuse_system_access` |

### 5) Human-in-the-loop controls

| OWASP Guidance | Result | Implementation | Reason Code(s) | Primary Toggle(s) | Automated Coverage |
|---|---|---|---|---|---|
| Require approval for high-risk transitions | PASS | Shared-zone and upload-style flows require approval | `ApprovalRequiredZone2`, `ApprovalRequiredExport`, `ApprovalRequiredNetworkUpload` | shared/network controls | `tests/linux_runtime_integration.rs::export_commit_uses_diff_and_approval_pipeline` |
| Keep decisions auditable | PASS | Receipts include deterministic reason/detail | all reason codes | N/A | `tests/owasp_scenarios.rs::test_decision_has_reason_code` |

## Code-backed scenario inventory

### `tests/owasp_scenarios.rs` (18 scenarios)

| Scenario Test | Runner(s) tested | Result |
|---|---|---|
| `test_indirect_injection_web_content` | Core policy engine (all runners) | PASS |
| `test_data_exfiltration_upload_approval` | Core policy engine (all runners) | PASS |
| `test_download_exec_chain` | Core policy engine (all runners) | PASS |
| `test_tool_misuse_mass_delete` | Core policy engine (all runners) | PASS |
| `test_interpreter_stream_exec` | Core policy engine (all runners) | PASS |
| `test_zone_classification_secrets` | Core policy engine (all runners) | PASS |

### `tests/linux_runtime_integration.rs` (12 scenarios)

| Scenario Test | Runner(s) tested | Result |
|---|---|---|
| `writes_or_deletes_outside_allowed_zone_are_blocked_with_reason_code` | Runtime mediation layer (runner-agnostic) | PASS |
| `execution_gating_blocks_temp_exec_and_quarantines_download_exec_metadata` | Runtime mediation layer (runner-agnostic) | PASS |
| `prompt_injection_style_exfil_command_is_blocked_by_network_preflight` | Runtime mediation layer (runner-agnostic) | PASS |
| `upload_style_network_to_allowlisted_host_requires_approval` | Runtime mediation layer (runner-agnostic) | PASS |
| `system_persistence_attempt_is_approval_gated_with_reason_and_metadata` | Runtime mediation layer (runner-agnostic) | PASS |

### `tests/ui_api_flow.rs` (security-relevant scenarios)

| Scenario Test | Runner(s) tested | Result |
|---|---|---|
| `openclaw_tool_preflight_blocks_interpreter_stream_exec` | OpenClaw tool preflight + shared policy backend | PASS |
| `openclaw_tool_preflight_blocks_agent_ruler_internal_paths` | OpenClaw tool preflight + shared policy backend | PASS |
| `openclaw_tool_preflight_blocks_direct_delivery_destination_copy` | OpenClaw tool preflight + shared policy backend | PASS |
| `approval_wait_endpoint_reports_timeout_then_resolution` | Shared approvals API (all runners) | PASS |
| `status_feed_is_redacted_and_includes_resolved_states` | Shared status-feed API (all runners) | PASS |

### `tests/runner_expansion_flow.rs` (Claude Code + OpenCode runner parity)

| Scenario Test | Runner tested | Result |
|---|---|---|
| `claudecode_network_preflight_creates_runner_tagged_approval_and_receipt` | Claude Code | PASS |
| `claudecode_tmp_write_stays_inside_confinement_namespace` | Claude Code | PASS |
| `opencode_network_preflight_creates_runner_tagged_approval_and_receipt` | OpenCode | PASS |
| `opencode_tmp_write_stays_inside_confinement_namespace` | OpenCode | PASS |

### `tests/runner_structured_output_flow.rs` (managed runtime guards)

| Scenario Test | Runner tested | Result |
|---|---|---|
| `claudecode_run_appends_structured_output_summary_receipt` | Claude Code | PASS |
| `claudecode_run_fails_with_login_guidance_when_managed_auth_is_missing` | Claude Code | PASS |
| `opencode_run_appends_structured_output_summary_receipt` | OpenCode | PASS |
| `opencode_run_uses_managed_xdg_paths` | OpenCode | PASS |

## Multi-runner end-to-end scenario ledger

Source: `local/live-validation/live-results-2026-03-12.md`

| Runner tested | Scenario | Result | Primary observed outcome | Evidence |
|---|---|---|---|---|
| OpenClaw | 2.1 System-critical write | PASS | denied (`deny_system_critical`) | `local/live-validation/openclaw-s2-1-system-critical-force.txt` |
| OpenClaw | 2.2 Secrets-path reads | PASS | denied (`deny_secrets`) | `local/live-validation/openclaw-s2-2-secrets-retest.txt` |
| OpenClaw | 5.2 Stream exec (`curl | bash`) | PASS | denied (`deny_interpreter_stream_exec`) | `local/live-validation/openclaw-s5-2-stream-exec.txt` |
| OpenClaw | 6.1 Mass delete guard | PASS | approval-required gate hit (`approval_required_mass_delete`) | `local/live-validation/openclaw-s6-1-mass-delete-preflight-wildcard.json` |
| Claude Code | 2.1 System-critical write | PASS | blocked by pre-tool governance | `local/live-validation/claude-s2-1-system-critical-force.txt` |
| Claude Code | 2.2 Secrets-path reads | PASS | blocked | `local/live-validation/claude-s2-2-secrets.txt` |
| Claude Code | 5.2 Stream exec (`curl | bash`) | PASS | blocked | `local/live-validation/claude-s5-2-stream-exec.txt` |
| Claude Code | 7.2 System persistence | PASS | approval-required gate (`approval_required_persistence`) | `local/live-validation/claude-s7-2-system-persistence-rerun.txt` |
| OpenCode | 2.1 System-critical write | PASS | denied | `local/live-validation/opencode-s2-1-system-critical-force.txt` |
| OpenCode | 2.2 Secrets-path reads | PASS | denied (`deny_secrets`) | `local/live-validation/opencode-s2-2-secrets-preflight-id_rsa.json` |
| OpenCode | 5.2 Stream exec (`curl | bash`) | PASS | blocked | `local/live-validation/opencode-s5-2-stream-exec.txt` |
| OpenCode | 7.2 System persistence | PASS | approval-required gate (`approval_required_persistence`) | `local/live-validation/opencode-s7-2-system-persistence-rerun.txt` |

## Additional regression paths covered

| Regression / abuse path | Runner(s) tested | Result | Coverage |
|---|---|---|---|
| Direct copy bypass to delivery destination | OpenClaw + shared backend | PASS | `tests/ui_api_flow.rs::openclaw_tool_preflight_blocks_direct_delivery_destination_copy` |
| Agent attempts `agent-ruler` CLI execution in governed context | OpenClaw + shared backend | PASS | `tests/ui_api_flow.rs::openclaw_tool_preflight_blocks_agent_ruler_cli_exec` |
| Approval wait timeout/resolve behavior | Shared API (all runners) | PASS | `tests/ui_api_flow.rs::approval_wait_endpoint_reports_timeout_then_resolution` |
| Auto-approve without trusted origin | Shared API (all runners) | PASS | `tests/ui_api_flow.rs::auto_approve_requires_control_panel_origin` |

## Key reason codes for prompt-injection defense

- `DenySystemCritical`
- `DenySecrets`
- `DenyNetworkDefault`
- `DenyNetworkNotAllowlisted`
- `DenyExecutionFromWorkspace`
- `DenyInterpreterStreamExec`
- `DenyExecutionDownloaded`
- `DenyPersistence`
- `ApprovalRequiredMassDelete`
- `ApprovalRequiredNetworkUpload`
- `ApprovalRequiredZone2`
- `ApprovalRequiredExport`
- `QuarantineDownloadExecChain`
- `QuarantineInterpreterDownload`

## Known limitations

1. Syscall-complete mediation is out of scope (no kernel hook/LSM layer).
2. Memory/IPC side-channel and in-memory exfil classes are not comprehensively mediated.
3. Operator social-engineering risk is reduced by explicit reason/detail context, but not eliminated.

## Validation commands

```bash
cargo test --test owasp_scenarios -- --nocapture
cargo test --test linux_runtime_integration -- --nocapture
cargo test --test ui_api_flow -- --nocapture
```
