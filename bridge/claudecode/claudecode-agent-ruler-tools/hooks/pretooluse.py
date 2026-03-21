#!/usr/bin/env python3
"""Fail-closed PreToolUse hook for Claude Code under Agent Ruler governance."""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request


DEFAULT_BASE_URL = "http://127.0.0.1:4622"
DEFAULT_TIMEOUT_MS = 10_000
DEFAULT_APPROVAL_WAIT_TIMEOUT_SECS = 90


def as_bool(value: object, default: bool) -> bool:
    if isinstance(value, bool):
        return value
    return default


def as_int(value: object, default: int, minimum: int, maximum: int) -> int:
    try:
        raw = int(value)
    except (TypeError, ValueError):
        raw = default
    return max(minimum, min(maximum, raw))


def as_text(value: object) -> str:
    if isinstance(value, str):
        return value
    return ""


def resolve_base_url() -> str:
    raw = as_text(os.environ.get("AGENT_RULER_BASE_URL")).strip()
    if not raw:
        raw = DEFAULT_BASE_URL
    return raw.rstrip("/")


def resolve_timeout_ms() -> int:
    raw = os.environ.get("AGENT_RULER_TIMEOUT_MS")
    return as_int(raw, DEFAULT_TIMEOUT_MS, 1_000, 60_000)


def resolve_wait_timeout_secs() -> int:
    raw = os.environ.get("AGENT_RULER_APPROVAL_WAIT_TIMEOUT_SECS")
    return as_int(raw, DEFAULT_APPROVAL_WAIT_TIMEOUT_SECS, 1, 300)


def should_auto_wait_for_approvals() -> bool:
    raw = as_text(os.environ.get("AGENT_RULER_AUTO_WAIT_FOR_APPROVALS")).strip().lower()
    if not raw:
        return True
    return raw not in {"0", "false", "no"}


def call_agent_ruler_json(method: str, path: str, body: dict | None = None, timeout_ms: int | None = None) -> dict:
    timeout_ms = timeout_ms or resolve_timeout_ms()
    url = resolve_base_url() + path
    payload = None
    if body is not None:
        payload = json.dumps(body).encode("utf-8")
    request = urllib.request.Request(
        url=url,
        method=method,
        headers={"Content-Type": "application/json"},
        data=payload,
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_ms / 1000.0) as response:
            text = response.read().decode("utf-8", errors="replace")
            if not text.strip():
                return {}
            return json.loads(text)
    except urllib.error.HTTPError as err:
        detail = err.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"{method} {path} failed ({err.code}): {detail}") from err
    except urllib.error.URLError as err:
        raise RuntimeError(f"{method} {path} failed: {err.reason}") from err


def build_tool_block_message(tool_name: str, payload: dict) -> str:
    status = as_text(payload.get("status")).lower()
    reason = as_text(payload.get("reason"))
    detail = as_text(payload.get("detail"))
    approval_id = as_text(payload.get("approval_id"))

    if status == "pending_approval":
        suffix = f" (approval_id={approval_id})" if approval_id else ""
        return f"Agent Ruler approval required for {tool_name}{suffix}. {approval_workflow_hint(approval_id)}"
    if reason and detail:
        return f"Agent Ruler blocked {tool_name}: {reason} ({detail}){boundary_workflow_hint(reason)}"
    if detail:
        return f"Agent Ruler blocked {tool_name}: {detail}{boundary_workflow_hint(reason)}"
    return f"Agent Ruler blocked {tool_name}{boundary_workflow_hint(reason)}"


def build_wait_failure_message(tool_name: str, approval_id: str, status: str) -> str:
    workflow_hint = approval_workflow_hint(approval_id)
    if status in {"denied", "expired"}:
        return f"Agent Ruler approval {approval_id} {status}; {tool_name} remains blocked. {workflow_hint}"
    if status in {"timeout", "pending"}:
        wait_timeout = resolve_wait_timeout_secs()
        return (
            f"Agent Ruler approval {approval_id} is still pending after waiting {wait_timeout}s; "
            f"{tool_name} remains blocked until operator resolution. {workflow_hint}"
        )
    return f"Agent Ruler approval {approval_id} unresolved; {tool_name} remains blocked. {workflow_hint}"


def approval_workflow_hint(approval_id: str) -> str:
    suffix = f" (approval_id={approval_id})" if approval_id else ""
    return (
        "Read agent_ruler_capabilities if you have not done so, then wait for operator resolution"
        + suffix
        + " using agent_ruler_wait_for_approval; do not loop-retry the same blocked action."
    )


def boundary_workflow_hint(reason_code: str) -> str:
    reason = as_text(reason_code).strip().lower()
    if reason == "deny_user_data_write":
        return " Use stage+deliver/import Agent Ruler tools for cross-zone transfers instead of direct destination writes. If the user did not specify a destination, omit dst so Agent Ruler uses its default user destination directory."
    if reason in {"deny_system_critical", "deny_secrets"}:
        return " Stay within workspace-safe paths, read agent_ruler_capabilities, and use Agent Ruler transfer tools."
    return " Follow Agent Ruler safe runtime workflow for boundary operations and use agent_ruler_capabilities for runtime discovery instead of guessing."


def wait_for_approval(approval_id: str) -> str:
    timeout = resolve_wait_timeout_secs()
    timeout_ms = max(resolve_timeout_ms(), timeout * 1000 + 5_000)
    response = call_agent_ruler_json(
        "GET",
        "/api/approvals/"
        + urllib.parse.quote(approval_id)
        + f"/wait?timeout={timeout}&poll_ms=500",
        timeout_ms=timeout_ms,
    )
    event = response.get("event")
    event_verdict = ""
    if isinstance(event, dict):
        event_verdict = as_text(event.get("verdict")).lower()
    verdict = as_text(response.get("verdict")).lower() or event_verdict
    resolved = as_bool(response.get("resolved"), False)
    timeout_hit = as_bool(response.get("timeout"), False)
    if isinstance(event, dict):
        resolved = as_bool(event.get("resolved"), resolved)
        timeout_hit = as_bool(event.get("timeout"), timeout_hit)
    if resolved:
        if verdict == "approved":
            return "approved"
        if verdict == "denied":
            return "denied"
        if verdict == "expired":
            return "expired"
        return "unknown"
    if timeout_hit:
        return "timeout"
    return "pending"


def allow_response(message: str | None = None) -> dict:
    payload: dict = {"continue": True}
    if message:
        payload["systemMessage"] = message
    return payload


def block_response(message: str) -> dict:
    return {
        "continue": False,
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
        },
        "systemMessage": message,
    }


def main() -> int:
    try:
        hook_input = json.load(sys.stdin)
    except Exception as err:  # noqa: BLE001
        print(json.dumps(block_response(f"Agent Ruler preflight input parse failed: {err}")))
        return 0

    tool_name = as_text(hook_input.get("tool_name")).strip()
    if not tool_name:
        print(json.dumps(allow_response()))
        return 0

    # Agent Ruler MCP helper tools are explicitly mediated by Agent Ruler APIs.
    if tool_name.lower().startswith("mcp__agent_ruler__"):
        print(json.dumps(allow_response()))
        return 0

    tool_input = hook_input.get("tool_input")
    if not isinstance(tool_input, dict):
        tool_input = {}

    request_body = {
        "tool_name": tool_name,
        "params": tool_input,
        "context": {
            "session_key": as_text(hook_input.get("session_id")),
        },
    }

    try:
        preflight = call_agent_ruler_json(
            "POST",
            "/api/claudecode/tool/preflight",
            body=request_body,
        )
    except Exception as err:  # noqa: BLE001
        print(
            json.dumps(
                block_response(
                    f"Agent Ruler preflight unavailable; blocked {tool_name} to preserve policy enforcement ({err}). Remain in workspace-safe mode until Agent Ruler connectivity is restored."
                )
            )
        )
        return 0

    if not as_bool(preflight.get("blocked"), False):
        print(json.dumps(allow_response()))
        return 0

    status = as_text(preflight.get("status"))
    approval_id = as_text(preflight.get("approval_id"))
    if (
        should_auto_wait_for_approvals()
        and status == "pending_approval"
        and approval_id
    ):
        wait_status = wait_for_approval(approval_id)
        if wait_status == "approved":
            print(json.dumps(allow_response()))
            return 0
        print(json.dumps(block_response(build_wait_failure_message(tool_name, approval_id, wait_status))))
        return 0

    print(json.dumps(block_response(build_tool_block_message(tool_name, preflight))))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
