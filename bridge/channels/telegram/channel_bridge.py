#!/usr/bin/env python3
"""Runner-agnostic Telegram bridge for Agent Ruler approvals.

This bridge polls Agent Ruler status feed for pending approvals, sends Telegram
notifications, and processes approve/deny commands from Telegram callbacks/text.
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import mimetypes
import os
import queue
import re
import secrets
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Dict, Iterable, List, Optional, Sequence, Tuple
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen

DEFAULT_HTTP_TIMEOUT_SECONDS = 10.0
DEFAULT_STATUS_FEED_LIMIT = 200
SHORT_ID_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
DEFAULT_SHORT_ID_LENGTH = 6
DEFAULT_POLL_INTERVAL_SECONDS = 8
DEFAULT_TYPING_KEEPALIVE_SECONDS = 4
DEFAULT_STREAM_EDIT_INTERVAL_SECONDS = 1.2
DEFAULT_PROGRESS_NOTICE_DELAY_SECONDS = 8.0
MAX_APPROVAL_TARGET_PREVIEW = 4
MAX_TELEGRAM_MESSAGE_LENGTH = 3900
TRANSFER_APPROVAL_CATEGORIES = {"shared_zone_stage", "deliver"}
TRANSFER_APPROVAL_OPERATIONS = {"export_commit", "deliver_commit", "import_copy"}
RUNNER_LABELS = {
    "claudecode": "Claude Code",
    "opencode": "OpenCode",
    "openclaw": "OpenClaw",
}

_CALLBACK_PATTERN = re.compile(
    r"^ar:(approve|deny):([A-Za-z0-9._:-]+)(?::[A-Za-z0-9._~-]+)?$",
    re.IGNORECASE,
)
_COMMAND_PATTERN = re.compile(
    r"^/?(approve|deny)\s+([A-Za-z0-9._:-]+)(?:\s+[A-Za-z0-9._~-]+)?\s*$",
    re.IGNORECASE,
)
_SHORT_ID_PATTERN = re.compile(r"^[A-Z2-9]{4,10}$")
_SESSION_ID_PATTERN = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
    re.IGNORECASE,
)
_REASON_DESCRIPTIONS = {
    "approval_required": "A protected action needs confirmation before the runner can continue.",
    "approval_required_zone2": "Writing to the shared zone requires approval.",
    "approval_required_export": "Moving data across Agent Ruler transfer boundaries requires approval.",
    "approval_required_mass_delete": "This request looks like a mass delete and needs explicit approval.",
    "approval_required_network_upload": "Uploading data to a network destination requires approval.",
    "approval_required_large_overwrite": "A large overwrite is requested and needs approval.",
    "approval_required_suspicious_pattern": "A high-risk operation pattern was detected and needs approval.",
    "approval_required_elevation": "System package installation needs operator approval.",
    "approval_required_persistence": "Persistence or startup changes need explicit approval.",
    "approval_required_system_critical": "This touches a system-critical path and needs approval.",
    "approval_required_user_data_write": "Writing to a user-data location needs approval.",
}
_CATEGORY_DESCRIPTIONS = {
    "approval_required": "A protected action needs confirmation before the runner can continue.",
    "shared_zone_write": "Writing to the shared zone requires approval.",
    "shared_zone_stage": "Staging files from workspace to shared zone requires approval.",
    "deliver": "Delivering files from shared zone to user destination requires approval.",
    "network_upload": "Uploading data to a network destination requires approval.",
    "mass_delete": "A potential mass delete operation requires approval.",
    "large_overwrite": "A large overwrite operation requires approval.",
    "suspicious_pattern": "A high-risk operation pattern requires approval.",
    "elevation": "System package installation requires operator approval.",
    "persistence": "Persistence or startup changes require approval.",
}


class BridgeError(Exception):
    """Bridge-specific exception."""


def log_info(message: str) -> None:
    print(f"[telegram-bridge] {message}", flush=True)


def already_resolved_status_from_error(detail: str) -> Optional[str]:
    lowered = (detail or "").strip().lower()
    if "not pending" not in lowered and "already resolved" not in lowered:
        return None
    if "approved" in lowered:
        return "approved"
    if "denied" in lowered:
        return "denied"
    if "expired" in lowered:
        return "expired"
    return None


def is_thread_send_rejection(detail: str) -> bool:
    lowered = (detail or "").strip().lower()
    return "message thread not found" in lowered


@dataclass
class BridgeConfig:
    runner_kind: str
    enabled: bool
    answer_streaming_enabled: bool
    ruler_url: str
    public_base_url: str
    poll_interval_seconds: int
    decision_ttl_seconds: int
    short_id_length: int
    state_file: Path
    runtime_dir: str
    bot_token: str
    chat_ids: List[str]
    allow_from: List[str]


@dataclass(frozen=True)
class ChatTarget:
    chat_id: str
    message_thread_id: Optional[int]


@dataclass
class PendingApproval:
    approval_id: str
    short_id: str
    created_at: int
    notified: bool = True


@dataclass
class ParsedDecisionCommand:
    decision: str
    reference: str


@dataclass
class TelegramAttachment:
    kind: str
    file_name: str
    mime_type: str
    size_bytes: int
    host_path: Path
    prompt_path: str


@dataclass
class ConversationTask:
    text: str
    attachments: Sequence[TelegramAttachment]
    session: Dict[str, Any]
    chat_id: str
    thread_id: Optional[int]
    reply_anchor: Optional[int]


@dataclass
class StreamReplyState:
    enabled: bool
    message_id: Optional[int] = None
    last_sent_text: str = ""
    last_emitted_message: str = ""
    last_update_at: float = 0.0
    saw_stream_activity: bool = False


class AgentRulerClient:
    def __init__(self, base_url: str, timeout_seconds: float = DEFAULT_HTTP_TIMEOUT_SECONDS):
        self.base_url = (base_url.strip() or "http://127.0.0.1:4622").rstrip("/")
        self.timeout_seconds = timeout_seconds

    def status_feed(self) -> List[Dict[str, Any]]:
        payload = self._request_json(
            "GET",
            (
                "/api/status/feed?"
                f"include_resolved=false&limit={max(1, min(DEFAULT_STATUS_FEED_LIMIT, 500))}"
            ),
        )
        if not isinstance(payload, list):
            raise BridgeError(f"unexpected /api/status/feed payload: {payload!r}")
        return payload

    def resolve(self, approval_id: str, decision: str) -> Dict[str, Any]:
        decision = decision.strip().lower()
        if decision not in {"approve", "deny"}:
            raise BridgeError(f"unsupported decision `{decision}`")
        safe = quote(approval_id, safe="")
        path = f"/api/approvals/{safe}/approve" if decision == "approve" else f"/api/approvals/{safe}/deny"
        try:
            payload = self._request_json("POST", path, body={})
            if isinstance(payload, dict):
                payload.setdefault("status", "approved" if decision == "approve" else "denied")
                return payload
            return {"status": "approved" if decision == "approve" else "denied"}
        except BridgeError as err:
            resolved = already_resolved_status_from_error(str(err))
            expected = "approved" if decision == "approve" else "denied"
            if resolved == expected:
                return {"status": expected, "already_resolved": True}
            if resolved is not None:
                raise BridgeError(
                    f"approval already resolved as `{resolved}`; requested `{expected}`"
                ) from err
            raise

    def resolve_telegram_session(
        self,
        *,
        runner_kind: str,
        chat_id: str,
        thread_id: int,
        message_anchor_id: Optional[int] = None,
        title: Optional[str] = None,
        bind_session_id: Optional[str] = None,
        bind_runner_session_key: Optional[str] = None,
        prefer_existing_runner_session: bool = False,
    ) -> Dict[str, Any]:
        payload: Dict[str, Any] = {
            "runner_kind": runner_kind,
            "chat_id": chat_id,
            "thread_id": int(thread_id),
        }
        if message_anchor_id is not None:
            payload["message_anchor_id"] = int(message_anchor_id)
        if title:
            payload["title"] = title
        if bind_session_id:
            payload["bind_session_id"] = bind_session_id.strip()
        if bind_runner_session_key:
            payload["bind_runner_session_key"] = bind_runner_session_key.strip()
        if prefer_existing_runner_session:
            payload["prefer_existing_runner_session"] = True
        response = self._request_json("POST", "/api/sessions/telegram/resolve", body=payload)
        if not isinstance(response, dict):
            raise BridgeError(f"unexpected telegram session payload: {response!r}")
        session = response.get("session")
        if not isinstance(session, dict):
            raise BridgeError(f"unexpected telegram session object: {response!r}")
        return response

    def run_command(self, cmd: Sequence[str]) -> Dict[str, Any]:
        command = [str(token).strip() for token in cmd if str(token).strip()]
        if not command:
            raise BridgeError("runner command must not be empty")
        payload = self._request_json("POST", "/api/run/command", body={"cmd": command})
        if not isinstance(payload, dict):
            raise BridgeError(f"unexpected /api/run/command payload: {payload!r}")
        return payload

    def approval_get(self, approval_id: str) -> Dict[str, Any]:
        safe = quote(approval_id.strip(), safe="")
        if not safe:
            raise BridgeError("approval_id must not be empty")
        payload = self._request_json("GET", f"/api/approvals/{safe}")
        if not isinstance(payload, dict):
            raise BridgeError(f"unexpected /api/approvals/{safe} payload: {payload!r}")
        return payload

    def has_pending_approvals(self) -> bool:
        payload = self._request_json("GET", "/api/approvals")
        if not isinstance(payload, list):
            return False
        for item in payload:
            if not isinstance(item, dict):
                continue
            if optional_text(item.get("status")).lower() == "pending":
                return True
        return False

    def update_session_runner_key(
        self,
        *,
        session_id: str,
        runner_session_key: str,
    ) -> Dict[str, Any]:
        safe_session_id = quote(session_id.strip(), safe="")
        if not safe_session_id:
            raise BridgeError("session_id must not be empty")
        session_key = runner_session_key.strip()
        if not session_key:
            raise BridgeError("runner_session_key must not be empty")
        payload = self._request_json(
            "POST",
            f"/api/sessions/{safe_session_id}/runner-session-key",
            body={"runner_session_key": session_key},
        )
        if not isinstance(payload, dict):
            raise BridgeError(f"unexpected session runner key payload: {payload!r}")
        return payload

    def append_ui_log(
        self,
        *,
        level: str,
        source: str,
        message: str,
        details: Optional[Dict[str, Any]] = None,
    ) -> None:
        payload: Dict[str, Any] = {
            "level": str(level or "info").strip() or "info",
            "source": str(source or "telegram-bridge").strip() or "telegram-bridge",
            "message": str(message or "").strip(),
        }
        if not payload["message"]:
            return
        if isinstance(details, dict):
            payload["details"] = details
        self._request_json("POST", "/api/ui/logs/event", body=payload)

    def _request_json(self, method: str, path: str, body: Optional[Dict[str, Any]] = None) -> Any:
        url = f"{self.base_url}{path}"
        data = None if body is None else json.dumps(body).encode("utf-8")
        request = Request(
            url=url,
            data=data,
            method=method,
            headers={"Content-Type": "application/json"},
        )
        try:
            with urlopen(request, timeout=self.timeout_seconds) as response:
                raw = response.read().decode("utf-8", errors="replace")
        except HTTPError as err:
            raw = err.read().decode("utf-8", errors="replace") if err.fp is not None else ""
            raise BridgeError(f"{method} {path} failed ({err.code}): {raw or err.reason}") from err
        except URLError as err:
            raise BridgeError(f"{method} {path} failed: {err}") from err

        if not raw.strip():
            return {}
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            return {"raw": raw}


class TelegramClient:
    def __init__(self, bot_token: str, timeout_seconds: float = DEFAULT_HTTP_TIMEOUT_SECONDS):
        token = bot_token.strip()
        if not token:
            raise BridgeError("telegram bot token is required")
        self.bot_token = token
        self.base_url = f"https://api.telegram.org/bot{token}"
        self.timeout_seconds = timeout_seconds

    def send_text(
        self,
        *,
        chat_id: str,
        message: str,
        inline_buttons: Optional[List[List[Dict[str, str]]]] = None,
        message_thread_id: Optional[int] = None,
        reply_to_message_id: Optional[int] = None,
    ) -> Dict[str, Any]:
        payload: Dict[str, Any] = {
            "chat_id": chat_id,
            "text": message,
            "disable_web_page_preview": True,
        }
        if message_thread_id is not None:
            payload["message_thread_id"] = int(message_thread_id)
        if reply_to_message_id is not None:
            payload["reply_to_message_id"] = int(reply_to_message_id)
        if inline_buttons:
            payload["reply_markup"] = {"inline_keyboard": inline_buttons}
        return self._request_json("sendMessage", payload)

    def create_forum_topic(self, *, chat_id: str, name: str) -> Dict[str, Any]:
        payload = {
            "chat_id": chat_id,
            "name": name.strip() or "Agent Ruler",
        }
        return self._request_json("createForumTopic", payload)

    def edit_text(
        self,
        *,
        chat_id: str,
        message_id: int,
        message: str,
    ) -> Dict[str, Any]:
        payload = {
            "chat_id": chat_id,
            "message_id": int(message_id),
            "text": message,
            "disable_web_page_preview": True,
        }
        return self._request_json("editMessageText", payload)

    def send_typing(
        self,
        *,
        chat_id: str,
        message_thread_id: Optional[int] = None,
    ) -> Dict[str, Any]:
        payload: Dict[str, Any] = {
            "chat_id": chat_id,
            "action": "typing",
        }
        if message_thread_id is not None:
            payload["message_thread_id"] = int(message_thread_id)
        return self._request_json("sendChatAction", payload)

    def get_updates(self, *, offset: int, timeout_seconds: int) -> List[Dict[str, Any]]:
        payload = self._request_json(
            "getUpdates",
            {
                "offset": offset,
                "timeout": max(0, min(timeout_seconds, 30)),
                "allowed_updates": ["message", "callback_query"],
            },
        )
        if not isinstance(payload, dict):
            raise BridgeError(f"unexpected getUpdates payload: {payload!r}")
        result = payload.get("result")
        if not isinstance(result, list):
            return []
        return [item for item in result if isinstance(item, dict)]

    def answer_callback(self, callback_id: str, text: str) -> None:
        try:
            self._request_json(
                "answerCallbackQuery",
                {"callback_query_id": callback_id, "text": text[:180]},
            )
        except BridgeError as err:
            log_info(f"answerCallbackQuery failed: {err}")

    def get_file(self, file_id: str) -> Dict[str, Any]:
        payload = self._request_json("getFile", {"file_id": file_id.strip()})
        result = payload.get("result") if isinstance(payload, dict) else None
        if not isinstance(result, dict):
            raise BridgeError(f"telegram getFile returned malformed payload: {payload!r}")
        return result

    def download_file(self, file_path: str) -> bytes:
        safe_path = quote(file_path.strip(), safe="/")
        if not safe_path:
            raise BridgeError("telegram file path is empty")
        url = f"https://api.telegram.org/file/bot{self.base_url.split('/bot', 1)[1]}/{safe_path}"
        request = Request(url=url, method="GET")
        try:
            with urlopen(request, timeout=self.timeout_seconds) as response:
                return response.read()
        except HTTPError as err:
            detail = err.read().decode("utf-8", errors="replace") if err.fp is not None else ""
            raise BridgeError(
                f"telegram file download failed ({err.code}): {detail or err.reason}"
            ) from err
        except URLError as err:
            if shutil.which("curl"):
                return self._download_file_with_curl(safe_path, err)
            raise BridgeError(f"telegram file download failed: {err}") from err

    def _request_json(self, method: str, payload: Dict[str, Any]) -> Any:
        url = f"{self.base_url}/{method}"
        data = json.dumps(payload).encode("utf-8")
        request = Request(
            url=url,
            data=data,
            method="POST",
            headers={"Content-Type": "application/json"},
        )
        try:
            with urlopen(request, timeout=self.timeout_seconds) as response:
                raw = response.read().decode("utf-8", errors="replace")
        except HTTPError as err:
            detail = err.read().decode("utf-8", errors="replace") if err.fp is not None else ""
            raise BridgeError(f"telegram {method} failed ({err.code}): {detail or err.reason}") from err
        except URLError as err:
            if shutil.which("curl"):
                return self._request_json_with_curl(method, payload, err)
            raise BridgeError(f"telegram {method} failed: {err}") from err

        if not raw.strip():
            raise BridgeError(f"telegram {method} returned empty response")
        return self._parse_telegram_json(raw, method)

    def _request_json_with_curl(
        self,
        method: str,
        payload: Dict[str, Any],
        original_error: URLError,
    ) -> Any:
        env = os.environ.copy()
        env["AGENT_RULER_TELEGRAM_BOT_TOKEN"] = self.bot_token
        env["AGENT_RULER_TELEGRAM_METHOD"] = method
        env["AGENT_RULER_TELEGRAM_TIMEOUT"] = str(max(5, int(self.timeout_seconds) + 5))
        command = (
            'curl -sS --max-time "$AGENT_RULER_TELEGRAM_TIMEOUT" '
            '-H "Content-Type: application/json" '
            '-X POST '
            '--data-binary @- '
            '"https://api.telegram.org/bot$AGENT_RULER_TELEGRAM_BOT_TOKEN/$AGENT_RULER_TELEGRAM_METHOD"'
        )
        result = subprocess.run(
            ["sh", "-c", command],
            input=json.dumps(payload).encode("utf-8"),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            check=False,
        )
        if result.returncode != 0:
            detail = result.stderr.decode("utf-8", errors="replace").strip()
            raise BridgeError(
                f"telegram {method} failed: {detail or original_error}"
            ) from original_error
        raw = result.stdout.decode("utf-8", errors="replace")
        if not raw.strip():
            raise BridgeError(f"telegram {method} returned empty response")
        return self._parse_telegram_json(raw, method)

    def _download_file_with_curl(self, safe_path: str, original_error: URLError) -> bytes:
        env = os.environ.copy()
        env["AGENT_RULER_TELEGRAM_BOT_TOKEN"] = self.bot_token
        env["AGENT_RULER_TELEGRAM_FILE_PATH"] = safe_path
        env["AGENT_RULER_TELEGRAM_TIMEOUT"] = str(max(5, int(self.timeout_seconds) + 5))
        command = (
            'curl -sS --max-time "$AGENT_RULER_TELEGRAM_TIMEOUT" -L '
            '"https://api.telegram.org/file/bot$AGENT_RULER_TELEGRAM_BOT_TOKEN/$AGENT_RULER_TELEGRAM_FILE_PATH"'
        )
        result = subprocess.run(
            ["sh", "-c", command],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            check=False,
        )
        if result.returncode != 0:
            detail = result.stderr.decode("utf-8", errors="replace").strip()
            raise BridgeError(
                f"telegram file download failed: {detail or original_error}"
            ) from original_error
        return result.stdout

    def _parse_telegram_json(self, raw: str, method: str) -> Any:
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError as err:
            raise BridgeError(f"telegram {method} returned invalid JSON") from err
        if not isinstance(parsed, dict):
            raise BridgeError(f"telegram {method} returned malformed payload")
        if not parsed.get("ok", False):
            raise BridgeError(f"telegram {method} error: {parsed}")
        return parsed


class StateStore:
    def __init__(self, path: Path):
        self.path = path

    def load(self) -> Dict[str, Any]:
        if not self.path.exists():
            return {
                "seen_approvals": [],
                "pending": [],
                "last_update_id": 0,
                "chat_thread_bindings": {},
            }
        raw = self.path.read_text(encoding="utf-8")
        if not raw.strip():
            return {
                "seen_approvals": [],
                "pending": [],
                "last_update_id": 0,
                "chat_thread_bindings": {},
            }
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            return {
                "seen_approvals": [],
                "pending": [],
                "last_update_id": 0,
                "chat_thread_bindings": {},
            }
        if not isinstance(parsed, dict):
            return {
                "seen_approvals": [],
                "pending": [],
                "last_update_id": 0,
                "chat_thread_bindings": {},
            }
        return {
            "seen_approvals": list(parsed.get("seen_approvals", [])),
            "pending": list(parsed.get("pending", [])),
            "last_update_id": int(parsed.get("last_update_id", 0) or 0),
            "chat_thread_bindings": parsed.get("chat_thread_bindings", {}),
        }

    def save(
        self,
        *,
        seen_approvals: Iterable[str],
        pending: Iterable[PendingApproval],
        last_update_id: int,
        chat_thread_bindings: Dict[str, int],
    ) -> None:
        payload = {
            "seen_approvals": sorted(set(seen_approvals)),
            "pending": [
                {
                    "approval_id": item.approval_id,
                    "short_id": item.short_id,
                    "created_at": item.created_at,
                    "notified": bool(item.notified),
                }
                for item in sorted(pending, key=lambda row: (row.created_at, row.approval_id))
            ],
            "last_update_id": max(0, int(last_update_id)),
            "chat_thread_bindings": {
                str(chat_id): int(thread_id)
                for chat_id, thread_id in (chat_thread_bindings or {}).items()
                if str(chat_id).strip() and isinstance(thread_id, int) and thread_id > 0
            },
        }
        self.path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.path.with_suffix(".tmp")
        tmp.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        tmp.replace(self.path)


class BridgeInstanceLock:
    def __init__(self, path: Path):
        self.path = path
        self._handle: Optional[Any] = None

    def acquire(self, *, runner_kind: str) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        handle = self.path.open("a+", encoding="utf-8")
        try:
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as err:
            handle.seek(0)
            owner = handle.read().strip()
            handle.close()
            detail = f" lock owner: {owner}" if owner else ""
            raise BridgeError(
                "another Agent Ruler Telegram bridge already owns this bot token."
                " Use one active bridge per bot token or configure a different Telegram bot."
                f"{detail}"
            ) from err
        handle.seek(0)
        handle.truncate()
        handle.write(f"pid={os.getpid()} runner={runner_kind} started_at={int(time.time())}\n")
        handle.flush()
        self._handle = handle

    def release(self) -> None:
        if self._handle is None:
            return
        try:
            self._handle.seek(0)
            self._handle.truncate()
            fcntl.flock(self._handle.fileno(), fcntl.LOCK_UN)
        finally:
            self._handle.close()
            self._handle = None


class TelegramBridgeRuntime:
    def __init__(
        self,
        config: BridgeConfig,
        ruler: AgentRulerClient,
        telegram: TelegramClient,
    ):
        self.config = config
        self.ruler = ruler
        self.telegram = telegram
        self.state_store = StateStore(config.state_file)
        self._state_lock = threading.RLock()
        self._conversation_queues: Dict[str, "queue.Queue[Optional[ConversationTask]]"] = {}
        self._conversation_workers: Dict[str, threading.Thread] = {}
        state = self.state_store.load()
        self.seen_approvals = set(str(item) for item in state.get("seen_approvals", []))
        self.last_update_id = int(state.get("last_update_id", 0) or 0)
        self.chat_thread_bindings: Dict[str, int] = {}
        raw_bindings = state.get("chat_thread_bindings", {})
        if isinstance(raw_bindings, dict):
            for chat_id, thread_id in raw_bindings.items():
                key = str(chat_id).strip()
                if not key:
                    continue
                if isinstance(thread_id, int) and thread_id > 0:
                    self.chat_thread_bindings[key] = thread_id
        self.session_runner_keys: Dict[str, str] = {}
        self.pending_by_approval: Dict[str, PendingApproval] = {}
        self.pending_by_short: Dict[str, PendingApproval] = {}
        for item in state.get("pending", []):
            if not isinstance(item, dict):
                continue
            approval_id = str(item.get("approval_id", "")).strip()
            short_id = normalize_short_id(str(item.get("short_id", "")).strip())
            created_at = int(item.get("created_at", 0) or 0)
            if not approval_id or not short_id or created_at <= 0:
                continue
            pending = PendingApproval(
                approval_id=approval_id,
                short_id=short_id,
                created_at=created_at,
                notified=bool(item.get("notified", True)),
            )
            if self._is_expired(pending):
                continue
            self.pending_by_approval[pending.approval_id] = pending
            self.pending_by_short[pending.short_id] = pending

    def runner_label(self) -> str:
        return RUNNER_LABELS.get(self.config.runner_kind, self.config.runner_kind or "Unknown")

    def _record_ui_log(
        self,
        *,
        level: str,
        message: str,
        details: Optional[Dict[str, Any]] = None,
    ) -> None:
        try:
            payload = dict(details or {})
            payload.setdefault("runner_id", self.config.runner_kind)
            self.ruler.append_ui_log(
                level=level,
                source=f"telegram-bridge-{self.config.runner_kind}",
                message=message,
                details=payload,
            )
        except Exception:
            # UI logs are best-effort telemetry and must never block bridge flow.
            return

    def save_state(self) -> None:
        with self._state_lock:
            chat_thread_bindings = dict(self.chat_thread_bindings)
            pending = list(self.pending_by_approval.values())
            seen_approvals = set(self.seen_approvals)
        self.state_store.save(
            seen_approvals=seen_approvals,
            pending=pending,
            last_update_id=self.last_update_id,
            chat_thread_bindings=chat_thread_bindings,
        )

    def tick(self) -> None:
        errors: List[Exception] = []
        try:
            self._prune_expired_pending()
            if self.config.enabled:
                try:
                    self._poll_pending_approvals()
                except Exception as err:  # pragma: no cover - exercised in integration flow
                    errors.append(err)
            try:
                self._poll_updates(timeout_seconds=max(1, min(self.config.poll_interval_seconds, 10)))
            except Exception as err:
                errors.append(err)
        finally:
            self.save_state()
        if errors:
            detail = " | ".join(str(err) for err in errors if str(err).strip())
            raise BridgeError(detail or str(errors[0]))

    def _poll_pending_approvals(self) -> None:
        events = self.ruler.status_feed()
        for event in events:
            approval_id = str(event.get("approval_id", "")).strip()
            verdict = str(event.get("verdict", "")).strip().lower()
            if not approval_id or verdict != "pending":
                continue

            pending = self.pending_by_approval.get(approval_id)
            if pending is None:
                if approval_id in self.seen_approvals:
                    continue
                pending = self._register_pending(approval_id)
                self.seen_approvals.add(approval_id)
                log_info(f"approval detected: approval_id={approval_id} short_id={pending.short_id}")

            if pending.notified:
                continue
            delivered = self._notify_pending(event, pending)
            if delivered > 0:
                pending.notified = True

    def _notify_pending(self, event: Dict[str, Any], pending: PendingApproval) -> int:
        approval_id = pending.approval_id
        reason = str(event.get("reason_code", "approval_required"))
        category = str(event.get("category", "approval_required"))
        runner_id = str(event.get("runner_id", "")).strip()
        if not runner_id:
            runner_id = self.runner_label()
        session_hint = str(event.get("session_hint", "")).strip()
        link = self._make_deep_link(str(event.get("open_in_webui", f"/approvals/{approval_id}")))
        reason_text = describe_approval_reason(reason, category)
        approval_view = self._fetch_approval_view(approval_id)
        why = optional_text(approval_view.get("why")) if isinstance(approval_view, dict) else ""
        if why:
            reason_text = why.replace(" | ", " - ")
        context_rows = self._approval_context_rows(event, approval_view)
        category_label = self._approval_category_label(category)

        lines = [
            "🚨 Approval required",
            "",
            f"Runner: {runner_id}",
            f"Short ID: {pending.short_id}",
            "",
            "Approval ID:",
            approval_id,
            "",
            "Reason:",
            reason_text,
            "",
            f"Category: {category_label}",
            "Status: ⏳ Waiting for your decision. The runner is paused and will auto-resume after approval.",
        ]
        if session_hint:
            lines.append(f"Session: {session_hint}")
        if context_rows:
            lines.append("")
            for label, value in context_rows:
                lines.append(f"{label}:")
                if isinstance(value, list):
                    lines.extend([f"- {item}" for item in value])
                else:
                    lines.append(str(value))
                lines.append("")
            if lines and not lines[-1]:
                lines.pop()
        lines.extend(
            [
                "",
                f"🔗 Control Panel: {link}",
                "",
                f"Reply with `approve {pending.short_id}` or `deny {pending.short_id}`",
            ]
        )
        message = "\n".join(lines)
        buttons = [
            [
                {"text": "✅ Approve", "callback_data": f"ar:approve:{pending.short_id}"},
                {"text": "🚫 Deny", "callback_data": f"ar:deny:{pending.short_id}"},
            ]
        ]

        delivered = 0
        for target in self._approval_delivery_targets():
            try:
                self.telegram.send_text(
                    chat_id=target.chat_id,
                    message=message,
                    inline_buttons=buttons,
                    message_thread_id=target.message_thread_id,
                )
                delivered += 1
            except BridgeError as err:
                log_info(
                    "message send failed: "
                    f"approval_id={approval_id} short_id={pending.short_id} chat={target.chat_id} reason={err}"
                )
        self._record_ui_log(
            level="info",
            message="Approval request forwarded to Telegram",
            details={
                "approval_id": approval_id,
                "short_id": pending.short_id,
                "category": category,
                "reason_code": reason,
                "delivery_count": delivered,
            },
        )
        return delivered

    def _fetch_approval_view(self, approval_id: str) -> Dict[str, Any]:
        try:
            payload = self.ruler.approval_get(approval_id)
        except BridgeError as err:
            log_info(f"unable to load approval {approval_id} details: {err}")
            return {}
        return payload if isinstance(payload, dict) else {}

    def _collect_approval_targets(
        self,
        event: Dict[str, Any],
        approval_view: Dict[str, Any],
    ) -> List[str]:
        targets: List[str] = []
        seen: set[str] = set()

        def add(value: Any) -> None:
            raw = optional_text(value)
            if not raw:
                return
            if raw in seen:
                return
            seen.add(raw)
            targets.append(raw)

        add(approval_view.get("resolved_src"))
        add(approval_view.get("resolved_dst"))
        action = approval_view.get("action") if isinstance(approval_view.get("action"), dict) else {}
        add(action.get("path"))
        add(action.get("secondary_path"))
        metadata = action.get("metadata") if isinstance(action.get("metadata"), dict) else {}
        for key in ("export_src", "export_dst", "import_src", "import_dst", "src", "dst", "target_path"):
            add(metadata.get(key))
        add(event.get("resolved_src"))
        add(event.get("resolved_dst"))

        return targets

    def _approval_context_rows(
        self,
        event: Dict[str, Any],
        approval_view: Dict[str, Any],
    ) -> List[Tuple[str, Any]]:
        action = approval_view.get("action") if isinstance(approval_view.get("action"), dict) else {}
        action_metadata = action.get("metadata") if isinstance(action.get("metadata"), dict) else {}
        event_metadata = event.get("metadata") if isinstance(event.get("metadata"), dict) else {}
        operation = optional_text(action.get("operation") or event.get("operation"))
        category = optional_text(event.get("category")).lower()
        transfer_like = (
            category in TRANSFER_APPROVAL_CATEGORIES or operation in TRANSFER_APPROVAL_OPERATIONS
        )

        source_path = self._first_path_value(
            [
                approval_view.get("resolved_src"),
                event.get("resolved_src"),
                action_metadata.get("export_src"),
                action_metadata.get("import_src"),
                action_metadata.get("src"),
                event_metadata.get("export_src"),
                event_metadata.get("import_src"),
                event_metadata.get("src"),
                action.get("secondary_path"),
                action.get("path"),
                event.get("path"),
                event.get("secondary_path"),
            ]
        )
        destination_path = self._first_path_value(
            [
                approval_view.get("resolved_dst"),
                event.get("resolved_dst"),
                action_metadata.get("export_dst"),
                action_metadata.get("import_dst"),
                action_metadata.get("dst"),
                action_metadata.get("target_path"),
                action_metadata.get("stage_ref"),
                event_metadata.get("export_dst"),
                event_metadata.get("import_dst"),
                event_metadata.get("dst"),
                event_metadata.get("target_path"),
                event_metadata.get("stage_ref"),
                action.get("path"),
                action.get("secondary_path"),
                event.get("secondary_path"),
                event.get("path"),
            ],
            exclude={source_path} if source_path else set(),
        )
        source_display = self._alias_runtime_path(source_path) if source_path else ""
        destination_display = self._alias_runtime_path(destination_path) if destination_path else ""

        targets = self._collect_approval_targets(event, approval_view)
        display_targets: List[str] = []
        seen_targets: set[str] = set()
        for target in targets:
            rendered = self._alias_runtime_path(target)
            if not rendered or rendered in seen_targets:
                continue
            seen_targets.add(rendered)
            display_targets.append(rendered)

        rows: List[Tuple[str, Any]] = []
        if transfer_like:
            if source_display:
                rows.append(("File involved", source_display))
            if destination_display:
                rows.append(("Destination", destination_display))
        elif source_display:
            rows.append(("File involved", source_display))

        used = {source_display, destination_display}
        extras = [item for item in display_targets if item not in used]
        if extras:
            preview = extras[:MAX_APPROVAL_TARGET_PREVIEW]
            remaining = len(extras) - len(preview)
            if remaining > 0:
                preview.append(f"… and {remaining} more")
            rows.append(("Context paths", preview))

        if operation:
            rows.append(("Operation", operation.replace("_", " ")))

        return rows

    def _first_path_value(self, values: Sequence[Any], exclude: Optional[set[str]] = None) -> str:
        blocked = exclude or set()
        for value in values:
            candidate = optional_text(value)
            if not candidate or candidate in blocked:
                continue
            return candidate
        return ""

    def _approval_category_label(self, category: str) -> str:
        raw = optional_text(category).lower() or "approval_required"
        pretty = humanize_label(raw)
        if pretty.lower() == raw.replace("_", " "):
            return pretty
        return f"{pretty} ({raw})"

    def _alias_runtime_path(self, raw_path: str) -> str:
        candidate = optional_text(raw_path)
        if not candidate:
            return ""
        try:
            path = Path(candidate)
        except Exception:
            return candidate
        runtime_dir = optional_text(self.config.runtime_dir)
        runtime_root = Path(runtime_dir).expanduser() if runtime_dir else None
        if path.is_absolute() and runtime_root is not None:
            workspace_root = runtime_root / "user_data" / "runners" / self.config.runner_kind / "workspace"
            shared_root = runtime_root / "shared-zone"
            rel_workspace = self._relative_if_under(path, workspace_root)
            if rel_workspace is not None:
                return self._compact_prefixed_path("workspace", rel_workspace)
            rel_shared = self._relative_if_under(path, shared_root)
            if rel_shared is not None:
                return self._compact_prefixed_path("shared-zone", rel_shared)
            rel_runtime = self._relative_if_under(path, runtime_root)
            if rel_runtime is not None:
                return self._compact_prefixed_path("runtime", rel_runtime)

        if path.is_absolute():
            home = Path.home()
            rel_home = self._relative_if_under(path, home)
            if rel_home is not None:
                return self._compact_prefixed_path("~", rel_home)
            return self._compact_absolute_path(path)
        return candidate

    def _relative_if_under(self, path: Path, root: Path) -> Optional[Path]:
        try:
            return path.relative_to(root)
        except ValueError:
            return None

    def _compact_prefixed_path(self, prefix: str, relative: Path) -> str:
        rel = relative.as_posix()
        if rel in {"", "."}:
            return prefix
        parts = [part for part in rel.split("/") if part]
        if len(parts) <= 4:
            return f"{prefix}/{rel}"
        return f"{prefix}/.../{'/'.join(parts[-3:])}"

    def _compact_absolute_path(self, path: Path) -> str:
        parts = [part for part in path.parts if part not in {path.anchor, "/"}]
        if len(parts) <= 4:
            return path.as_posix()
        return f".../{'/'.join(parts[-3:])}"

    def _approval_delivery_targets(self) -> List[ChatTarget]:
        targets: List[ChatTarget] = []
        seen: set[Tuple[str, Optional[int]]] = set()
        with self._state_lock:
            bindings = dict(self.chat_thread_bindings)
        for chat_id, thread_id in sorted(bindings.items()):
            key = (chat_id, thread_id)
            if key in seen:
                continue
            seen.add(key)
            targets.append(ChatTarget(chat_id=chat_id, message_thread_id=thread_id))
        return targets

    def _poll_updates(self, *, timeout_seconds: int) -> None:
        updates = self.telegram.get_updates(offset=self.last_update_id + 1, timeout_seconds=timeout_seconds)
        for update in updates:
            update_id = int(update.get("update_id", 0) or 0)
            if update_id > self.last_update_id:
                self.last_update_id = update_id
            self._handle_update(update)

    def _handle_update(self, update: Dict[str, Any]) -> None:
        callback = update.get("callback_query")
        message = update.get("message")
        if isinstance(callback, dict):
            self._handle_callback_query(callback)
            return
        if isinstance(message, dict):
            self._handle_message(message)

    def _handle_callback_query(self, callback: Dict[str, Any]) -> None:
        callback_id = str(callback.get("id", "")).strip()
        data = str(callback.get("data", "")).strip()
        user = callback.get("from") if isinstance(callback.get("from"), dict) else {}
        user_id = str(user.get("id", "")).strip()
        msg = callback.get("message") if isinstance(callback.get("message"), dict) else {}
        chat = msg.get("chat") if isinstance(msg.get("chat"), dict) else {}
        chat_id = str(chat.get("id", "")).strip()
        thread_id = msg.get("message_thread_id")
        if not isinstance(thread_id, int):
            thread_id = None

        parsed = parse_decision_command(data)
        if parsed is None:
            if callback_id:
                self.telegram.answer_callback(callback_id, "🚫 Ignored")
            return
        if not self.config.enabled:
            if callback_id:
                self.telegram.answer_callback(callback_id, "🚫 Bridge disabled")
            return
        self._handle_decision_command(
            parsed=parsed,
            user_id=user_id,
            chat_id=chat_id,
            thread_id=thread_id,
            callback_id=callback_id,
            reply_to_message_id=msg.get("message_id") if isinstance(msg.get("message_id"), int) else None,
        )

    def _handle_message(self, message: Dict[str, Any]) -> None:
        text = self._extract_message_text(message)
        if not text and not self._message_has_supported_attachments(message):
            return
        user = message.get("from") if isinstance(message.get("from"), dict) else {}
        user_id = str(user.get("id", "")).strip()
        chat = message.get("chat") if isinstance(message.get("chat"), dict) else {}
        chat_id = str(chat.get("id", "")).strip()
        thread_id = message.get("message_thread_id")
        if not isinstance(thread_id, int):
            thread_id = None
        message_id = message.get("message_id")
        if not isinstance(message_id, int):
            message_id = None
        command = normalize_text_command(text)
        if command == "/whoami":
            self._safe_reply(
                chat_id,
                thread_id,
                self._whoami_message(user_id),
                reply_to_message_id=message_id,
            )
            return
        if not self.config.enabled:
            self._safe_reply(
                chat_id,
                thread_id,
                self._disabled_mode_message(),
                reply_to_message_id=message_id,
            )
            return

        parsed = parse_decision_command(text)
        if parsed is not None:
            self._handle_decision_command(
                parsed=parsed,
                user_id=user_id,
                chat_id=chat_id,
                thread_id=thread_id,
                callback_id="",
                reply_to_message_id=message_id,
            )
            return

        self._handle_threaded_message(
            text=text,
            message=message,
            user_id=user_id,
            chat_id=chat_id,
            thread_id=thread_id,
            message_id=message_id,
        )

    def _handle_threaded_message(
        self,
        *,
        text: str,
        message: Dict[str, Any],
        user_id: str,
        chat_id: str,
        thread_id: Optional[int],
        message_id: Optional[int],
    ) -> None:
        if not self._chat_allowed(chat_id, thread_id):
            return
        if not self._sender_allowed(user_id):
            self._safe_reply(
                chat_id,
                thread_id,
                "🚫 Message ignored : Sender is not allowed.",
                reply_to_message_id=message_id,
            )
            return

        command = normalize_text_command(text)
        command_arg = extract_command_argument(text)
        bind_session_id: Optional[str] = None
        bind_runner_session_key: Optional[str] = None
        prefer_existing_runner_session = False
        force_new_topic = command in {"/new", "/continue", "/link"}
        topic_seed = text or "Telegram attachment"
        if command == "/new" and command_arg:
            topic_seed = command_arg
        if command in {"/continue", "/link"}:
            if command_arg:
                if _SESSION_ID_PATTERN.match(command_arg):
                    bind_session_id = command_arg
                else:
                    bind_runner_session_key = command_arg
            else:
                prefer_existing_runner_session = True
        elif command in {"/status", "/help", "/start"}:
            prefer_existing_runner_session = True
        elif not command:
            prefer_existing_runner_session = True

        reply_anchor = message_id
        if thread_id is None:
            if not force_new_topic:
                with self._state_lock:
                    thread_id = self.chat_thread_bindings.get(chat_id)
            if not isinstance(thread_id, int) or thread_id <= 0:
                thread_id = self._bootstrap_thread_for_private_chat(chat_id, topic_seed)
            if thread_id is None:
                self._safe_reply(
                    chat_id,
                    None,
                    self._threadless_mode_help(),
                    reply_to_message_id=message_id,
                )
                return
            # New topic has no inbound message in-thread yet, so route by thread id.
            reply_anchor = None
        with self._state_lock:
            self.chat_thread_bindings[chat_id] = thread_id

        try:
            resolved = self.ruler.resolve_telegram_session(
                runner_kind=self.config.runner_kind,
                chat_id=chat_id,
                thread_id=thread_id,
                message_anchor_id=message_id,
                title=self._derive_session_title(text),
                bind_session_id=bind_session_id,
                bind_runner_session_key=bind_runner_session_key,
                prefer_existing_runner_session=prefer_existing_runner_session,
            )
        except BridgeError as err:
            self._safe_reply(
                chat_id,
                thread_id,
                    f"🚫 Session routing failed : Runner {self.runner_label()}. {err}",
                reply_to_message_id=reply_anchor,
            )
            return

        session = resolved.get("session") if isinstance(resolved.get("session"), dict) else {}
        created = bool(resolved.get("created", False))
        if command in {"/help", "/start"}:
            self._safe_reply(
                chat_id,
                thread_id,
                self._help_message(session),
                reply_to_message_id=reply_anchor,
            )
            return
        if command == "/status":
            self._safe_reply(
                chat_id,
                thread_id,
                self._status_message(session, created=False),
                reply_to_message_id=reply_anchor,
            )
            return
        if command in {"/continue", "/link"}:
            self._safe_reply(
                chat_id,
                thread_id,
                self._status_message(
                    session,
                    created=created,
                    heading="Session linked",
                ),
                reply_to_message_id=reply_anchor,
            )
            return
        if not command:
            try:
                attachments = self._stage_message_attachments(
                    message,
                    chat_id=chat_id,
                    thread_id=thread_id,
                    message_id=message_id,
                )
            except BridgeError as err:
                self._safe_reply(
                    chat_id,
                    thread_id,
                    f"🚫 Attachment relay failed\n\n{err}",
                    reply_to_message_id=reply_anchor,
                )
                return
            self._handle_plain_text_message(
                text=text,
                attachments=attachments,
                session=session,
                chat_id=chat_id,
                thread_id=thread_id,
                reply_anchor=None,
            )
            return
        if created:
            self._safe_reply(
                chat_id,
                thread_id,
                self._status_message(session, created=True),
                reply_to_message_id=reply_anchor,
            )

    def _handle_plain_text_message(
        self,
        *,
        text: str,
        attachments: Sequence[TelegramAttachment],
        session: Dict[str, Any],
        chat_id: str,
        thread_id: Optional[int],
        reply_anchor: Optional[int],
    ) -> None:
        self._enqueue_conversation_task(
            ConversationTask(
                text=text,
                attachments=list(attachments),
                session=dict(session),
                chat_id=chat_id,
                thread_id=thread_id,
                reply_anchor=reply_anchor,
            )
        )

    def _enqueue_conversation_task(self, task: ConversationTask) -> None:
        slot = self._conversation_slot(task.chat_id, task.thread_id)
        with self._state_lock:
            work_queue = self._conversation_queues.get(slot)
            worker = self._conversation_workers.get(slot)
            if work_queue is None or worker is None or not worker.is_alive():
                work_queue = queue.Queue()
                self._conversation_queues[slot] = work_queue
                worker = threading.Thread(
                    target=self._conversation_worker_loop,
                    args=(slot, work_queue),
                    daemon=True,
                )
                self._conversation_workers[slot] = worker
                worker.start()
        work_queue.put(task)

    def _conversation_slot(self, chat_id: str, thread_id: Optional[int]) -> str:
        thread_token = str(thread_id) if isinstance(thread_id, int) else "root"
        return f"{chat_id}::{thread_token}"

    def _conversation_worker_loop(
        self,
        slot: str,
        work_queue: "queue.Queue[Optional[ConversationTask]]",
    ) -> None:
        try:
            while True:
                try:
                    task = work_queue.get(timeout=300)
                except queue.Empty:
                    return
                if task is None:
                    return
                try:
                    self._process_conversation_task(task)
                finally:
                    work_queue.task_done()
        finally:
            with self._state_lock:
                current = self._conversation_queues.get(slot)
                if current is work_queue:
                    self._conversation_queues.pop(slot, None)
                    self._conversation_workers.pop(slot, None)

    def _process_conversation_task(self, task: ConversationTask) -> None:
        typing_stop = threading.Event()
        typing_thread = threading.Thread(
            target=self._typing_keepalive_loop,
            args=(task.chat_id, task.thread_id, typing_stop),
            daemon=True,
        )
        typing_thread.start()
        stream_state = StreamReplyState(enabled=self.config.answer_streaming_enabled)
        progress_thread = threading.Thread(
            target=self._delayed_progress_notice,
            args=(task.chat_id, task.thread_id, task.reply_anchor, typing_stop, stream_state),
            daemon=True,
        )
        progress_thread.start()
        self._record_ui_log(
            level="info",
            message="Runner request dispatched from Telegram thread",
            details={
                "chat_id": task.chat_id,
                "thread_id": task.thread_id,
                "session_id": optional_text(task.session.get("id")),
                "has_attachments": bool(task.attachments),
            },
        )

        try:
            reply = self._dispatch_runner_message(
                task.text,
                task.attachments,
                task.session,
                on_partial_text=(
                    lambda partial: self._update_stream_reply(
                        task.chat_id,
                        task.thread_id,
                        task.reply_anchor,
                        stream_state,
                        partial,
                    )
                )
                if self.config.answer_streaming_enabled
                else None,
                on_stream_activity=(
                    lambda: self._mark_stream_activity(stream_state)
                    if self.config.answer_streaming_enabled
                    else None
                ),
            )
        except BridgeError as err:
            self._finalize_stream_reply(
                task.chat_id,
                task.thread_id,
                task.reply_anchor,
                stream_state,
                f"🚫 Runner request failed : {err}",
            )
            typing_stop.set()
            typing_thread.join(timeout=1)
            progress_thread.join(timeout=1)
            self._record_ui_log(
                level="error",
                message="Runner request failed in Telegram bridge",
                details={
                    "chat_id": task.chat_id,
                    "thread_id": task.thread_id,
                    "session_id": optional_text(task.session.get("id")),
                    "error": str(err),
                },
            )
            return

        self._finalize_stream_reply(
            task.chat_id,
            task.thread_id,
            task.reply_anchor,
            stream_state,
            reply,
        )
        typing_stop.set()
        typing_thread.join(timeout=1)
        progress_thread.join(timeout=1)
        self._record_ui_log(
            level="info",
            message="Runner request completed in Telegram bridge",
            details={
                "chat_id": task.chat_id,
                "thread_id": task.thread_id,
                "session_id": optional_text(task.session.get("id")),
                "reply_chars": len(optional_text(reply)),
            },
        )

    def _delayed_progress_notice(
        self,
        chat_id: str,
        thread_id: Optional[int],
        reply_anchor: Optional[int],
        stop_event: threading.Event,
        stream_state: StreamReplyState,
    ) -> None:
        if stop_event.wait(DEFAULT_PROGRESS_NOTICE_DELAY_SECONDS):
            return
        if (
            stream_state.message_id is not None
            or stream_state.last_sent_text
            or stream_state.saw_stream_activity
        ):
            return
        if not self._should_emit_progress_notice():
            return
        self._safe_reply(
            chat_id,
            thread_id,
            "⏳ Working on it. I will continue automatically once approvals are resolved.",
            reply_to_message_id=reply_anchor,
        )

    def _should_emit_progress_notice(self) -> bool:
        checker = getattr(self.ruler, "has_pending_approvals", None)
        if not callable(checker):
            return False
        try:
            return bool(checker())
        except BridgeError:
            return False

    def _mark_stream_activity(self, stream_state: StreamReplyState) -> None:
        stream_state.saw_stream_activity = True

    def _typing_keepalive_loop(
        self,
        chat_id: str,
        thread_id: Optional[int],
        stop_event: threading.Event,
    ) -> None:
        while not stop_event.is_set():
            try:
                self.telegram.send_typing(chat_id=chat_id, message_thread_id=thread_id)
            except BridgeError as err:
                log_info(f"typing keepalive failed for chat {chat_id}: {err}")
                return
            if stop_event.wait(DEFAULT_TYPING_KEEPALIVE_SECONDS):
                return

    def _update_stream_reply(
        self,
        chat_id: str,
        thread_id: Optional[int],
        reply_anchor: Optional[int],
        stream_state: StreamReplyState,
        text: str,
    ) -> None:
        if not stream_state.enabled:
            return
        cleaned = self._truncate_telegram_message(text)
        if not cleaned:
            return
        previous = stream_state.last_sent_text
        if cleaned == previous:
            return
        if previous and self._stream_text_equivalent(previous, cleaned):
            stream_state.last_sent_text = cleaned
            return

        message_text = cleaned
        if previous and cleaned.startswith(previous):
            # Stream callbacks often deliver cumulative text snapshots. Emit only
            # the new suffix so Telegram bubbles stay traceable and non-duplicative.
            message_text = cleaned[len(previous) :].strip()
            if not message_text:
                stream_state.last_sent_text = cleaned
                return

        now = time.monotonic()
        if (
            stream_state.message_id is not None
            and now - stream_state.last_update_at < DEFAULT_STREAM_EDIT_INTERVAL_SECONDS
        ):
            return
        if (
            stream_state.last_emitted_message
            and self._stream_text_equivalent(stream_state.last_emitted_message, message_text)
        ):
            stream_state.last_sent_text = cleaned
            stream_state.last_update_at = now
            return
        try:
            # Keep per-message traceability deterministic: emit a new Telegram
            # bubble for each stream snapshot instead of rewriting prior text.
            sent = self._send_text_message(
                chat_id,
                thread_id,
                message_text,
                reply_to_message_id=reply_anchor,
            )
            stream_state.message_id = self._extract_telegram_message_id(sent)
        except BridgeError as err:
            stream_state.enabled = False
            log_info(f"stream reply update failed for chat {chat_id}: {err}")
            return
        stream_state.last_sent_text = cleaned
        stream_state.last_emitted_message = message_text
        stream_state.last_update_at = now

    def _finalize_stream_reply(
        self,
        chat_id: str,
        thread_id: Optional[int],
        reply_anchor: Optional[int],
        stream_state: StreamReplyState,
        text: str,
    ) -> None:
        cleaned = self._truncate_telegram_message(text)
        if stream_state.enabled and stream_state.message_id is not None:
            # Keep traceability deterministic for approval-resume flows. If the
            # final output is cumulative, emit only the remaining suffix.
            final_text = cleaned
            previous = stream_state.last_sent_text
            if previous and self._stream_text_equivalent(previous, cleaned):
                return
            if previous and cleaned.startswith(previous):
                final_text = cleaned[len(previous) :].strip()
            if (
                final_text
                and stream_state.last_emitted_message
                and self._stream_text_equivalent(stream_state.last_emitted_message, final_text)
            ):
                return
            if final_text:
                self._safe_reply(
                    chat_id,
                    thread_id,
                    final_text,
                    reply_to_message_id=reply_anchor,
                )
                stream_state.last_emitted_message = final_text
            return
        self._safe_reply(
            chat_id,
            thread_id,
            cleaned,
            reply_to_message_id=reply_anchor,
        )

    def _stream_text_equivalent(self, previous: str, current: str) -> bool:
        left = re.sub(r"\s+", " ", str(previous or "")).strip()
        right = re.sub(r"\s+", " ", str(current or "")).strip()
        return bool(left and left == right)

    def _send_text_message(
        self,
        chat_id: str,
        thread_id: Optional[int],
        message: str,
        *,
        reply_to_message_id: Optional[int] = None,
    ) -> Dict[str, Any]:
        try:
            return self.telegram.send_text(
                chat_id=chat_id,
                message=message,
                message_thread_id=thread_id,
                reply_to_message_id=reply_to_message_id,
            )
        except BridgeError as err:
            if (
                thread_id is not None
                and reply_to_message_id is not None
                and is_thread_send_rejection(str(err))
            ):
                return self.telegram.send_text(
                    chat_id=chat_id,
                    message=message,
                    message_thread_id=None,
                    reply_to_message_id=reply_to_message_id,
                )
            raise

    def _extract_telegram_message_id(self, payload: Dict[str, Any]) -> Optional[int]:
        if not isinstance(payload, dict):
            return None
        result = payload.get("result") if isinstance(payload.get("result"), dict) else payload
        message_id = result.get("message_id") if isinstance(result, dict) else None
        if isinstance(message_id, int) and message_id > 0:
            return message_id
        return None

    def _dispatch_runner_message(
        self,
        prompt: str,
        attachments: Sequence[TelegramAttachment],
        session: Dict[str, Any],
        *,
        on_partial_text: Optional[Callable[[str], None]] = None,
        on_stream_activity: Optional[Callable[[], None]] = None,
    ) -> str:
        runner = self.config.runner_kind
        if runner == "claudecode":
            return self._run_claudecode_prompt(
                prompt,
                attachments,
                session,
                on_partial_text=on_partial_text,
                on_stream_activity=on_stream_activity,
            )
        if runner == "opencode":
            return self._run_opencode_prompt(
                prompt,
                attachments,
                session,
                on_partial_text=on_partial_text,
                on_stream_activity=on_stream_activity,
            )
        raise BridgeError(
            f"runner `{runner}` does not support conversational relay in this bridge"
        )

    def _run_claudecode_prompt(
        self,
        prompt: str,
        attachments: Sequence[TelegramAttachment],
        session: Dict[str, Any],
        *,
        on_partial_text: Optional[Callable[[str], None]] = None,
        on_stream_activity: Optional[Callable[[], None]] = None,
    ) -> str:
        output_format = "stream-json" if on_partial_text is not None else "json"
        cmd = ["claude", "-p", "--output-format", output_format]
        if output_format == "stream-json":
            cmd.append("--verbose")
        runner_session_key = self._resolve_runner_session_key(session)
        if runner_session_key:
            cmd.extend(["-r", runner_session_key])
        cmd.append(self._build_runner_prompt(prompt, attachments))
        result = self._execute_runner_command(
            cmd,
            on_event=self._make_claude_stream_callback(
                session,
                on_partial_text,
                on_stream_activity,
            ),
        )
        if int(result.get("exit_code", 0) or 0) != 0:
            raise BridgeError(self._nonzero_command_error(result))
        raw = self._extract_command_payload_text(result)
        return self._parse_claudecode_reply(raw, session)

    def _run_opencode_prompt(
        self,
        prompt: str,
        attachments: Sequence[TelegramAttachment],
        session: Dict[str, Any],
        *,
        on_partial_text: Optional[Callable[[str], None]] = None,
        on_stream_activity: Optional[Callable[[], None]] = None,
    ) -> str:
        session_id = optional_text(session.get("id"))
        runner_session_key = self._resolve_runner_session_key(session)
        cmd = ["opencode", "run", "--format", "json"]
        if runner_session_key:
            cmd.extend(["--session", runner_session_key])
        cmd.append(self._build_runner_prompt(prompt, attachments))
        result = self._execute_runner_command(
            cmd,
            on_event=self._make_opencode_stream_callback(
                session,
                on_partial_text,
                on_stream_activity,
            ),
        )
        if int(result.get("exit_code", 0) or 0) != 0:
            raise BridgeError(self._nonzero_command_error(result))
        raw = self._extract_command_payload_text(result)
        _ = session_id
        return self._parse_opencode_reply(raw, session)

    def _execute_runner_command(
        self,
        cmd: Sequence[str],
        *,
        on_event: Optional[Callable[[str], None]] = None,
    ) -> Dict[str, Any]:
        if isinstance(self.ruler, AgentRulerClient):
            return self._execute_runner_cli_command(cmd, on_event=on_event)
        try:
            payload = self.ruler.run_command(cmd)
        except BridgeError as err:
            raise BridgeError(str(err)) from err
        if not isinstance(payload, dict):
            raise BridgeError("runner command returned unexpected payload")
        if on_event is not None:
            stdout = optional_text(payload.get("stdout"))
            if stdout:
                for line in stdout.splitlines():
                    on_event(line)
        return payload

    def _execute_runner_cli_command(
        self,
        cmd: Sequence[str],
        *,
        on_event: Optional[Callable[[str], None]] = None,
    ) -> Dict[str, Any]:
        agent_ruler_bin = os.environ.get("AGENT_RULER_BIN") or shutil.which("agent-ruler")
        if not agent_ruler_bin:
            raise BridgeError("agent-ruler binary not found in PATH")
        full_cmd = [
            agent_ruler_bin,
            "--runtime-dir",
            self.config.runtime_dir,
            "run",
            "--",
            *[str(token) for token in cmd],
        ]
        process = subprocess.Popen(
            full_cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        stdout_lines: List[str] = []
        stderr_lines: List[str] = []

        def read_stream(stream: Any, sink: List[str], event_callback: Optional[Callable[[str], None]]) -> None:
            try:
                for line in iter(stream.readline, ""):
                    sink.append(line)
                    if event_callback is not None:
                        event_callback(line.rstrip("\n"))
                remainder = stream.read()
                if remainder:
                    sink.append(remainder)
                    if event_callback is not None:
                        event_callback(remainder)
            finally:
                stream.close()

        stdout_thread = threading.Thread(
            target=read_stream,
            args=(process.stdout, stdout_lines, on_event),
            daemon=True,
        )
        stderr_thread = threading.Thread(
            target=read_stream,
            args=(process.stderr, stderr_lines, None),
            daemon=True,
        )
        stdout_thread.start()
        stderr_thread.start()
        return_code = process.wait()
        stdout_thread.join()
        stderr_thread.join()
        stdout = "".join(stdout_lines)
        stderr = "".join(stderr_lines)
        if return_code != 0 and not (stdout or stderr):
            raise BridgeError(f"agent-ruler run failed with exit code {return_code}")
        return {
            "status": "completed" if return_code == 0 else "failed",
            "exit_code": return_code,
            "stdout": stdout,
            "stderr": stderr,
        }

    def _extract_command_payload_text(self, result: Dict[str, Any]) -> str:
        stdout = optional_text(result.get("stdout"))
        stderr = optional_text(result.get("stderr"))
        if stdout:
            return stdout
        if stderr:
            return stderr
        error = optional_text(result.get("error"))
        if error:
            raise BridgeError(error)
        raise BridgeError("runner returned no output")

    def _nonzero_command_error(self, result: Dict[str, Any]) -> str:
        stderr = optional_text(result.get("stderr"))
        stdout = optional_text(result.get("stdout"))
        error = optional_text(result.get("error"))
        if stderr:
            return stderr
        if stdout:
            return stdout
        if error:
            return error
        exit_code = int(result.get("exit_code", 1) or 1)
        return f"runner command exited with code {exit_code}"

    def _truncate_telegram_message(self, text: str) -> str:
        cleaned = str(text or "").strip()
        if len(cleaned) <= MAX_TELEGRAM_MESSAGE_LENGTH:
            return cleaned
        truncated = cleaned[: MAX_TELEGRAM_MESSAGE_LENGTH - 40].rstrip()
        return f"{truncated}\n\n… (truncated)"

    def _resolve_runner_session_key(self, session: Dict[str, Any]) -> str:
        direct = optional_text(session.get("runner_session_key"))
        if direct:
            return direct
        session_id = optional_text(session.get("id"))
        if not session_id:
            return ""
        with self._state_lock:
            return optional_text(self.session_runner_keys.get(session_id))

    def _make_claude_stream_callback(
        self,
        session: Dict[str, Any],
        on_partial_text: Optional[Callable[[str], None]],
        on_stream_activity: Optional[Callable[[], None]],
    ) -> Optional[Callable[[str], None]]:
        if on_partial_text is None:
            return None
        state = {"chunks": [], "final_result": ""}

        def handle(raw_line: str) -> None:
            candidate = str(raw_line or "").strip()
            if not candidate:
                return
            try:
                payload = json.loads(candidate)
            except json.JSONDecodeError:
                return
            if not isinstance(payload, dict):
                return
            if on_stream_activity is not None:
                on_stream_activity()

            self._remember_runner_session_key(
                session,
                optional_text(payload.get("session_id") or payload.get("sessionId")),
                log_label="claudecode",
            )
            if payload.get("is_error") is True:
                error_text = optional_text(payload.get("result")) or optional_text(payload.get("message"))
                if error_text:
                    state["final_result"] = error_text
                return

            payload_type = optional_text(payload.get("type")).lower()
            if payload_type in {"result", "final"}:
                final_result = optional_text(payload.get("result"))
                if final_result:
                    state["final_result"] = final_result
                return

            chunk = self._extract_claude_stream_text(payload)
            if not chunk:
                return
            state["chunks"].append(chunk)
            on_partial_text("".join(state["chunks"]).strip())

        return handle

    def _extract_claude_stream_text(self, payload: Dict[str, Any]) -> str:
        payload_type = optional_text(payload.get("type")).lower()
        if payload_type in {"content_block_delta", "message_delta"}:
            delta = payload.get("delta") if isinstance(payload.get("delta"), dict) else {}
            return optional_text(delta.get("text"))
        if payload_type == "content_block_start":
            block = payload.get("content_block") if isinstance(payload.get("content_block"), dict) else {}
            return optional_text(block.get("text"))
        if payload_type == "assistant":
            return self._extract_message_content_text(payload.get("message"))
        if payload_type == "text":
            return optional_text(payload.get("text"))
        return ""

    def _extract_message_content_text(self, payload: Any) -> str:
        if not isinstance(payload, dict):
            return ""
        content = payload.get("content")
        if not isinstance(content, list):
            return ""
        chunks: List[str] = []
        for item in content:
            if not isinstance(item, dict):
                continue
            chunk = optional_text(item.get("text")) or optional_text(item.get("value"))
            if chunk:
                chunks.append(chunk)
        return "".join(chunks).strip()

    def _make_opencode_stream_callback(
        self,
        session: Dict[str, Any],
        on_partial_text: Optional[Callable[[str], None]],
        on_stream_activity: Optional[Callable[[], None]],
    ) -> Optional[Callable[[str], None]]:
        if on_partial_text is None:
            return None
        chunks: List[str] = []

        def handle(raw_line: str) -> None:
            candidate = str(raw_line or "").strip()
            if not candidate:
                return
            try:
                payload = json.loads(candidate)
            except json.JSONDecodeError:
                return
            if not isinstance(payload, dict):
                return
            if on_stream_activity is not None:
                on_stream_activity()
            self._remember_runner_session_key(
                session,
                optional_text(
                    payload.get("sessionID") or payload.get("sessionId") or payload.get("session_id")
                ),
                log_label="opencode",
            )
            chunk = self._extract_opencode_text(payload)
            if not chunk:
                return
            chunks.append(chunk)
            on_partial_text("\n".join(chunks).strip())

        return handle

    def _extract_opencode_text(self, payload: Dict[str, Any]) -> str:
        payload_type = optional_text(payload.get("type")).lower()
        part = payload.get("part") if isinstance(payload.get("part"), dict) else {}
        if payload_type == "error":
            return ""

        direct = optional_text(part.get("text")) or optional_text(payload.get("text"))
        if direct:
            return direct

        if payload_type in {"assistant", "message"}:
            message = payload.get("message") if isinstance(payload.get("message"), dict) else payload
            return self._extract_message_content_text(message)

        delta = payload.get("delta") if isinstance(payload.get("delta"), dict) else {}
        return optional_text(delta.get("text"))

    def _remember_runner_session_key(
        self,
        session: Dict[str, Any],
        discovered: str,
        *,
        log_label: str,
    ) -> None:
        session_id = optional_text(session.get("id"))
        if not session_id or not discovered:
            return
        previous = optional_text(session.get("runner_session_key"))
        if previous == discovered:
            return
        session["runner_session_key"] = discovered
        with self._state_lock:
            self.session_runner_keys[session_id] = discovered
        updater = getattr(self.ruler, "update_session_runner_key", None)
        if callable(updater):
            try:
                updater(
                    session_id=session_id,
                    runner_session_key=discovered,
                )
            except BridgeError as err:
                log_info(
                    f"unable to persist {log_label} session key for telegram_session={session_id}: {err}"
                )
                return
        self.save_state()
        log_info(
            f"learned {log_label} session key: "
            f"telegram_session={session_id} runner_session={discovered}"
        )

    def _parse_claudecode_reply(self, raw: str, session: Dict[str, Any]) -> str:
        payloads = self._parse_json_objects(raw)
        chunks: List[str] = []
        final_result = ""
        for payload in payloads:
            session_key = optional_text(payload.get("session_id") or payload.get("sessionId"))
            self._remember_runner_session_key(session, session_key, log_label="claudecode")
            result_text = optional_text(payload.get("result"))
            if payload.get("is_error") is True:
                raise BridgeError(result_text or "Claude Code returned an error")
            payload_type = optional_text(payload.get("type")).lower()
            if payload_type in {"result", "final"} and result_text:
                final_result = result_text
                continue
            chunk = self._extract_claude_stream_text(payload)
            if chunk:
                chunks.append(chunk)

        if final_result:
            return final_result
        if chunks:
            return "".join(chunks).strip()

        cleaned = raw.strip()
        if cleaned:
            return cleaned
        raise BridgeError("Claude Code returned no output")

    def _parse_opencode_reply(
        self,
        raw: str,
        session: Dict[str, Any],
    ) -> str:
        payloads = self._parse_json_objects(raw)
        reply_chunks: List[str] = []
        error_text: Optional[str] = None

        for payload in payloads:
            if not isinstance(payload, dict):
                continue
            session_candidate = optional_text(
                payload.get("sessionID") or payload.get("sessionId") or payload.get("session_id")
            )
            if session_candidate:
                self._remember_runner_session_key(
                    session,
                    session_candidate,
                    log_label="opencode",
                )

            payload_type = optional_text(payload.get("type")).lower()
            if payload_type == "error":
                part = payload.get("part") if isinstance(payload.get("part"), dict) else {}
                error_text = (
                    optional_text(part.get("text"))
                    or optional_text(payload.get("error"))
                    or optional_text(payload.get("message"))
                )
                continue

            text = self._extract_opencode_text(payload)
            if text:
                reply_chunks.append(text)

        if error_text:
            raise BridgeError(error_text)
        if reply_chunks:
            return "\n".join(reply_chunks).strip()
        return "✅ OpenCode completed the request, but did not emit a text summary."

    def _parse_json_objects(self, raw: str) -> List[Dict[str, Any]]:
        trimmed = str(raw or "").strip()
        if not trimmed:
            raise BridgeError("runner returned no output")
        try:
            parsed = json.loads(trimmed)
        except json.JSONDecodeError:
            values: List[Dict[str, Any]] = []
            for index, line in enumerate(trimmed.splitlines(), start=1):
                candidate = line.strip()
                if not candidate:
                    continue
                if not (candidate.startswith("{") or candidate.startswith("[")):
                    continue
                try:
                    value = json.loads(candidate)
                except json.JSONDecodeError as err:
                    log_info(
                        "ignoring non-JSON structured-output line: "
                        f"line={index} detail={err}"
                    )
                    continue
                if isinstance(value, dict):
                    values.append(value)
            if values:
                return values
            raise BridgeError("runner returned invalid structured output")
        if isinstance(parsed, dict):
            return [parsed]
        raise BridgeError("runner returned malformed structured output")

    def _build_runner_prompt(
        self,
        prompt: str,
        attachments: Sequence[TelegramAttachment],
    ) -> str:
        message_text = optional_text(prompt)
        if not attachments:
            return message_text

        lines: List[str] = []
        if message_text:
            lines.extend([message_text, ""])
        else:
            lines.extend(
                [
                    "Please inspect the staged Telegram attachments below and respond normally.",
                    "",
                ]
            )
        lines.append("Telegram attachments saved in the workspace:")
        for index, attachment in enumerate(attachments, start=1):
            detail = f"{index}. {attachment.file_name} [{attachment.kind}"
            if attachment.mime_type:
                detail += f", {attachment.mime_type}"
            if attachment.size_bytes > 0:
                detail += f", {attachment.size_bytes} bytes"
            detail += f"] -> {attachment.prompt_path}"
            lines.append(detail)
        lines.extend(
            [
                "",
                "Use the file paths above directly from the workspace when responding.",
            ]
        )
        return "\n".join(lines).strip()

    def _bootstrap_thread_for_private_chat(self, chat_id: str, text: str) -> Optional[int]:
        topic_name = self._derive_topic_name(text)
        try:
            created = self.telegram.create_forum_topic(chat_id=chat_id, name=topic_name)
        except BridgeError as err:
            log_info(f"unable to bootstrap private topic for chat {chat_id}: {err}")
            return None

        if not isinstance(created, dict):
            log_info(f"unexpected createForumTopic payload for chat {chat_id}: {created!r}")
            return None

        topic_payload = created.get("result") if isinstance(created.get("result"), dict) else created
        thread_id = topic_payload.get("message_thread_id") if isinstance(topic_payload, dict) else None
        if not isinstance(thread_id, int) or thread_id <= 0:
            log_info(f"createForumTopic returned invalid thread id for chat {chat_id}: {created!r}")
            return None
        return thread_id

    def _extract_message_text(self, message: Dict[str, Any]) -> str:
        text = optional_text(message.get("text"))
        if text:
            return text
        return optional_text(message.get("caption"))

    def _message_has_supported_attachments(self, message: Dict[str, Any]) -> bool:
        return bool(self._attachment_descriptors(message))

    def _stage_message_attachments(
        self,
        message: Dict[str, Any],
        *,
        chat_id: str,
        thread_id: Optional[int],
        message_id: Optional[int],
    ) -> List[TelegramAttachment]:
        descriptors = self._attachment_descriptors(message)
        if not descriptors:
            return []

        workspace_root = self._runner_workspace_root()
        thread_label = str(thread_id) if isinstance(thread_id, int) and thread_id > 0 else "root"
        message_label = str(message_id) if isinstance(message_id, int) and message_id > 0 else str(int(time.time()))
        relative_dir = (
            Path(".agent-ruler-telegram")
            / self.config.runner_kind
            / f"chat-{self._safe_path_token(chat_id)}"
            / f"thread-{self._safe_path_token(thread_label)}"
            / f"msg-{self._safe_path_token(message_label)}"
        )
        host_dir = workspace_root / relative_dir
        host_dir.mkdir(parents=True, exist_ok=True)

        staged: List[TelegramAttachment] = []
        for index, descriptor in enumerate(descriptors, start=1):
            file_id = descriptor["file_id"]
            file_info = self.telegram.get_file(file_id)
            file_path = optional_text(file_info.get("file_path"))
            if not file_path:
                raise BridgeError(
                    f"telegram getFile did not return a file path for `{descriptor['kind']}`"
                )
            payload = self.telegram.download_file(file_path)
            file_name = self._attachment_storage_name(descriptor, file_path, index)
            host_path = self._unique_path(host_dir / file_name)
            host_path.write_bytes(payload)
            staged.append(
                TelegramAttachment(
                    kind=descriptor["kind"],
                    file_name=host_path.name,
                    mime_type=descriptor["mime_type"],
                    size_bytes=len(payload),
                    host_path=host_path,
                    prompt_path=(relative_dir / host_path.name).as_posix(),
                )
            )
        log_info(
            "staged telegram attachments: "
            f"runner={self.config.runner_kind} chat={chat_id} thread={thread_label} count={len(staged)}"
        )
        return staged

    def _runner_workspace_root(self) -> Path:
        workspace = (
            Path(self.config.runtime_dir).expanduser()
            / "user_data"
            / "runners"
            / self.config.runner_kind
            / "workspace"
        )
        workspace.mkdir(parents=True, exist_ok=True)
        return workspace

    def _attachment_descriptors(self, message: Dict[str, Any]) -> List[Dict[str, Any]]:
        descriptors: List[Dict[str, Any]] = []

        photo_items = message.get("photo")
        if isinstance(photo_items, list):
            candidates = [item for item in photo_items if isinstance(item, dict)]
            if candidates:
                largest = max(candidates, key=lambda item: int(item.get("file_size", 0) or 0))
                file_id = optional_text(largest.get("file_id"))
                if file_id:
                    photo_token = optional_text(largest.get("file_unique_id")) or file_id
                    descriptors.append(
                        {
                            "kind": "photo",
                            "file_id": file_id,
                            "file_name": f"photo-{self._safe_path_token(photo_token)}.jpg",
                            "mime_type": "image/jpeg",
                            "size_bytes": int(largest.get("file_size", 0) or 0),
                        }
                    )

        for field, kind, fallback_name, fallback_mime in [
            ("document", "document", "document.bin", ""),
            ("video", "video", "video.mp4", "video/mp4"),
            ("audio", "audio", "audio.mp3", "audio/mpeg"),
            ("voice", "voice", "voice.ogg", "audio/ogg"),
            ("animation", "animation", "animation.gif", "image/gif"),
            ("video_note", "video_note", "video-note.mp4", "video/mp4"),
            ("sticker", "sticker", "sticker.webp", "image/webp"),
        ]:
            item = message.get(field)
            if not isinstance(item, dict):
                continue
            file_id = optional_text(item.get("file_id"))
            if not file_id:
                continue
            file_name = optional_text(item.get("file_name")) or fallback_name
            mime_type = optional_text(item.get("mime_type")) or fallback_mime
            descriptors.append(
                {
                    "kind": kind,
                    "file_id": file_id,
                    "file_name": file_name,
                    "mime_type": mime_type,
                    "size_bytes": int(item.get("file_size", 0) or 0),
                }
            )

        return descriptors

    def _attachment_storage_name(
        self,
        descriptor: Dict[str, Any],
        telegram_file_path: str,
        index: int,
    ) -> str:
        original_name = optional_text(descriptor.get("file_name"))
        if not original_name:
            original_name = Path(telegram_file_path).name
        if not original_name:
            original_name = f"{descriptor['kind']}-{index}"
        sanitized = self._safe_attachment_name(original_name, descriptor["kind"], descriptor["mime_type"])
        return f"{index:02d}-{sanitized}"

    def _safe_attachment_name(self, name: str, kind: str, mime_type: str) -> str:
        candidate = Path(name).name.strip()
        if not candidate:
            candidate = kind
        stem = Path(candidate).stem or kind
        suffix = Path(candidate).suffix
        if not suffix and mime_type:
            suffix = mimetypes.guess_extension(mime_type) or ""
        safe_stem = re.sub(r"[^A-Za-z0-9._-]+", "-", stem).strip(".-_") or kind
        safe_suffix = re.sub(r"[^A-Za-z0-9.]+", "", suffix)[:12]
        return f"{safe_stem[:80]}{safe_suffix}"

    def _safe_path_token(self, value: str) -> str:
        cleaned = re.sub(r"[^A-Za-z0-9._-]+", "-", str(value or "").strip())
        return cleaned.strip(".-_") or "unknown"

    def _unique_path(self, path: Path) -> Path:
        if not path.exists():
            return path
        stem = path.stem
        suffix = path.suffix
        parent = path.parent
        for index in range(2, 1000):
            candidate = parent / f"{stem}-{index}{suffix}"
            if not candidate.exists():
                return candidate
        raise BridgeError(f"unable to allocate attachment path for {path.name}")

    def _handle_decision_command(
        self,
        *,
        parsed: ParsedDecisionCommand,
        user_id: str,
        chat_id: str,
        thread_id: Optional[int],
        callback_id: str,
        reply_to_message_id: Optional[int],
    ) -> None:
        if callback_id:
            self.telegram.answer_callback(callback_id, "🚨 Processing approval decision...")

        if not self._chat_allowed(chat_id, thread_id):
            return

        if not self._sender_allowed(user_id):
            self._safe_reply(
                chat_id,
                thread_id,
                "🚫 Decision ignored : Sender is not allowed.",
                reply_to_message_id=reply_to_message_id,
            )
            return

        approval_id = self._resolve_approval_reference(parsed.reference)
        if not approval_id:
            self._safe_reply(
                chat_id,
                thread_id,
                f"🚫 Unknown approval reference : `{parsed.reference}`. Check active approvals in Control Panel.",
                reply_to_message_id=reply_to_message_id,
            )
            return

        try:
            result = self.ruler.resolve(approval_id, parsed.decision)
        except BridgeError as err:
            self._record_ui_log(
                level="error",
                message="Telegram approval decision failed",
                details={
                    "approval_id": approval_id,
                    "decision": parsed.decision,
                    "chat_id": chat_id,
                    "thread_id": thread_id,
                    "error": str(err),
                },
            )
            self._safe_reply(
                chat_id,
                thread_id,
                f"🚫 Decision failed for `{approval_id}` : {err}",
                reply_to_message_id=reply_to_message_id,
            )
            return

        status = str(result.get("status", "resolved"))
        self._remove_pending(approval_id)
        if parsed.decision == "approve":
            self._safe_reply(
                chat_id,
                thread_id,
                f"✅ Approved : `{approval_id}` ({status}).",
                reply_to_message_id=reply_to_message_id,
            )
        else:
            self._safe_reply(
                chat_id,
                thread_id,
                f"🚫 Denied : `{approval_id}` ({status}).",
                reply_to_message_id=reply_to_message_id,
            )
        log_info(
            "inbound decision resolved: "
            f"approval_id={approval_id} decision={parsed.decision} user={user_id}"
        )
        self._record_ui_log(
            level="info",
            message="Telegram approval decision resolved",
            details={
                "approval_id": approval_id,
                "decision": parsed.decision,
                "chat_id": chat_id,
                "thread_id": thread_id,
                "status": status,
            },
        )

    def _safe_reply(
        self,
        chat_id: str,
        thread_id: Optional[int],
        message: str,
        *,
        reply_to_message_id: Optional[int] = None,
    ) -> None:
        if not chat_id:
            return
        try:
            self._send_text_message(
                chat_id,
                thread_id if thread_id is not None else None,
                message,
                reply_to_message_id=reply_to_message_id,
            )
        except BridgeError as err:
            log_info(f"reply failed for chat {chat_id}: {err}")

    def _sender_allowed(self, sender_id: str) -> bool:
        allow = [item for item in self.config.allow_from if item]
        if not allow:
            return False
        if "*" in allow:
            return True
        return sender_id in allow

    def _chat_allowed(self, chat_id: str, thread_id: Optional[int]) -> bool:
        if not chat_id:
            return False
        _ = thread_id
        return True

    def _resolve_approval_reference(self, reference: str) -> Optional[str]:
        ref = reference.strip()
        if not ref:
            return None
        pending = self.pending_by_approval.get(ref)
        if pending is not None:
            return pending.approval_id

        short = normalize_short_id(ref)
        if short is not None and short in self.pending_by_short:
            return self.pending_by_short[short].approval_id

        if short is not None:
            return None
        if re.fullmatch(r"[A-Za-z0-9._:-]{4,200}", ref):
            return ref
        return None

    def _register_pending(self, approval_id: str) -> PendingApproval:
        with self._state_lock:
            existing = self.pending_by_approval.get(approval_id)
            if existing is not None:
                return existing
            for _ in range(100):
                short_id = "".join(
                    secrets.choice(SHORT_ID_ALPHABET) for _ in range(self.config.short_id_length)
                )
                if short_id not in self.pending_by_short:
                    break
            else:
                short_id = f"AR{len(self.pending_by_short) + 1:04d}"
            pending = PendingApproval(
                approval_id=approval_id,
                short_id=short_id,
                created_at=int(time.time()),
                notified=False,
            )
            self.pending_by_approval[pending.approval_id] = pending
            self.pending_by_short[pending.short_id] = pending
            return pending

    def _remove_pending(self, approval_id: str) -> None:
        with self._state_lock:
            pending = self.pending_by_approval.pop(approval_id, None)
            if pending is not None:
                self.pending_by_short.pop(pending.short_id, None)

    def _prune_expired_pending(self) -> None:
        expired: List[str] = []
        for approval_id, pending in self.pending_by_approval.items():
            if self._is_expired(pending):
                expired.append(approval_id)
        for approval_id in expired:
            self._remove_pending(approval_id)

    def _is_expired(self, pending: PendingApproval) -> bool:
        return int(time.time()) > pending.created_at + self.config.decision_ttl_seconds

    def _make_deep_link(self, path: str) -> str:
        cleaned = path.strip() or "/approvals"
        if cleaned.startswith("http://") or cleaned.startswith("https://"):
            return cleaned
        if not cleaned.startswith("/"):
            cleaned = f"/{cleaned}"
        return f"{self.config.public_base_url.rstrip('/')}{cleaned}"

    def _threadless_mode_help(self) -> str:
        return (
            "🧵 Threaded mode is required.\n\n"
            f"Runner: {self.runner_label()}\n"
            "Enable \"Threaded Mode\" in BotFather, then start a thread and send /status.\n"
            "Use /whoami to get your Telegram sender ID for allow_from setup."
        )

    def _help_message(self, session: Dict[str, Any]) -> str:
        lines = [
            "📘 Agent Ruler Telegram",
            "",
            f"Runner: {self.runner_label()}",
            "Enable \"Threaded Mode\" in BotFather for 1:1 threaded chats.",
            "",
            "Commands:",
            "/start - show this help.",
            "/help - show this help.",
            "/whoami - show your Telegram sender ID for allow_from setup.",
            "/status - show the current runner, Agent Ruler session, and Telegram thread binding.",
            "/continue - bind this thread to the recent runner session when available.",
            "/continue <session-id> - bind this thread to a specific Agent Ruler session.",
            "/continue <runner-session-key> - bind this thread to a specific runner session.",
            "/link - alias for /continue.",
            "/new [topic] - create a fresh Telegram topic/session for a different task.",
            "approve SHORTID - approve a pending action.",
            "deny SHORTID - deny a pending action.",
            "",
            "Conversation:",
            "Plain text messages are forwarded to the bound runner session and replies stay in-thread.",
            "Typing keepalive is sent while the runner is working or waiting; Telegram chat actions do not support custom waiting text.",
            "For long operations a short waiting/progress message is posted in-thread before the final reply.",
            "Photos, videos, documents, voice notes, audio, and stickers are staged into the managed workspace and referenced in the forwarded message.",
            "",
            f"🔗 Control Panel: {self._make_deep_link('/runners')}",
        ]
        status_lines = self._status_lines(session, created=False)
        if status_lines:
            lines.extend(["", *status_lines])
        return "\n".join(lines)

    def _status_message(
        self,
        session: Dict[str, Any],
        *,
        created: bool,
        heading: Optional[str] = None,
    ) -> str:
        heading = heading or ("✅ Session ready" if created else "🧵 Thread status")
        return "\n".join([heading, *self._status_lines(session, created=created)])

    def _status_lines(self, session: Dict[str, Any], *, created: bool) -> List[str]:
        display_label = optional_text(session.get("display_label"))
        session_id = optional_text(session.get("id"))
        runner_session_key = self._resolve_runner_session_key(session)
        thread_id = session.get("telegram_thread_id")
        last_active = optional_text(session.get("last_active_at"))
        lines = ["", f"Runner: {self.runner_label()}"]
        if display_label:
            lines.append(f"Label: {display_label}")
        if session_id:
            lines.append(f"Agent Ruler Session: {session_id}")
        if runner_session_key:
            lines.append(f"Runner Session: {runner_session_key}")
        if isinstance(thread_id, int) and thread_id > 0:
            lines.append(f"Thread: {thread_id}")
        if last_active:
            lines.append(f"Last Active: {last_active}")
        lines.append(f"🔗 Control Panel: {self._make_deep_link('/runners')}")
        if created:
            lines.append("Use /status anytime to confirm the current runner binding.")
        return lines

    def _whoami_message(self, sender_id: str) -> str:
        resolved = sender_id.strip() or "unknown"
        return "\n".join(
            [
                "🆔 Telegram sender identity",
                "",
                f"Sender ID: {resolved}",
                "Use this value in bridge `allow_from` for this runner.",
                "After saving, send /status in your Telegram thread to bind the session.",
            ]
        )

    def _disabled_mode_message(self) -> str:
        return (
            "⏸️ Telegram bridge is currently disabled.\n\n"
            "Only /whoami is available in this mode.\n"
            "Enable the runner Telegram bridge in Control Panel to use /status, /continue, /new, and approvals."
        )

    def _derive_session_title(self, text: str) -> Optional[str]:
        candidate = str(text or "").strip()
        if not candidate or candidate.startswith("/"):
            return None
        first_line = candidate.splitlines()[0].strip()
        if not first_line:
            return None
        if len(first_line) > 80:
            first_line = f"{first_line[:77].rstrip()}..."
        return first_line

    def _derive_topic_name(self, text: str) -> str:
        derived = self._derive_session_title(text)
        if not derived:
            return "Agent Ruler"
        return derived


def normalize_short_id(value: str) -> Optional[str]:
    candidate = value.strip().upper()
    if not candidate or not _SHORT_ID_PATTERN.match(candidate):
        return None
    return candidate


def optional_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value.strip()
    return str(value).strip()


def humanize_label(value: str) -> str:
    cleaned = value.strip().replace("_", " ").replace("-", " ")
    if not cleaned:
        return "Unknown"
    return " ".join(part.capitalize() for part in cleaned.split())


def normalize_text_command(text: str) -> str:
    raw = str(text or "").strip().split()[0] if str(text or "").strip() else ""
    if not raw.startswith("/"):
        return ""
    return raw.split("@", 1)[0].lower()


def describe_approval_reason(reason_code: str, category: str) -> str:
    reason = str(reason_code or "").strip().lower() or "approval_required"
    category_clean = str(category or "").strip().lower() or "approval_required"
    if reason == "approval_required_export" and category_clean == "shared_zone_stage":
        return "Staging files from workspace to shared zone requires approval."
    if reason == "approval_required_export" and category_clean == "deliver":
        return "Delivering files from shared zone to a user destination requires approval."
    message = _REASON_DESCRIPTIONS.get(reason)
    if not message:
        message = _CATEGORY_DESCRIPTIONS.get(category_clean, "A protected action needs your confirmation")
    return message


def extract_command_argument(text: str) -> str:
    parts = str(text or "").strip().split(maxsplit=1)
    if len(parts) < 2:
        return ""
    return parts[1].strip()


def normalize_runner_kind(value: str) -> str:
    candidate = str(value or "").strip().lower()
    if candidate in RUNNER_LABELS:
        return candidate
    return ""


def parse_decision_command(text: str) -> Optional[ParsedDecisionCommand]:
    raw = text.strip()
    if not raw:
        return None
    callback = parse_callback_payload(raw)
    if callback is not None:
        return callback

    match = _COMMAND_PATTERN.match(raw)
    if not match:
        return None
    decision = match.group(1).lower()
    reference = match.group(2).strip()
    short = normalize_short_id(reference)
    if short is not None:
        reference = short
    return ParsedDecisionCommand(decision=decision, reference=reference)


def parse_callback_payload(payload: str) -> Optional[ParsedDecisionCommand]:
    match = _CALLBACK_PATTERN.match(payload.strip())
    if not match:
        return None
    decision = match.group(1).lower()
    reference = match.group(2).strip()
    short = normalize_short_id(reference)
    if short is not None:
        reference = short
    return ParsedDecisionCommand(decision=decision, reference=reference)


def unique_trimmed(values: Sequence[str]) -> List[str]:
    output: List[str] = []
    for value in values:
        trimmed = str(value).strip()
        if not trimmed:
            continue
        if trimmed in output:
            continue
        output.append(trimmed)
    return output


def parse_chat_target(raw: str) -> ChatTarget:
    value = str(raw or "").strip()
    if not value:
        raise BridgeError("chat_ids entries must not be empty")

    if "#" not in value:
        return ChatTarget(chat_id=value, message_thread_id=None)

    chat_id, thread_raw = value.split("#", 1)
    chat_id = chat_id.strip()
    thread_raw = thread_raw.strip()
    if not chat_id:
        raise BridgeError(f"invalid chat target `{value}`: missing chat id")
    if not thread_raw:
        raise BridgeError(f"invalid chat target `{value}`: missing thread id")
    try:
        thread_id = int(thread_raw)
    except ValueError as err:
        raise BridgeError(f"invalid chat target `{value}`: thread id must be an integer") from err
    if thread_id <= 0:
        raise BridgeError(f"invalid chat target `{value}`: thread id must be positive")

    return ChatTarget(chat_id=chat_id, message_thread_id=thread_id)


def load_config(path: Path, args: argparse.Namespace) -> BridgeConfig:
    raw = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        raise BridgeError(f"invalid config file: {path}")

    runner_kind = normalize_runner_kind(
        str(raw.get("runner_kind") or os.environ.get("AGENT_RULER_RUNNER_ID") or "")
    )
    if not runner_kind:
        raise BridgeError("runner_kind is required in bridge config")
    enabled = bool(raw.get("enabled", False))
    answer_streaming_enabled = bool(raw.get("answer_streaming_enabled", True))
    ruler_url = str(args.ruler_url or raw.get("ruler_url") or "http://127.0.0.1:4622").strip()
    public_base_url = str(
        args.public_base_url or raw.get("public_base_url") or ruler_url
    ).strip()
    poll_interval_seconds = int(raw.get("poll_interval_seconds") or DEFAULT_POLL_INTERVAL_SECONDS)
    decision_ttl_seconds = int(raw.get("decision_ttl_seconds") or 7200)
    short_id_length = int(raw.get("short_id_length") or DEFAULT_SHORT_ID_LENGTH)
    state_file_raw = str(args.state_file or raw.get("state_file") or "").strip()
    if not state_file_raw:
        raise BridgeError("state_file is required")
    state_file = Path(state_file_raw).expanduser()
    runtime_dir = str(raw.get("runtime_dir") or "").strip()
    bot_token = str(raw.get("bot_token") or "").strip()
    chat_ids: List[str] = []
    allow_from = unique_trimmed([str(item) for item in raw.get("allow_from", []) or []])

    return BridgeConfig(
        runner_kind=runner_kind,
        enabled=enabled,
        answer_streaming_enabled=answer_streaming_enabled,
        ruler_url=ruler_url,
        public_base_url=public_base_url,
        poll_interval_seconds=max(1, min(poll_interval_seconds, 300)),
        decision_ttl_seconds=max(60, min(decision_ttl_seconds, 604800)),
        short_id_length=max(4, min(short_id_length, 10)),
        state_file=state_file,
        runtime_dir=runtime_dir,
        bot_token=bot_token,
        chat_ids=chat_ids,
        allow_from=allow_from,
    )


def bridge_lock_path(config: BridgeConfig) -> Path:
    token_hash = hashlib.sha256(config.bot_token.encode("utf-8")).hexdigest()[:16]
    return Path(tempfile.gettempdir()) / f"agent-ruler-telegram-{token_hash}.lock"


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Agent Ruler Telegram approvals bridge")
    parser.add_argument("--config", required=True, help="Bridge config JSON file")
    parser.add_argument("--ruler-url", help="Override Agent Ruler base URL")
    parser.add_argument("--public-base-url", help="Override public Control Panel base URL")
    parser.add_argument("--state-file", help="Override state file path")
    parser.add_argument("--once", action="store_true", help="Run one polling tick and exit")
    return parser


def main(argv: Optional[List[str]] = None) -> int:
    args = build_arg_parser().parse_args(argv)
    try:
        config = load_config(Path(args.config), args)
    except Exception as err:
        print(f"config error: {err}", file=sys.stderr)
        return 2

    if not config.bot_token:
        print("config error: bot_token is empty", file=sys.stderr)
        return 2

    ruler = AgentRulerClient(config.ruler_url)
    telegram = TelegramClient(config.bot_token)
    bridge_lock = BridgeInstanceLock(bridge_lock_path(config))
    try:
        bridge_lock.acquire(runner_kind=config.runner_kind)
    except BridgeError as err:
        print(f"[telegram-bridge] startup blocked: {err}", file=sys.stderr)
        return 2
    runtime = TelegramBridgeRuntime(config, ruler, telegram)
    log_info(
        "config loaded: "
        f"runner={config.runner_kind} chat_ids={len(config.chat_ids)} "
        f"allow_from={len(config.allow_from)} poll={config.poll_interval_seconds}s "
        f"streaming={'on' if config.answer_streaming_enabled else 'off'}"
    )
    if not config.enabled:
        log_info("bridge command mode: /whoami is available; session/approval commands are disabled")

    if args.once:
        try:
            runtime.tick()
        except Exception as err:
            print(f"[telegram-bridge] one-shot failed: {err}", file=sys.stderr)
            return 1
        return 0

    stop = False

    def handle_signal(_sig: int, _frame: Any) -> None:
        nonlocal stop
        stop = True

    signal.signal(signal.SIGINT, handle_signal)
    signal.signal(signal.SIGTERM, handle_signal)

    try:
        while not stop:
            try:
                runtime.tick()
            except Exception as err:
                print(f"[telegram-bridge] runtime error: {err}", file=sys.stderr)
            # getUpdates already long-polls for a bounded timeout, keep a short
            # sleep so failed loops do not spin.
            time.sleep(0.25)
    finally:
        runtime.save_state()
        bridge_lock.release()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
