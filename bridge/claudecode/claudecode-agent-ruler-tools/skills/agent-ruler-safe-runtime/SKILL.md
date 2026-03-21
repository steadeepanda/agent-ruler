---
name: agent-ruler-safe-runtime
description: This skill should be used when Claude Code is operating under Agent Ruler governance, when boundary operations need approval, or when deciding how to move data across workspace/shared/user destinations.
version: 1.0.0
---

# Agent Ruler Safe Runtime

Use this guidance whenever Claude Code runs under Agent Ruler.

## Core Invariants

1. Use Agent Ruler API tools (`agent_ruler_*`) for cross-zone transfer and approval workflows.
2. Do not run `agent-ruler` CLI commands from agent tool calls.
3. `pending_approval` is blocking; wait for resolution and avoid duplicate/retry request loops.
4. `agent_ruler_wait_for_approval` must use full approval IDs only.
5. Never approve or deny Agent Ruler approvals from agent context.
6. Never bypass transfer boundaries with direct shell copy/move between zones.
7. Never directly write to user destinations from runner tool calls.
8. Do not claim completion until the corresponding Agent Ruler endpoint reports final success.

## Zone Mental Model

- Zone 0 (`workspace`): agent working area.
- Zone 1 (`user_data` / user-owned paths): user computer data and delivery destinations.
- Zone 2 (`shared-zone`): controlled transfer staging area.

Map user intent by ownership:
- Project/workspace intent -> Zone 0.
- User files/destination intent -> Zone 1.
- Cross-owner transfer intent -> staged Zone 2 flow.

## Required Runtime Discovery

Before boundary operations:
1. Read `agent_ruler_capabilities`.
2. Use runtime-safe metadata exposed through Claude Code's Agent Ruler adapter.
3. If the user did not specify a delivery destination, omit `dst` so Agent Ruler uses its default user destination directory.

## Required Workflow

- Export/deliver:
  1. `agent_ruler_request_export_stage`
  2. `agent_ruler_request_delivery`
- Import into workspace:
  1. `agent_ruler_request_import`
- Approval flow:
  1. poll/wait via `agent_ruler_status_feed` and `agent_ruler_wait_for_approval`
  2. continue only after resolved approval outcome

## Approval Discipline

- If a request returns `pending_approval`, wait on the same `approval_id` immediately in the same turn.
- Do not ask the user to tell you to wait; enter wait flow automatically.
- Use `agent_ruler_wait_for_approval` with the full approval id and continue automatically after approved resolution.
- Do not retry the same blocked request while it is pending.
- If denied/expired, report a blocked outcome with next-step guidance.
- If wait timeout is reached and approval is still pending, return a concise fallback that it is still waiting and how to continue after approval.

## Telegram Session and Thread Discipline

- When Telegram channel is available, send concise operator-facing updates there when useful (approval pending, blocked outcome, milestone completion).
- Reuse the existing related Telegram thread/session for recurring task types.
- Create a new Telegram thread/session only when topic scope is substantially different, no suitable thread/session exists, or prior thread is unavailable.
- When this runner is using the threaded Telegram bridge, prefer `/continue` in Telegram to bind continuation; use `/continue <session-id>` or `/continue <runner-session-key>` for explicit binding.
- Keep thread usage low-noise and avoid duplicate thread creation.

## Prohibited Workflow

- Direct shell copy/move between workspace and user destination paths.
- Direct writes into user destination paths when transfer APIs exist.
- Any attempt to modify runtime internals, system-critical, or secrets paths.
