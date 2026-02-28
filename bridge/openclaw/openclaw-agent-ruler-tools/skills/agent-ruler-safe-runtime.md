# Agent Ruler Safe Runtime Skill

Use this skill whenever the agent is running under Agent Ruler mediation.

## Core Invariants (Must Follow)

1. Use Agent Ruler API tools (`agent_ruler_*`) for cross-zone transfer and approval workflows.
2. Do not run `agent-ruler` CLI commands from agent tool calls.
3. Treat `pending_approval` as blocking; wait for resolution and avoid duplicate/retry request loops.
4. `agent_ruler_wait_for_approval` must use full approval IDs only (no short callback aliases).
5. Never approve/deny Agent Ruler approvals from the agent side; those are user/operator decisions.
6. Never bypass transfer boundaries with direct shell copy/move between zones.
7. Do not claim success until the corresponding Agent Ruler request endpoint reports a completed status.

## Zone Mental Model (Interpret User Intent Correctly)

- Zone 0 (`workspace`): agent working area.
  - Phrases like "in this repo", "project files", "codebase", "workspace" usually map here.
- Zone 1 (`user_data` / user-owned paths): user computer data.
  - Phrases like "my files", "my Downloads", "my desktop", "on my computer", "send to me" usually refer to user-owned destinations.
- Zone 2 (`shared-zone`): controlled transfer staging area between agent workspace and user destinations.

When user wording is ambiguous, infer zone by ownership:
- Agent-owned task context -> Zone 0.
- User-owned destination/source intent -> Zone 1.
- Boundary crossing between agent and user -> stage/deliver or import workflow via Agent Ruler APIs.

## Required Runtime Discovery Before Boundary Operations

Before import/export assumptions, consult runtime information:
1. Read runtime/capabilities from Agent Ruler tools (`agent_ruler_capabilities` and runtime-safe metadata available through the adapter).
2. Resolve current workspace/shared/delivery semantics from runtime-aware tool outputs.
3. Use those resolved paths/semantics in API requests instead of guessed host paths.

## Endpoint Contract (Tool Layer)

- `agent_ruler_capabilities`: discover current safe contract and workflow hints.
- `agent_ruler_status_feed`: poll redacted approval states.
- `agent_ruler_wait_for_approval`: wait for final resolution.
- `agent_ruler_request_export_stage`: workspace -> shared-zone.
- `agent_ruler_request_delivery`: shared-zone -> user destination.
- `agent_ruler_request_import`: user/external source -> workspace.

## Intent-to-Workflow Mapping

- "Send/share/deliver/export this to me/them/path":
  1. `agent_ruler_request_export_stage`
  2. `agent_ruler_request_delivery`
- "Bring/import/load this into project/workspace":
  1. `agent_ruler_request_import`
- "Approve/deny pending request":
  1. Agent Ruler approvals: user/operator-only; do not decide from agent tools.
  2. Non-Agent-Ruler operational approvals (for example external pairing/auth) may be handled only when explicitly requested and not bypassing Agent Ruler policy boundaries.

## Approval Handling Rules

- If request returns `pending_approval`, wait and report status; do not re-submit the same request while pending.
- If approval ends as denied/expired/timeout, return a blocked outcome and next step guidance.
- Prefer status/deep-link guidance for operator action context.

## Anti-Bypass Rules

- No direct network send as a substitute for delivery workflow.
- No raw shell copy/move between workspace and user destinations when transfer APIs exist.
- No calls to operator-only endpoints from agent context.

## Scope Boundary

This skill governs Agent Ruler mediated behavior and approval queue discipline.
It does not block legitimate external operational flows outside Agent Ruler scope, unless they would bypass Agent Ruler boundary controls.
