# Agent Ruler Safe Runtime

Use this guidance whenever OpenCode runs under Agent Ruler mediation.

## Core Invariants (Must Follow)

1. Use Agent Ruler API tools (`agent_ruler_*`) for cross-zone transfer and approval workflows.
2. Do not run `agent-ruler` CLI commands from agent tool calls.
3. Do not invoke runner CLIs (`openclaw`, `claude`, `opencode`) from agent shell/exec tools; runner lifecycle/config actions must stay on the operator-managed `agent-ruler run -- ...` path.
4. Treat `pending_approval` as blocking; wait for resolution and avoid duplicate/retry request loops.
5. `agent_ruler_wait_for_approval` must use full approval IDs only (no short callback aliases).
6. Never approve/deny Agent Ruler approvals from the agent side; those are user/operator decisions.
7. Never bypass transfer boundaries with direct shell copy/move between zones.
8. Never directly write to user destination paths from OpenCode tools.
9. Do not claim success until the corresponding Agent Ruler request endpoint reports a completed status.

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
3. If the user did not specify a delivery destination, use Agent Ruler's default user destination directory from the runtime contract; do not guess alternate paths.
4. Use those resolved paths/semantics in API requests instead of guessed host paths.

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
  3. Omit `dst` when the user did not specify a destination so Agent Ruler uses its default user destination directory.
- "Bring/import/load this into project/workspace":
  1. `agent_ruler_request_import`
- "Approve/deny pending request":
  1. Agent Ruler approvals: user/operator-only; do not decide from agent tools.
  2. Non-Agent-Ruler operational approvals may be handled only when explicitly requested and not bypassing Agent Ruler policy boundaries.

## Approval Handling Rules

- If a request returns `pending_approval`, immediately enter wait flow in the same turn using that exact full `approval_id`; do not ask the user to tell you to wait.
- Keep the action in waiting state until approved/denied/expired or the configured wait timeout is reached.
- Use `agent_ruler_wait_for_approval` (full approval id only) and continue automatically after approved resolution.
- If a tool action is blocked/pending, follow the wait workflow immediately instead of attempting alternate direct-write or retry paths.
- If approval ends as denied/expired, return a blocked outcome and next step guidance.
- If the wait window times out and approval is still pending, return a concise "still waiting" fallback and tell the operator how to continue after approval.
- Prefer status/deep-link guidance for operator action context.

## Telegram Session and Thread Discipline

- When Telegram channel is available, use it for concise operator-facing updates (approval pending, blocked outcome, milestone completion) when that helps the user act quickly.
- Reuse the existing related Telegram thread/session for recurring task topics (for example periodic status reports) instead of creating new threads.
- Create a new Telegram thread/session only when the topic is substantially different, no suitable existing thread/session exists, or the prior thread is unavailable/deleted.
- When this runner is using the threaded Telegram bridge, prefer continuation into Telegram using `/continue` in a thread; use `/continue <session-id>` or `/continue <runner-session-key>` when explicit binding is needed.
- Keep messages brief and action-oriented; do not create noisy or duplicate thread updates.

## Anti-Bypass Rules

- No direct network send as a substitute for delivery workflow.
- No raw shell copy/move between workspace and user destinations when transfer APIs exist.
- No calls to operator-only endpoints from agent context.
- No direct reads/writes to runtime internals or protected system paths.

## Scope Boundary

This guidance governs Agent Ruler mediated behavior and approval queue discipline.
It does not block legitimate external operational flows outside Agent Ruler scope, unless they would bypass Agent Ruler boundary controls.
