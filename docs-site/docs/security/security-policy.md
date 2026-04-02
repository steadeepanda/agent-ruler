---
title: Security Policy
---

> Synced automatically from `SECURITY.md`. Edit the source file and run `npm --prefix docs-site run docs:sync`.
# Security Policy

## Supported Versions
`v0.1.10` (Linux-first) is currently maintained.

## Reporting a Vulnerability
Please report vulnerabilities privately by opening a security advisory or contacting maintainers directly before public disclosure.

Include:
- affected version/commit,
- reproduction steps,
- impact,
- suggested mitigation (if known).

## Response Expectations
- Initial triage target: 72 hours.
- Confirmed high-severity issues are prioritized for patch release.

## Security Design Principles
- Deterministic policy engine.
- Least privilege + default deny.
- Explicit approval boundaries for high-risk operations.
- Append-only receipts for forensic traceability.
- Runtime state separation from source tree by default.
- UI/API command paths reuse the same deterministic policy/runner controls as CLI.

## Runtime Data Handling
By default, mutable runtime artifacts are stored under:
- `~/.local/share/agent-ruler/projects/<project-key>/`

This includes workspace outputs, approvals, receipts, staged-export state, and quarantine artifacts.

Recommendations:
- Keep runtime root on encrypted storage for sensitive workloads.
- Restrict runtime root permissions to the executing user.
- Do not share runtime directories across untrusted users.

## Bypass and Degraded Mode Warnings
- Unsafe bypass (`--bypass`) for import/export/deliver is opt-in and should be emergency-only.
- Degraded confinement mode (`allow_degraded_confinement=true`) is opt-in and should only be used when host policy blocks bubblewrap namespaces.
- Both modes reduce security posture and are explicitly marked in receipts.

## Current Security Limitations
- v0.1.10 enforcement requires launching via Agent Ruler runner/API flows.
- Linux confinement currently uses user-space namespace controls (`bubblewrap`) without kernel driver support.
- Some hosts/VMs may restrict unprivileged user namespaces (for example `setting up uid map: Permission denied`).
- Windows enforcement is planned for a later version.

## Prompt Injection and Internet Access
Prompt injection risk is treated as a policy-governed execution risk, not as trusted instruction following.

Current protections in v0.1.10:
- Network default-deny via policy (`rules.network.default_deny=true` by default).
- Bubblewrap network namespace isolation (`--unshare-net`) when default-deny is active.
- Host allowlist enforcement for explicit command-line egress checks, including internet-enabled mode when an allowlist is configured.
- Deterministic command preflight URL host extraction before launch.
- Upload-style exfil preflight detection (for example `curl --data`, `--data-binary`, upload flags, stream upload patterns) with approval requirement.
- Interpreter preflight checks for script-path execution and stream download-to-exec command patterns.
- Secret-path deny rules and zone-based filesystem boundaries to reduce direct exfil paths.
- Receipts for blocked/allowed/pending sensitive operations.

Recommended hardening when enabling internet:
- Keep `default_deny=true` unless internet is required.
- If internet is required, use strict host allowlists and keep them minimal.
- Run in agent-mode approval flows for high-risk transfer/delivery operations.
- Review receipts during and after runs for unexpected network operations.

Limitation note:
- v0.1.10 does not provide syscall-complete outbound mediation for arbitrary binaries without kernel-level hooks; command preflight primarily covers explicit URL-bearing command patterns.
