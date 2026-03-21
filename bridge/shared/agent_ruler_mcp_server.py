#!/usr/bin/env python3
"""Agent Ruler MCP server for runner-safe import/export/approval tools.

This server intentionally exposes a minimal stdio MCP surface that proxies to
Agent Ruler HTTP APIs. It never mutates host state directly.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Dict, Optional


PROTOCOL_VERSION = "2024-11-05"
SERVER_NAME = "agent-ruler-mcp"
SERVER_VERSION = "1.0.0"
DEFAULT_BASE_URL = "http://127.0.0.1:4622"
DEFAULT_APPROVAL_WAIT_TIMEOUT_SECS = 90
DEFAULT_APPROVAL_WAIT_POLL_MS = 500
TRANSPORT_LSP = "lsp"
TRANSPORT_JSONL = "jsonl"


def _json_dumps(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def _read_exact(stream: Any, size: int) -> Optional[bytes]:
    data = b""
    while len(data) < size:
        chunk = stream.read(size - len(data))
        if not chunk:
            return None
        data += chunk
    return data


def _read_lsp_message(first_line: bytes) -> Optional[Dict[str, Any]]:
    headers: Dict[str, str] = {}
    line = first_line
    while True:
        if line == b"":
            return None
        if line in (b"\r\n", b"\n"):
            break
        text = line.decode("utf-8", errors="replace").strip()
        if ":" in text:
            key, value = text.split(":", 1)
            headers[key.strip().lower()] = value.strip()
        line = sys.stdin.buffer.readline()
        if line == b"":
            return None

    raw_length = headers.get("content-length", "")
    if not raw_length:
        return None
    try:
        length = int(raw_length)
    except ValueError:
        return None
    if length <= 0:
        return None

    payload = _read_exact(sys.stdin.buffer, length)
    if payload is None:
        return None
    return json.loads(payload.decode("utf-8"))


def _read_jsonl_message(first_line: bytes) -> Optional[Dict[str, Any]]:
    buffer = bytearray(first_line)
    max_bytes = 1_048_576
    while len(buffer) <= max_bytes:
        text = bytes(buffer).decode("utf-8", errors="replace").strip()
        if not text:
            return None
        try:
            parsed = json.loads(text)
            return parsed if isinstance(parsed, dict) else None
        except json.JSONDecodeError:
            line = sys.stdin.buffer.readline()
            if line == b"":
                return None
            buffer.extend(line)
    return None


def _read_message() -> tuple[Optional[Dict[str, Any]], Optional[str]]:
    while True:
        line = sys.stdin.buffer.readline()
        if line == b"":
            return None, None
        if line in (b"\r\n", b"\n"):
            continue
        stripped = line.lstrip()
        if stripped.startswith(b"{") or stripped.startswith(b"["):
            return _read_jsonl_message(line), TRANSPORT_JSONL
        if b":" in line:
            return _read_lsp_message(line), TRANSPORT_LSP


def _write_message(message: Dict[str, Any], transport: str) -> None:
    if transport == TRANSPORT_JSONL:
        sys.stdout.write(_json_dumps(message))
        sys.stdout.write("\n")
        sys.stdout.flush()
        return

    body = _json_dumps(message).encode("utf-8")
    header = f"Content-Length: {len(body)}\r\n\r\n".encode("ascii")
    sys.stdout.buffer.write(header)
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()


def _success(request_id: Any, result: Any) -> Dict[str, Any]:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def _error(request_id: Any, code: int, message: str, data: Any = None) -> Dict[str, Any]:
    payload: Dict[str, Any] = {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }
    if data is not None:
        payload["error"]["data"] = data
    return payload


def _tool_specs() -> list[Dict[str, Any]]:
    return [
        {
            "name": "agent_ruler_capabilities",
            "description": "Read the Agent Ruler safe runtime/capabilities contract before boundary operations.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": False,
            },
        },
        {
            "name": "agent_ruler_status_feed",
            "description": "Read Agent Ruler approval/status feed (redacted). Use this for safe polling instead of retrying blocked actions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include_resolved": {"type": "boolean"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500},
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "agent_ruler_wait_for_approval",
            "description": "Wait for approval resolution by full approval id. Use when a request returns pending_approval.",
            "inputSchema": {
                "type": "object",
                "required": ["approval_id"],
                "properties": {
                    "approval_id": {"type": "string"},
                    "timeout": {"type": "integer", "minimum": 1, "maximum": 300},
                    "poll_ms": {"type": "integer", "minimum": 100, "maximum": 5000},
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "agent_ruler_request_export_stage",
            "description": "Request stage export (workspace -> shared-zone) via Agent Ruler.",
            "inputSchema": {
                "type": "object",
                "required": ["src"],
                "properties": {
                    "src": {"type": "string"},
                    "dst": {"type": "string"},
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "agent_ruler_request_delivery",
            "description": "Request delivery (shared-zone -> user destination) via Agent Ruler.",
            "inputSchema": {
                "type": "object",
                "required": ["stage_ref"],
                "properties": {
                    "stage_ref": {"type": "string"},
                    "dst": {"type": "string"},
                    "move_artifact": {"type": "boolean"},
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "agent_ruler_request_import",
            "description": "Request import (user/external source -> workspace) via Agent Ruler.",
            "inputSchema": {
                "type": "object",
                "required": ["src"],
                "properties": {
                    "src": {"type": "string"},
                    "dst": {"type": "string"},
                },
                "additionalProperties": False,
            },
        },
    ]


def _runner_context() -> Dict[str, str]:
    runner_id = os.environ.get("AGENT_RULER_RUNNER_ID", "").strip()
    return {"runner_id": runner_id} if runner_id else {}


def _agent_ruler_call(
    method: str,
    path: str,
    query: Optional[Dict[str, Any]] = None,
    body: Optional[Dict[str, Any]] = None,
    timeout_secs: int = 30,
) -> Dict[str, Any]:
    base_url = os.environ.get("AGENT_RULER_BASE_URL", DEFAULT_BASE_URL).strip() or DEFAULT_BASE_URL
    url = base_url.rstrip("/") + path
    if query:
        encoded = urllib.parse.urlencode(query, doseq=True)
        url = f"{url}?{encoded}"

    data_bytes = None
    headers = {"Content-Type": "application/json"}
    if body is not None:
        payload = dict(body)
        payload.setdefault("context", {}).update(_runner_context())
        data_bytes = _json_dumps(payload).encode("utf-8")

    request = urllib.request.Request(url=url, method=method.upper(), headers=headers, data=data_bytes)
    try:
        with urllib.request.urlopen(request, timeout=timeout_secs) as response:
            raw = response.read().decode("utf-8", errors="replace")
            if not raw.strip():
                return {}
            return json.loads(raw)
    except urllib.error.HTTPError as err:
        body_text = err.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"{method} {path} failed ({err.code}): {body_text}") from err
    except urllib.error.URLError as err:
        raise RuntimeError(f"{method} {path} failed: {err.reason}") from err


def _tool_result(payload: Any, is_error: bool = False) -> Dict[str, Any]:
    text = json.dumps(payload, ensure_ascii=False, indent=2)
    result: Dict[str, Any] = {"content": [{"type": "text", "text": text}]}
    if is_error:
        result["isError"] = True
    return result


def _require_string(arguments: Dict[str, Any], key: str) -> str:
    value = str(arguments.get(key, "")).strip()
    if not value:
        raise ValueError(f"`{key}` is required")
    return value


def _as_bool(value: Any, default: bool) -> bool:
    if isinstance(value, bool):
        return value
    return default


def _as_int(value: Any, default: int, min_value: int, max_value: int) -> int:
    try:
        raw = int(value)
    except (TypeError, ValueError):
        raw = default
    return max(min_value, min(max_value, raw))


def _handle_tool_call(name: str, arguments: Dict[str, Any]) -> Dict[str, Any]:
    if name == "agent_ruler_capabilities":
        data = _agent_ruler_call("GET", "/api/capabilities")
        return _tool_result(data)

    if name == "agent_ruler_status_feed":
        include_resolved = _as_bool(arguments.get("include_resolved"), False)
        limit = _as_int(arguments.get("limit"), 100, 1, 500)
        data = _agent_ruler_call(
            "GET",
            "/api/status/feed",
            query={"include_resolved": "true" if include_resolved else "false", "limit": limit},
        )
        return _tool_result(data)

    if name == "agent_ruler_wait_for_approval":
        approval_id = _require_string(arguments, "approval_id")
        timeout = _as_int(arguments.get("timeout"), 90, 1, 300)
        poll_ms = _as_int(arguments.get("poll_ms"), 500, 100, 5000)
        data = _agent_ruler_call(
            "GET",
            f"/api/approvals/{urllib.parse.quote(approval_id)}/wait",
            query={"timeout": timeout, "poll_ms": poll_ms},
            timeout_secs=max(30, timeout + 10),
        )
        return _tool_result(data)

    if name == "agent_ruler_request_export_stage":
        src = _require_string(arguments, "src")
        body: Dict[str, Any] = {"src": src}
        dst = str(arguments.get("dst", "")).strip()
        if dst:
            body["dst"] = dst
        data = _agent_ruler_call("POST", "/api/export/request", body=body)
        data = _apply_pending_approval_auto_wait(data)
        return _tool_result(data)

    if name == "agent_ruler_request_delivery":
        stage_ref = _require_string(arguments, "stage_ref")
        body = {"stage_ref": stage_ref}
        dst = str(arguments.get("dst", "")).strip()
        if dst:
            body["dst"] = dst
        if isinstance(arguments.get("move_artifact"), bool):
            body["move_artifact"] = arguments["move_artifact"]
        data = _agent_ruler_call("POST", "/api/export/deliver/request", body=body)
        data = _apply_pending_approval_auto_wait(data)
        return _tool_result(data)

    if name == "agent_ruler_request_import":
        src = _require_string(arguments, "src")
        body = {"src": src}
        dst = str(arguments.get("dst", "")).strip()
        if dst:
            body["dst"] = dst
        data = _agent_ruler_call("POST", "/api/import/request", body=body)
        data = _apply_pending_approval_auto_wait(data)
        return _tool_result(data)

    raise ValueError(f"unknown tool: {name}")


def _apply_pending_approval_auto_wait(payload: Dict[str, Any]) -> Dict[str, Any]:
    if not isinstance(payload, dict):
        return payload
    status = str(payload.get("status", "")).strip().lower()
    approval_id = str(payload.get("approval_id", "")).strip()
    if status != "pending_approval" or not approval_id or not _auto_wait_enabled():
        return payload

    timeout_secs = _approval_wait_timeout_secs()
    wait_data = _agent_ruler_call(
        "GET",
        f"/api/approvals/{urllib.parse.quote(approval_id)}/wait",
        query={"timeout": timeout_secs, "poll_ms": DEFAULT_APPROVAL_WAIT_POLL_MS},
        timeout_secs=max(30, timeout_secs + 10),
    )
    raw_event = wait_data.get("event") if isinstance(wait_data, dict) else None
    event = raw_event if isinstance(raw_event, dict) else {}
    verdict = (
        str(wait_data.get("verdict", "")).strip().lower()
        or str(event.get("verdict", "")).strip().lower()
    )
    resolved = bool(wait_data.get("resolved") or bool(event.get("resolved")))
    timed_out = bool(wait_data.get("timeout") or bool(event.get("timeout")))

    merged = dict(payload)
    merged["auto_wait"] = {
        "enabled": True,
        "timeout_seconds": timeout_secs,
        "resolved": resolved,
        "timeout": timed_out,
        "verdict": verdict or None,
    }

    if resolved and verdict == "approved":
        merged["status"] = "approved_after_wait"
        merged["message"] = (
            f"Approval {approval_id} approved while waiting; continue the workflow."
        )
        return merged
    if resolved and verdict in {"denied", "expired"}:
        merged["status"] = "blocked"
        merged["reason"] = f"approval_{verdict}"
        merged["message"] = (
            f"Approval {approval_id} resolved as {verdict}; operation remains blocked."
        )
        return merged
    if timed_out:
        merged["status"] = "pending_approval"
        merged["message"] = (
            f"Approval {approval_id} is still pending after waiting {timeout_secs}s."
        )
        return merged
    return merged


def _auto_wait_enabled() -> bool:
    raw = str(os.environ.get("AGENT_RULER_AUTO_WAIT_FOR_APPROVALS", "")).strip().lower()
    if raw:
        return raw not in {"0", "false", "no"}
    return True


def _approval_wait_timeout_secs() -> int:
    raw = str(os.environ.get("AGENT_RULER_APPROVAL_WAIT_TIMEOUT_SECS", "")).strip()
    try:
        parsed = int(raw)
    except (TypeError, ValueError):
        parsed = DEFAULT_APPROVAL_WAIT_TIMEOUT_SECS
    return max(1, min(300, parsed))


def _handle_request(message: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    request_id = message.get("id")
    method = message.get("method")
    params = message.get("params") or {}

    if method == "initialize":
        return _success(
            request_id,
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
            },
        )

    if method == "notifications/initialized":
        return None

    if method == "ping":
        return _success(request_id, {})

    if method == "tools/list":
        return _success(request_id, {"tools": _tool_specs()})

    if method == "tools/call":
        tool_name = str(params.get("name", "")).strip()
        arguments = params.get("arguments") or {}
        if not isinstance(arguments, dict):
            arguments = {}
        try:
            result = _handle_tool_call(tool_name, arguments)
            return _success(request_id, result)
        except Exception as err:  # noqa: BLE001
            return _success(
                request_id,
                _tool_result(
                    {
                        "status": "error",
                        "tool_name": tool_name,
                        "detail": str(err),
                    },
                    is_error=True,
                ),
            )

    if request_id is None:
        return None
    return _error(request_id, -32601, f"method not found: {method}")


def main() -> int:
    message: Optional[Dict[str, Any]] = None
    transport = TRANSPORT_LSP
    while True:
        try:
            message, incoming_transport = _read_message()
            if message is None:
                return 0
            if incoming_transport is not None:
                transport = incoming_transport
            response = _handle_request(message)
            if response is not None:
                _write_message(response, transport)
        except Exception as err:  # noqa: BLE001
            request_id = None
            if isinstance(message, dict):
                request_id = message.get("id")
            if request_id is not None:
                _write_message(
                    _error(request_id, -32603, "internal error", str(err)),
                    transport,
                )


if __name__ == "__main__":
    raise SystemExit(main())
