#!/usr/bin/env python3
"""OpenClaw channel bridge for Agent Ruler approvals.

This bridge polls Agent Ruler redacted approval status and notifies operators through
OpenClaw channels (Telegram, WhatsApp, Discord). It also accepts inbound chat commands
(forwarded by an OpenClaw message hook) to approve/deny safely.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import queue
import re
import secrets
import signal
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set, Tuple
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen

DEFAULT_RULER_URL = "http://127.0.0.1:4622"
DEFAULT_BRIDGE_BIND = "127.0.0.1:4661"
DEFAULT_STATE_FILE = (
    Path.home() / ".local" / "share" / "agent-ruler" / "bridge" / "openclaw-state.json"
)
DEFAULT_DECISION_TTL_SECONDS = 7200
DEFAULT_RECENT_RESOLVED_SHORT_TTL_SECONDS = 600
DEFAULT_HTTP_TIMEOUT_SECONDS = 10.0
DEFAULT_SHORT_ID_LENGTH = 6
DEFAULT_AGENT_RULER_BIN = "agent-ruler"
DEFAULT_ROUTE_REFRESH_INTERVAL_SECONDS = 8
DEFAULT_TELEGRAM_TYPING_KEEPALIVE_SECONDS = 5
DEFAULT_TELEGRAM_TOKEN_CACHE_TTL_SECONDS = 60

SUPPORTED_CHANNELS = {"telegram", "whatsapp", "discord"}
SHORT_ID_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
OPENCLAW_BRIDGE_ROUTES_POINTER = (
    "plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes"
)
TRANSFER_APPROVAL_CATEGORIES = {"shared_zone_stage", "deliver"}
TRANSFER_APPROVAL_OPERATIONS = {"export_commit", "deliver_commit", "import_copy"}
MAX_APPROVAL_CONTEXT_PREVIEW = 4
MAX_PREAPPROVAL_ASSISTANT_TEXT_CHARS = 3500
RUNNER_LABELS = {
    "claudecode": "Claude Code",
    "opencode": "OpenCode",
    "openclaw": "OpenClaw",
}
REASON_DESCRIPTIONS = {
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
CATEGORY_DESCRIPTIONS = {
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

_CALLBACK_PATTERN = re.compile(
    r"^ar:(approve|deny):([A-Za-z0-9._:-]+)(?::[A-Za-z0-9._~-]+)?$",
    re.IGNORECASE,
)
_COMMAND_PATTERN = re.compile(
    r"^/?(approve|deny)\s+([A-Za-z0-9._:-]+)(?:\s+[A-Za-z0-9._~-]+)?\s*$",
    re.IGNORECASE,
)
_AR_COMMAND_PATTERN = re.compile(
    r"^/?ar(approve|deny)\s+([A-Za-z0-9._:-]+)(?:\s+[A-Za-z0-9._~-]+)?\s*$",
    re.IGNORECASE,
)
_SHORT_ID_PATTERN = re.compile(r"^[A-Z2-9]{4,10}$")


@dataclass
class RouteConfig:
    channel: str
    target: str
    allow_from: List[str]
    account: Optional[str] = None
    telegram_inline_buttons: bool = False
    telegram_streaming_enabled: bool = False
    whatsapp_use_poll: bool = True
    message_thread_id: Optional[int] = None


@dataclass
class BridgeConfig:
    ruler_url: str
    public_base_url: str
    poll_interval_seconds: int
    decision_ttl_seconds: int
    inbound_bind: str
    state_file: Path
    openclaw_bin: str
    openclaw_home: Optional[str]
    agent_ruler_bin: str
    runtime_dir: Optional[Path]
    dry_run_send: bool
    short_id_length: int
    telegram_typing_keepalive: bool
    telegram_typing_interval_seconds: int
    routes_source: str
    routes: List[RouteConfig]


@dataclass
class ParsedDecisionCommand:
    decision: str
    reference: str


@dataclass
class PendingApproval:
    approval_id: str
    short_id: str
    created_at: int
    notified: bool = True


class BridgeError(Exception):
    """Bridge-specific error type."""


def log_info(message: str) -> None:
    print(f"[bridge] {message}", flush=True)


def already_resolved_status_from_error(detail: str) -> Optional[str]:
    """Parse duplicate approval-decision errors into a normalized status."""
    lowered = (detail or "").strip().lower()
    if "not pending" not in lowered and "already resolved" not in lowered:
        return None
    if (
        "status: approved" in lowered
        or '"status":"approved"' in lowered
        or '"status": "approved"' in lowered
    ):
        return "approved"
    if (
        "status: denied" in lowered
        or '"status":"denied"' in lowered
        or '"status": "denied"' in lowered
    ):
        return "denied"
    if (
        "status: expired" in lowered
        or '"status":"expired"' in lowered
        or '"status": "expired"' in lowered
    ):
        return "expired"
    return None


class AgentRulerClient:
    def __init__(self, base_url: str, timeout_seconds: float = DEFAULT_HTTP_TIMEOUT_SECONDS):
        base = base_url.strip() or DEFAULT_RULER_URL
        self.base_url = base.rstrip("/")
        self.timeout_seconds = timeout_seconds

    def status_feed(self, include_resolved: bool = False, limit: int = 200) -> List[Dict[str, Any]]:
        path = (
            f"/api/status/feed?include_resolved={'true' if include_resolved else 'false'}"
            f"&limit={max(1, min(limit, 500))}"
        )
        payload = self._request_json("GET", path)
        if not isinstance(payload, list):
            raise BridgeError(f"unexpected /api/status/feed payload: {payload!r}")
        return payload

    def approve(self, approval_id: str) -> Dict[str, Any]:
        safe = quote(approval_id, safe="")
        payload = self._request_json("POST", f"/api/approvals/{safe}/approve", body={})
        if not isinstance(payload, dict):
            raise BridgeError(f"unexpected approval payload: {payload!r}")
        return payload

    def deny(self, approval_id: str) -> Dict[str, Any]:
        safe = quote(approval_id, safe="")
        payload = self._request_json("POST", f"/api/approvals/{safe}/deny", body={})
        if not isinstance(payload, dict):
            raise BridgeError(f"unexpected deny payload: {payload!r}")
        return payload

    def wait_for_approval(
        self, approval_id: str, timeout_seconds: int = 30, poll_ms: int = 500
    ) -> Dict[str, Any]:
        safe = quote(approval_id, safe="")
        timeout = max(1, min(int(timeout_seconds), 300))
        poll = max(100, min(int(poll_ms), 2000))
        payload = self._request_json(
            "GET",
            f"/api/approvals/{safe}/wait?timeout={timeout}&poll_ms={poll}",
        )
        if not isinstance(payload, dict):
            raise BridgeError(f"unexpected wait payload: {payload!r}")
        return payload

    def approval(self, approval_id: str) -> Dict[str, Any]:
        safe = quote(approval_id, safe="")
        payload = self._request_json("GET", f"/api/approvals/{safe}")
        if not isinstance(payload, dict):
            raise BridgeError(f"unexpected approval detail payload: {payload!r}")
        return payload

    def _request_json(self, method: str, path: str, body: Optional[Dict[str, Any]] = None) -> Any:
        url = f"{self.base_url}{path}"
        data = None
        if body is not None:
            data = json.dumps(body).encode("utf-8")
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


class OpenClawMessenger:
    def __init__(
        self,
        openclaw_bin: str = "openclaw",
        dry_run_send: bool = False,
        openclaw_home: Optional[str] = None,
    ):
        self.openclaw_bin = openclaw_bin
        self.dry_run_send = dry_run_send
        self.openclaw_home = openclaw_home
        self._telegram_token_cache: Dict[str, Tuple[str, float]] = {}

    def send_text(
        self,
        *,
        channel: str,
        target: str,
        message: str,
        account: Optional[str] = None,
        telegram_buttons: Optional[List[List[Dict[str, str]]]] = None,
        message_thread_id: Optional[int] = None,
    ) -> Dict[str, Any]:
        cmd = [
            self.openclaw_bin,
            "message",
            "send",
            "--channel",
            channel,
            "--target",
            target,
            "--message",
            message,
        ]
        if account:
            cmd.extend(["--account", account])
        if message_thread_id is not None and channel == "telegram":
            # OpenClaw Telegram uses explicit `--thread-id` for forum topics.
            cmd.extend(["--thread-id", str(message_thread_id)])
        if telegram_buttons is not None:
            cmd.extend(["--buttons", json.dumps(telegram_buttons, separators=(",", ":"))])
        return self._run_command(cmd)

    def send_poll(
        self,
        *,
        channel: str,
        target: str,
        question: str,
        options: Sequence[str],
        account: Optional[str] = None,
    ) -> Dict[str, Any]:
        opts = [opt.strip() for opt in options if opt and opt.strip()]
        if len(opts) < 2:
            raise BridgeError("poll requires at least two options")

        cmd = [
            self.openclaw_bin,
            "message",
            "poll",
            "--channel",
            channel,
            "--target",
            target,
            "--poll-question",
            question,
        ]
        for option in opts:
            cmd.extend(["--poll-option", option])
        if account:
            cmd.extend(["--account", account])
        return self._run_command(cmd)

    def send_typing(self, *, channel: str, target: str, account: Optional[str] = None) -> Dict[str, Any]:
        if channel != "telegram":
            return {"ok": False, "skipped": "unsupported_channel"}
        chat_id = target.strip()
        if not chat_id:
            raise BridgeError("telegram typing keepalive requires non-empty target")

        token = self._resolve_telegram_bot_token(account)
        if not token:
            raise BridgeError("telegram typing keepalive unavailable: bot token not found")
        if self.dry_run_send:
            return {
                "dry_run": True,
                "channel": channel,
                "target": chat_id,
                "action": "typing",
            }

        return self._telegram_post_form(
            token=token,
            method="sendChatAction",
            form={"chat_id": chat_id, "action": "typing"},
            error_prefix="telegram typing keepalive failed",
        )

    def answer_callback(
        self,
        *,
        channel: str,
        callback_query_id: str,
        text: str,
        account: Optional[str] = None,
    ) -> None:
        if channel != "telegram":
            return
        callback_id = callback_query_id.strip()
        if not callback_id:
            return

        token = self._resolve_telegram_bot_token(account)
        if not token:
            log_info(
                "answerCallbackQuery failed: telegram callback answer unavailable: bot token not found"
            )
            return
        if self.dry_run_send:
            return

        try:
            self._telegram_request_json(
                token=token,
                method="answerCallbackQuery",
                payload={"callback_query_id": callback_id, "text": text[:180]},
            )
        except BridgeError as err:
            log_info(f"answerCallbackQuery failed: {err}")

    def _telegram_request_json(
        self,
        *,
        token: str,
        method: str,
        payload: Dict[str, Any],
    ) -> Dict[str, Any]:
        data = json.dumps(payload).encode("utf-8")
        request = Request(
            url=f"https://api.telegram.org/bot{token}/{method}",
            data=data,
            method="POST",
            headers={"Content-Type": "application/json"},
        )
        try:
            with urlopen(request, timeout=DEFAULT_HTTP_TIMEOUT_SECONDS) as response:
                raw = response.read().decode("utf-8", errors="replace")
        except HTTPError as err:
            detail = err.read().decode("utf-8", errors="replace") if err.fp is not None else ""
            raise BridgeError(f"telegram {method} failed ({err.code}): {detail or err.reason}") from err
        except URLError as err:
            raise BridgeError(f"telegram {method} failed: {err}") from err

        if not raw.strip():
            raise BridgeError(f"telegram {method} returned empty response")
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError as err:
            raise BridgeError(f"telegram {method} returned invalid JSON") from err
        if not isinstance(parsed, dict):
            raise BridgeError(f"telegram {method} returned malformed payload")
        if not parsed.get("ok", False):
            raise BridgeError(f"telegram {method} error: {parsed}")
        return parsed

    def _telegram_post_form(
        self,
        *,
        token: str,
        method: str,
        form: Dict[str, str],
        error_prefix: str,
    ) -> Dict[str, Any]:
        data = urlencode(form).encode("utf-8")
        request = Request(
            url=f"https://api.telegram.org/bot{token}/{method}",
            data=data,
            method="POST",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )
        try:
            with urlopen(request, timeout=DEFAULT_HTTP_TIMEOUT_SECONDS) as response:
                raw = response.read().decode("utf-8", errors="replace")
        except HTTPError as err:
            detail = err.read().decode("utf-8", errors="replace") if err.fp is not None else ""
            raise BridgeError(f"{error_prefix} ({err.code}): {detail or err.reason}") from err
        except URLError as err:
            raise BridgeError(f"{error_prefix}: {err}") from err

        if not raw.strip():
            return {"ok": True}
        try:
            payload = json.loads(raw)
            if isinstance(payload, dict):
                return payload
        except json.JSONDecodeError:
            pass
        return {"ok": True}

    def _resolve_telegram_bot_token(self, account: Optional[str]) -> Optional[str]:
        cache_key = (account or "default").strip() or "default"
        now = time.monotonic()
        cached = self._telegram_token_cache.get(cache_key)
        if cached is not None and cached[1] > now:
            return cached[0]

        token = None
        if cache_key != "default":
            token = self._read_openclaw_config_string(
                f"channels.telegram.accounts.{cache_key}.botToken"
            )
            if not token:
                token = self._read_openclaw_config_string(
                    f"channels.telegram.accounts.{cache_key}.token"
                )

        if not token:
            token = self._read_openclaw_config_string("channels.telegram.botToken")
        if not token:
            token = self._read_openclaw_config_string("channels.telegram.token")
        if not token:
            return None

        self._telegram_token_cache[cache_key] = (
            token,
            now + DEFAULT_TELEGRAM_TOKEN_CACHE_TTL_SECONDS,
        )
        return token

    def _read_openclaw_config_string(self, path: str) -> Optional[str]:
        cmd = [self.openclaw_bin, "config", "get", path, "--json"]
        env = os.environ.copy()
        if self.openclaw_home:
            env["OPENCLAW_HOME"] = self.openclaw_home
        try:
            run = subprocess.run(cmd, capture_output=True, text=True, check=False, env=env)
        except OSError:
            return None
        if run.returncode != 0:
            stderr = (run.stderr or "").strip().lower()
            if "config path not found" in stderr:
                return None
            return None
        raw = (run.stdout or "").strip()
        if not raw:
            return None
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            return None
        if isinstance(parsed, str):
            token = parsed.strip()
            return token or None
        return None

    def _run_command(self, cmd: List[str]) -> Dict[str, Any]:
        if self.dry_run_send:
            return {
                "dry_run": True,
                "cmd": cmd,
            }

        run = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if run.returncode != 0:
            stderr = run.stderr.strip() or run.stdout.strip() or "unknown openclaw command failure"
            raise BridgeError(f"openclaw command failed: {stderr}")

        payload = run.stdout.strip()
        if not payload:
            return {"ok": True}
        try:
            return json.loads(payload)
        except json.JSONDecodeError:
            return {"ok": True, "raw": payload}


class StateStore:
    def __init__(self, path: Path):
        self.path = path

    def load(self) -> Dict[str, Any]:
        if not self.path.exists():
            return {"seen_approvals": [], "pending": []}

        raw = self.path.read_text(encoding="utf-8")
        data = json.loads(raw)
        if not isinstance(data, dict):
            return {"seen_approvals": [], "pending": []}

        return {
            "seen_approvals": list(data.get("seen_approvals", [])),
            "pending": list(data.get("pending", [])),
        }

    def save(self, *, seen_approvals: Iterable[str], pending: Iterable[PendingApproval]) -> None:
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
        }
        self.path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.path.with_suffix(".tmp")
        tmp.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        tmp.replace(self.path)


class ApprovalBridgeRuntime:
    def __init__(
        self,
        config: BridgeConfig,
        client: AgentRulerClient,
        messenger: OpenClawMessenger,
    ):
        self.config = config
        self.client = client
        self.messenger = messenger
        self.state_store = StateStore(config.state_file)
        state = self.state_store.load()
        self.seen_approvals = set(str(item) for item in state.get("seen_approvals", []))
        self.pending_by_approval: Dict[str, PendingApproval] = {}
        self.pending_by_short: Dict[str, PendingApproval] = {}
        self.recently_resolved_short: Dict[str, int] = {}
        self._recently_resolved_ttl_seconds = DEFAULT_RECENT_RESOLVED_SHORT_TTL_SECONDS
        self._lock = threading.Lock()
        self._inbound_queue: "queue.Queue[Dict[str, Any]]" = queue.Queue()
        self._inbound_worker_lock = threading.Lock()
        self._inbound_worker_started = False
        self._route_refresh_lock = threading.Lock()
        self._next_routes_refresh_at = 0.0
        self._route_refresh_interval_seconds = max(
            3, min(config.poll_interval_seconds, DEFAULT_ROUTE_REFRESH_INTERVAL_SECONDS)
        )
        self._typing_keepalive_interval_seconds = max(
            3, int(config.telegram_typing_interval_seconds)
        )
        self._last_typing_keepalive_at: Dict[Tuple[str, str, str], float] = {}
        self._sent_preapproval_assistant_keys: Set[str] = set()

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

    def persist_state(self) -> None:
        with self._lock:
            self.state_store.save(
                seen_approvals=self.seen_approvals,
                pending=self.pending_by_approval.values(),
            )

    def refresh_routes(self, *, force: bool = False) -> None:
        if not self.config.openclaw_bin:
            return
        if self.config.routes_source == "bridge_config":
            # Explicit bridge config routes are authoritative.
            return
        if not self.config.routes_source.startswith("openclaw_"):
            # Non-OpenClaw sources (tests/manual injection) are static.
            return

        now = time.monotonic()
        if not force and now < self._next_routes_refresh_at:
            return

        with self._route_refresh_lock:
            now = time.monotonic()
            if not force and now < self._next_routes_refresh_at:
                return

            try:
                source, routes, synced = resolve_openclaw_routes(
                    openclaw_bin=self.config.openclaw_bin,
                    openclaw_home=self.config.openclaw_home,
                    allow_persist=False,
                )
            except BridgeError as err:
                self._next_routes_refresh_at = now + self._route_refresh_interval_seconds
                log_info(f"routes refresh failed: {err}")
                return
            previous_signature = route_signature(self.config.routes)
            updated_signature = route_signature(routes)
            previous_source = self.config.routes_source

            self.config.routes = routes
            self.config.routes_source = source
            self._next_routes_refresh_at = now + self._route_refresh_interval_seconds

            if previous_source != source or previous_signature != updated_signature:
                log_info(f"routes refreshed: source={source} routes={len(routes)}")
            if synced:
                log_info(
                    "routes auto-synced into OpenClaw config at "
                    f"`{OPENCLAW_BRIDGE_ROUTES_POINTER}`."
                )

    def poll_once(self) -> Dict[str, Any]:
        self.refresh_routes()
        self._prune_expired_pending()
        self._prune_recently_resolved_short()
        events = self.client.status_feed(include_resolved=False, limit=200)
        notified = 0
        pending_ids_in_feed: Set[str] = set()
        for event in events:
            approval_id = str(event.get("approval_id", "")).strip()
            verdict = str(event.get("verdict", "")).strip().lower()
            if not approval_id or verdict != "pending":
                continue
            pending_ids_in_feed.add(approval_id)

            pending: Optional[PendingApproval] = None
            should_notify = False
            with self._lock:
                pending = self.pending_by_approval.get(approval_id)
                if pending is None:
                    if approval_id in self.seen_approvals:
                        log_info(
                            "approval re-detected after local state drift: "
                            f"approval_id={approval_id}"
                        )
                    pending = self._register_pending_locked(approval_id)
                    self.seen_approvals.add(approval_id)
                    log_info(
                        f"approval detected: approval_id={approval_id} short_id={pending.short_id}"
                    )
                should_notify = not pending.notified

            if pending is not None and should_notify:
                delivered = self._notify_pending(event, pending)
                if delivered > 0:
                    with self._lock:
                        current = self.pending_by_approval.get(approval_id)
                        if current is not None:
                            current.notified = True
                    notified += 1

        self._reconcile_non_pending(pending_ids_in_feed)
        self._emit_typing_keepalive_for_pending()
        self.persist_state()
        return {"notified": notified, "events_seen": len(events)}

    def _reconcile_non_pending(self, pending_ids: Set[str]) -> None:
        stale: List[Tuple[str, str]] = []
        with self._lock:
            for approval_id, pending in self.pending_by_approval.items():
                if approval_id in pending_ids:
                    continue
                stale.append((approval_id, pending.short_id))
        for approval_id, short_id in stale:
            self._remove_pending(approval_id)
            log_info(
                "approval cleared: "
                f"approval_id={approval_id} short_id={short_id} reason=no-longer-pending"
            )

    def _emit_typing_keepalive_for_pending(self) -> None:
        if not self.config.telegram_typing_keepalive:
            return
        with self._lock:
            has_pending = bool(self.pending_by_approval)
        if not has_pending:
            return

        now = time.monotonic()
        for route in self.config.routes:
            if route.channel != "telegram":
                continue
            slot = (route.channel, route.account or "default", route.target)
            last = self._last_typing_keepalive_at.get(slot, 0.0)
            if now - last < self._typing_keepalive_interval_seconds:
                continue
            try:
                self.messenger.send_typing(
                    channel=route.channel,
                    target=route.target,
                    account=route.account,
                )
                self._last_typing_keepalive_at[slot] = now
            except BridgeError as err:
                log_info(
                    "typing keepalive failed: "
                    f"channel={route.channel} target={route.target} reason={err}"
                )

    def enqueue_inbound_event(self, payload: Dict[str, Any]) -> None:
        self._ensure_inbound_worker()
        self._inbound_queue.put(payload)

    def _ensure_inbound_worker(self) -> None:
        with self._inbound_worker_lock:
            if self._inbound_worker_started:
                return
            worker = threading.Thread(target=self._inbound_worker_loop, daemon=True)
            worker.start()
            self._inbound_worker_started = True

    def _inbound_worker_loop(self) -> None:
        while True:
            payload = self._inbound_queue.get()
            try:
                result = self.handle_inbound_event(payload)
                log_info(
                    "inbound decision async result: "
                    f"status={result.get('status')} reason={result.get('reason', '')}"
                )
            except Exception as err:  # pragma: no cover - defensive
                log_info(f"inbound decision async error: {err}")
            finally:
                self._inbound_queue.task_done()

    def handle_inbound_event(self, payload: Dict[str, Any]) -> Dict[str, Any]:
        self.refresh_routes()
        started_at = time.monotonic()
        inbound = normalize_inbound_event(payload)
        if inbound is None:
            return {"status": "ignored", "reason": "invalid inbound payload"}
        feedback_message: Optional[str] = None

        parsed = parse_decision_command(inbound["content"])
        if parsed is None:
            self._answer_callback(inbound, "🚫 Ignored")
            return {"status": "ignored", "reason": "not an approval command"}

        log_info(
            "inbound decision detected: "
            f"channel={inbound['channel']} sender={inbound['sender']} decision={parsed.decision} reference={parsed.reference}"
        )
        self._answer_callback(inbound, "🚨 Processing approval decision...")

        route = select_route(self.config.routes, inbound)
        if route is None:
            return {
                "status": "ignored",
                "reason": f"no route matched channel={inbound['channel']} sender={inbound['sender']}",
            }

        sender = inbound["sender"]
        if not sender_allowed(route, sender):
            feedback_message = self._emit_feedback(
                inbound,
                "Approval command ignored. Your sender identity is not on the channel allowlist.",
                route,
            )
            result = {"status": "ignored", "reason": "sender not allowed"}
            if feedback_message:
                result["feedback_message"] = feedback_message
            return result

        approval_id = self._resolve_approval_reference(parsed.reference)
        if not approval_id:
            if self._is_recently_resolved_reference(parsed.reference):
                feedback_message = self._emit_feedback(
                    inbound,
                    "This approval was already resolved. Open Control Panel for the latest queue.",
                    route,
                )
                result = {"status": "ignored", "reason": "approval already resolved"}
                if feedback_message:
                    result["feedback_message"] = feedback_message
                return result
            feedback_message = self._emit_feedback(
                inbound,
                f"I could not map '{parsed.reference}' to a pending approval. Open Control Panel to resolve it.",
                route,
            )
            result = {"status": "ignored", "reason": "unknown approval reference"}
            if feedback_message:
                result["feedback_message"] = feedback_message
            return result

        with self._lock:
            pending = self.pending_by_approval.get(approval_id)
        if pending and self._is_expired(pending):
            self._expire_pending(approval_id)
            feedback_message = self._emit_feedback(
                inbound,
                f"Approval reference {parsed.reference} expired. Open Control Panel for the latest queue.",
                route,
            )
            result = {"status": "ignored", "reason": "approval reference expired"}
            if feedback_message:
                result["feedback_message"] = feedback_message
            return result

        try:
            resolve_started = time.monotonic()
            result = self._resolve_via_agent_ruler(parsed.decision, approval_id)
            resolve_ms = int((time.monotonic() - resolve_started) * 1000)
            status = str(result.get("status", parsed.decision)).lower()
            reply_started = time.monotonic()
            if parsed.decision == "approve":
                feedback_message = self._emit_feedback(
                    inbound,
                    f"✅ Approved {approval_id} ({status}).",
                    route,
                )
            else:
                feedback_message = self._emit_feedback(
                    inbound,
                    f"🛑 Denied {approval_id} ({status}).",
                    route,
                )
            reply_ms = int((time.monotonic() - reply_started) * 1000)
            total_ms = int((time.monotonic() - started_at) * 1000)
            log_info(
                "inbound decision latency: "
                f"approval_id={approval_id} decision={parsed.decision} resolve_ms={resolve_ms} reply_ms={reply_ms} total_ms={total_ms}"
            )
        except BridgeError as err:
            feedback_message = self._emit_feedback(
                inbound,
                f"Decision failed for {approval_id}: {err}",
                route,
            )
            result = {"status": "error", "reason": str(err)}
            if feedback_message:
                result["feedback_message"] = feedback_message
            return result

        self._remove_pending(approval_id)
        self.persist_state()
        log_info(
            "inbound decision resolved: "
            f"approval_id={approval_id} decision={parsed.decision} sender={sender}"
        )
        result = {
            "status": "resolved",
            "decision": parsed.decision,
            "approval_id": approval_id,
        }
        if feedback_message:
            result["feedback_message"] = feedback_message
        return result

    def _answer_callback(self, inbound: Dict[str, str], text: str) -> None:
        if inbound.get("channel") != "telegram":
            return
        callback_id = inbound.get("callback_query_id", "").strip()
        if not callback_id:
            return
        self.messenger.answer_callback(
            channel="telegram",
            callback_query_id=callback_id,
            text=text,
            account=inbound.get("account") or None,
        )

    def _resolve_via_agent_ruler(self, decision: str, approval_id: str) -> Dict[str, Any]:
        decision = decision.strip().lower()
        if decision not in {"approve", "deny"}:
            raise BridgeError(f"unsupported decision `{decision}`")

        expected_status = "approved" if decision == "approve" else "denied"
        try:
            if decision == "approve":
                result = self.client.approve(approval_id)
            else:
                result = self.client.deny(approval_id)
            payload = result if isinstance(result, dict) else {}
            payload = dict(payload)
            payload.setdefault("status", expected_status)
            payload.setdefault("via", "agent-ruler-api")
            return payload
        except BridgeError as api_err:
            resolved_status = already_resolved_status_from_error(str(api_err))
            if resolved_status == expected_status:
                return {
                    "status": expected_status,
                    "via": "agent-ruler-api",
                    "already_resolved": True,
                }
            if resolved_status is not None:
                raise BridgeError(
                    f"approval already resolved as `{resolved_status}`; requested `{expected_status}`"
                ) from api_err
            agent_ruler_bin = self.config.agent_ruler_bin.strip()
            if not agent_ruler_bin:
                raise
            log_info(
                "inbound decision api fallback: "
                f"approval_id={approval_id} decision={decision} reason={api_err}"
            )
            return self._resolve_via_agent_ruler_cli(
                decision=decision,
                approval_id=approval_id,
                agent_ruler_bin=agent_ruler_bin,
            )

    def _resolve_via_agent_ruler_cli(
        self,
        *,
        decision: str,
        approval_id: str,
        agent_ruler_bin: str,
    ) -> Dict[str, Any]:
        cmd = [agent_ruler_bin]
        if self.config.runtime_dir:
            cmd.extend(["--runtime-dir", str(self.config.runtime_dir)])
        cmd.extend(["approve", "--decision", decision, "--id", approval_id])
        run = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if run.returncode != 0:
            detail = (run.stderr or run.stdout or "").strip()
            if not detail:
                detail = f"exit status {run.returncode}"
            expected_status = "approved" if decision == "approve" else "denied"
            resolved_status = already_resolved_status_from_error(detail)
            if resolved_status == expected_status:
                return {
                    "status": expected_status,
                    "via": "agent-ruler-cli",
                    "already_resolved": True,
                }
            if resolved_status is not None:
                raise BridgeError(
                    f"approval already resolved as `{resolved_status}`; requested `{expected_status}`"
                )
            raise BridgeError(f"`{' '.join(cmd)}` failed: {detail}")
        stdout_detail = (run.stdout or "").strip()
        stderr_detail = (run.stderr or "").strip()
        combined_lower = "\n".join(part for part in [stdout_detail, stderr_detail] if part).lower()
        if "no approvals matched" in combined_lower:
            raise BridgeError(
                f"`{' '.join(cmd)}` reported no approvals matched for `{approval_id}`"
            )

        wait_cmd = [agent_ruler_bin]
        if self.config.runtime_dir:
            wait_cmd.extend(["--runtime-dir", str(self.config.runtime_dir)])
        wait_cmd.extend(["wait", "--id", approval_id, "--timeout", "1", "--json"])
        wait_run = subprocess.run(wait_cmd, capture_output=True, text=True, check=False)
        if wait_run.returncode != 0:
            wait_detail = (wait_run.stderr or wait_run.stdout or "").strip()
            if not wait_detail:
                wait_detail = f"exit status {wait_run.returncode}"
            raise BridgeError(f"`{' '.join(wait_cmd)}` failed: {wait_detail}")
        wait_raw = (wait_run.stdout or "").strip()
        wait_payload: Dict[str, Any] = {}
        if wait_raw:
            try:
                parsed_wait = json.loads(wait_raw)
                if isinstance(parsed_wait, dict):
                    wait_payload = parsed_wait
            except json.JSONDecodeError as err:
                raise BridgeError(
                    f"`{' '.join(wait_cmd)}` returned invalid JSON: {wait_raw[:160]}"
                ) from err

        expected_status = "approved" if decision == "approve" else "denied"
        wait_status = str(wait_payload.get("status", "")).strip().lower()
        if wait_status != expected_status:
            if wait_status in {"", "pending", "timeout"}:
                raise BridgeError(
                    f"approval `{approval_id}` still pending after CLI fallback "
                    f"(wait status `{wait_status or 'unknown'}`)"
                )
            raise BridgeError(
                f"approval `{approval_id}` resolved as `{wait_status}` "
                f"after CLI fallback; expected `{expected_status}`"
            )
        return {
            "status": "approved" if decision == "approve" else "denied",
            "via": "agent-ruler-cli",
        }

    def _register_pending_locked(self, approval_id: str) -> PendingApproval:
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
            suffix = len(self.pending_by_short) + 1
            short_id = f"AR{suffix:04d}"

        pending = PendingApproval(
            approval_id=approval_id,
            short_id=short_id,
            created_at=int(time.time()),
            notified=False,
        )
        self.pending_by_approval[pending.approval_id] = pending
        self.pending_by_short[pending.short_id] = pending
        return pending

    def _resolve_approval_reference(self, reference: str) -> Optional[str]:
        ref = reference.strip()
        if not ref:
            return None

        with self._lock:
            direct = self.pending_by_approval.get(ref)
            if direct is not None:
                return direct.approval_id

            short = normalize_short_id(ref)
            if short and short in self.pending_by_short:
                return self.pending_by_short[short].approval_id

        short = normalize_short_id(ref)
        if short:
            return None

        # If the operator entered full approval id manually, let the API validate it.
        if re.fullmatch(r"[A-Za-z0-9._:-]{4,200}", ref):
            return ref
        return None

    def _remove_pending(self, approval_id: str) -> None:
        with self._lock:
            pending = self.pending_by_approval.pop(approval_id, None)
            if pending is not None:
                self.pending_by_short.pop(pending.short_id, None)
                self.recently_resolved_short[pending.short_id] = (
                    int(time.time()) + self._recently_resolved_ttl_seconds
                )

    def _expire_pending(self, approval_id: str) -> None:
        self._remove_pending(approval_id)
        self.persist_state()

    def _prune_expired_pending(self) -> None:
        expired: List[str] = []
        with self._lock:
            for approval_id, pending in self.pending_by_approval.items():
                if self._is_expired(pending):
                    expired.append(approval_id)
        for approval_id in expired:
            self._remove_pending(approval_id)

    def _prune_recently_resolved_short(self) -> None:
        now = int(time.time())
        with self._lock:
            stale = [
                short
                for short, expires_at in self.recently_resolved_short.items()
                if expires_at <= now
            ]
            for short in stale:
                self.recently_resolved_short.pop(short, None)

    def _is_recently_resolved_reference(self, reference: str) -> bool:
        short = normalize_short_id(reference)
        if short is None:
            return False
        now = int(time.time())
        with self._lock:
            expires_at = self.recently_resolved_short.get(short)
            if expires_at is None:
                return False
            if expires_at <= now:
                self.recently_resolved_short.pop(short, None)
                return False
            return True

    def _is_expired(self, pending: PendingApproval) -> bool:
        return int(time.time()) > pending.created_at + self.config.decision_ttl_seconds

    def _notify_pending(self, event: Dict[str, Any], pending: PendingApproval) -> int:
        approval_id = pending.approval_id
        reason = str(event.get("reason_code", "approval_required"))
        category = str(event.get("category", "approval_required"))
        runner_id = optional_text(event.get("runner_id")) or "openclaw"
        runner_label = runner_display_label(runner_id)
        session_hint = str(event.get("session_hint", "")).strip()
        operation = str(event.get("operation", "")).strip()
        approval_detail: Dict[str, Any] = {}
        action_payload: Dict[str, Any] = {}
        detail_fetched = False
        should_fetch_detail = not operation
        if should_fetch_detail:
            detail_fetched = True
            try:
                approval_detail = self.client.approval(approval_id)
            except BridgeError as err:
                log_info(f"approval detail lookup failed for {approval_id}: {err}")
            action_raw = approval_detail.get("action")
            if isinstance(action_raw, dict):
                action_payload = action_raw
            if not operation:
                operation = str(action_payload.get("operation", "")).strip()
        if not approval_detail and not detail_fetched:
            try:
                approval_detail = self.client.approval(approval_id)
            except BridgeError:
                approval_detail = {}

        link = self._make_deep_link(str(event.get("open_in_webui", f"/approvals/{approval_id}")))
        reason_text = describe_approval_reason(reason, category)
        why = optional_text(approval_detail.get("why")) if isinstance(approval_detail, dict) else ""
        if why:
            reason_text = why.replace(" | ", " - ")
        context_rows = self._approval_context_rows(
            event=event,
            approval_detail=approval_detail,
            operation=operation,
        )
        category_label = self._approval_category_label(category)
        delivered = 0

        if not self.config.routes:
            log_info(
                "message deferred: "
                f"approval_id={approval_id} short_id={pending.short_id} reason=no_routes_configured"
            )
            return delivered

        for route in self.config.routes:
            buttons = None
            poll_note = ""
            message_thread_id = route.message_thread_id
            if route.channel == "telegram" and message_thread_id is None:
                message_thread_id = self._resolve_telegram_thread_id_for_pending(event, session_hint)
            self._maybe_send_preapproval_assistant_text(
                route=route,
                session_hint=session_hint,
                pending=pending,
                message_thread_id=message_thread_id,
                event=event,
                approval_detail=approval_detail,
            )

            if route.channel == "telegram" and route.telegram_inline_buttons:
                buttons = [
                    [
                        {
                            "text": "✅ Approve",
                            "callback_data": f"/arapprove {pending.short_id}",
                        },
                        {
                            "text": "🚫 Deny",
                            "callback_data": f"/ardeny {pending.short_id}",
                        },
                    ]
                ]

            if route.channel == "whatsapp" and route.whatsapp_use_poll:
                poll_question = f"Agent Ruler approval {pending.short_id}: approve or deny?"
                poll_options = [
                    f"approve {pending.short_id}",
                    f"deny {pending.short_id}",
                ]
                try:
                    self.messenger.send_poll(
                        channel=route.channel,
                        target=route.target,
                        account=route.account,
                        question=poll_question,
                        options=poll_options,
                    )
                    poll_note = (
                        f"WhatsApp poll sent. You can tap poll options, or reply with commands below."
                    )
                    log_info(
                        "message queued: "
                        f"approval_id={approval_id} short_id={pending.short_id} channel=whatsapp transport=poll"
                    )
                except BridgeError as err:
                    poll_note = (
                        f"WhatsApp poll unavailable ({err}). Use the quick commands below instead."
                    )
                    log_info(
                        "message send failed: "
                        f"approval_id={approval_id} short_id={pending.short_id} channel=whatsapp transport=poll reason={err}"
                    )

            heading = "🚨 *Approval required*"
            msg_lines: List[str] = []
            if route.channel == "telegram" and buttons is not None:
                # OpenClaw Telegram update sequencing uses callback message text
                # as part of its lane key. Keeping "/stop" on line 1 routes
                # approval callbacks into the control lane so button taps are
                # not blocked behind long-running agent turns in the same chat.
                msg_lines.append("/stop")
                msg_lines.append("")
            msg_lines.extend(
                [
                    heading,
                    "",
                    f"*Runner:* {runner_label}",
                    f"*Short ID:* `{pending.short_id}`",
                    "",
                    "*Approval ID:*",
                    f"`{approval_id}`",
                    "",
                    "*Reason:*",
                    reason_text,
                    "",
                    f"*Category:* {category_label}",
                    "*Status:* ⏳ Waiting for your decision. The runner is paused and will auto-resume after approval.",
                    f"*🔗 Open in Control Panel:* {link}",
                ]
            )
            if session_hint:
                msg_lines.insert(4, f"*Session:* {session_hint}")
            if context_rows:
                msg_lines.append("")
                for label, value in context_rows:
                    msg_lines.append(f"*{label}:*")
                    if isinstance(value, list):
                        msg_lines.extend([f"- {item}" for item in value])
                    else:
                        msg_lines.append(str(value))
                    msg_lines.append("")
                if msg_lines and not msg_lines[-1]:
                    msg_lines.pop()

            if poll_note:
                msg_lines.append("")
                msg_lines.append(f"🗳️ {poll_note}")
            elif buttons is not None:
                msg_lines.append("")
                msg_lines.append(
                    f"✅ Use the buttons below, or reply `approve {pending.short_id}` / `deny {pending.short_id}`."
                )
            else:
                msg_lines.append("")
                msg_lines.append(
                    f"Reply with `approve {pending.short_id}` or `deny {pending.short_id}`, or use the Control Panel link."
                )

            msg = "\n".join(msg_lines)
            log_info(
                "message queued: "
                f"approval_id={approval_id} short_id={pending.short_id} channel={route.channel} transport=text"
            )
            try:
                self.messenger.send_text(
                    channel=route.channel,
                    target=route.target,
                    account=route.account,
                    message=msg,
                    telegram_buttons=buttons,
                    message_thread_id=message_thread_id,
                )
                log_info(
                    "message sent: "
                    f"approval_id={approval_id} short_id={pending.short_id} channel={route.channel} "
                    f"thread_id={message_thread_id if message_thread_id is not None else 'none'} transport=text"
                )
                delivered += 1
            except BridgeError as err:
                log_info(
                    "message send failed: "
                    f"approval_id={approval_id} short_id={pending.short_id} channel={route.channel} transport=text reason={err}"
                )
        return delivered

    def _maybe_send_preapproval_assistant_text(
        self,
        *,
        route: RouteConfig,
        session_hint: str,
        pending: PendingApproval,
        message_thread_id: Optional[int],
        event: Dict[str, Any],
        approval_detail: Dict[str, Any],
    ) -> None:
        if route.channel != "telegram":
            return
        if route.telegram_streaming_enabled:
            return
        action_paths = self._approval_action_paths(event=event, approval_detail=approval_detail)
        session_key = session_hint.strip()
        if not session_key and not action_paths:
            return
        assistant_text = self._extract_preapproval_assistant_text(
            session_hint=session_key,
            cutoff_epoch_seconds=pending.created_at,
            action_paths=action_paths,
        )
        if not assistant_text:
            return
        action_paths_hint = "|".join(sorted(action_paths))
        dedupe_key = self._preapproval_assistant_key(
            channel=route.channel,
            target=route.target,
            lookup_hint=session_key or action_paths_hint or "none",
            message=assistant_text,
        )
        with self._lock:
            if dedupe_key in self._sent_preapproval_assistant_keys:
                return
            self._sent_preapproval_assistant_keys.add(dedupe_key)
        try:
            self.messenger.send_text(
                channel=route.channel,
                target=route.target,
                account=route.account,
                message=assistant_text,
                message_thread_id=message_thread_id,
            )
            log_info(
                "pre-approval assistant text sent: "
                f"session={session_key or 'none'} channel={route.channel} "
                f"thread_id={message_thread_id if message_thread_id is not None else 'none'}"
            )
        except BridgeError as err:
            log_info(
                "pre-approval assistant text send failed: "
                f"session={session_key or 'none'} channel={route.channel} reason={err}"
            )

    def _preapproval_assistant_key(
        self, *, channel: str, target: str, lookup_hint: str, message: str
    ) -> str:
        lookup_digest = hashlib.sha256(lookup_hint.encode("utf-8")).hexdigest()[:16]
        digest = hashlib.sha256(message.encode("utf-8")).hexdigest()[:24]
        return f"{channel}:{target}:{lookup_digest}:{digest}"

    def _extract_preapproval_assistant_text(
        self, *, session_hint: str, cutoff_epoch_seconds: int, action_paths: Set[str]
    ) -> str:
        if cutoff_epoch_seconds <= 0:
            return ""
        candidates = self._session_log_candidates(session_hint=session_hint)
        if not candidates:
            return ""
        cutoff = int(cutoff_epoch_seconds)
        for candidate in candidates:
            text = self._extract_preapproval_assistant_text_from_file(
                candidate,
                cutoff,
                action_paths=action_paths,
            )
            if text:
                return text
        return ""

    def _session_log_candidates(self, *, session_hint: str) -> List[Path]:
        openclaw_home = (self.config.openclaw_home or "").strip()
        if not openclaw_home:
            return []
        home_path = Path(openclaw_home)
        state_roots: List[Path] = []
        dot_state = home_path / ".openclaw"
        if dot_state.exists():
            state_roots.append(dot_state)
        if home_path.exists():
            state_roots.append(home_path)

        matches: List[Path] = []
        for root in state_roots:
            agents_dir = root / "agents"
            if not agents_dir.exists():
                continue
            for sessions_dir in agents_dir.glob("*/sessions"):
                if not sessions_dir.is_dir():
                    continue
                if session_hint:
                    matches.extend(sorted(sessions_dir.glob(f"{session_hint}*.jsonl")))
                else:
                    matches.extend(sorted(sessions_dir.glob("*.jsonl")))

        deduped: Dict[str, Path] = {}
        for path in matches:
            deduped[str(path)] = path
        ordered = list(deduped.values())
        ordered.sort(key=lambda path: path.stat().st_mtime if path.exists() else 0, reverse=True)
        if not session_hint:
            return ordered[:50]
        return ordered

    def _approval_action_paths(
        self, *, event: Dict[str, Any], approval_detail: Dict[str, Any]
    ) -> Set[str]:
        action = approval_detail.get("action")
        action_payload = action if isinstance(action, dict) else {}
        metadata = (
            action_payload.get("metadata")
            if isinstance(action_payload.get("metadata"), dict)
            else {}
        )
        values = [
            event.get("path"),
            event.get("secondary_path"),
            event.get("resolved_src"),
            event.get("resolved_dst"),
            approval_detail.get("resolved_src"),
            approval_detail.get("resolved_dst"),
            action_payload.get("path"),
            action_payload.get("secondary_path"),
            metadata.get("export_src"),
            metadata.get("export_dst"),
            metadata.get("import_src"),
            metadata.get("import_dst"),
        ]
        return {text for raw in values if (text := optional_text(raw))}

    def _extract_preapproval_assistant_text_from_file(
        self,
        session_path: Path,
        cutoff_epoch_seconds: int,
        *,
        action_paths: Set[str],
    ) -> str:
        latest_user_ts: Optional[int] = None
        first_assistant_text: Optional[str] = None
        matches_action_path = not action_paths
        try:
            with session_path.open("r", encoding="utf-8") as handle:
                for raw_line in handle:
                    line = raw_line.strip()
                    if not line:
                        continue
                    if not matches_action_path and any(path in line for path in action_paths):
                        matches_action_path = True
                    try:
                        event = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    if event.get("type") != "message":
                        continue
                    message = event.get("message")
                    if not isinstance(message, dict):
                        continue
                    role = str(message.get("role", "")).strip().lower()
                    timestamp_value = str(event.get("timestamp", "")).strip()
                    event_ts = parse_iso8601_epoch(timestamp_value)
                    if event_ts is None or event_ts > cutoff_epoch_seconds:
                        continue
                    if role == "user":
                        latest_user_ts = event_ts
                        first_assistant_text = None
                        continue
                    if role != "assistant" or latest_user_ts is None:
                        continue
                    if event_ts < latest_user_ts:
                        continue
                    assistant_text = extract_assistant_text(message)
                    if not assistant_text:
                        continue
                    if first_assistant_text is None:
                        first_assistant_text = assistant_text
        except OSError:
            return ""
        if not matches_action_path:
            return ""
        return first_assistant_text or ""

    def _approval_context_rows(
        self,
        *,
        event: Dict[str, Any],
        approval_detail: Dict[str, Any],
        operation: str,
    ) -> List[Tuple[str, Any]]:
        action = approval_detail.get("action") if isinstance(approval_detail.get("action"), dict) else {}
        action_metadata = action.get("metadata") if isinstance(action.get("metadata"), dict) else {}
        event_metadata = event.get("metadata") if isinstance(event.get("metadata"), dict) else {}
        operation_value = optional_text(operation or action.get("operation") or event.get("operation"))
        category = optional_text(event.get("category")).lower()
        transfer_like = (
            category in TRANSFER_APPROVAL_CATEGORIES or operation_value in TRANSFER_APPROVAL_OPERATIONS
        )

        source_path = self._first_path_value(
            [
                approval_detail.get("resolved_src"),
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
                approval_detail.get("resolved_dst"),
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

        targets = self._collect_approval_targets(event, approval_detail)
        display_targets: List[str] = []
        seen: Set[str] = set()
        for target in targets:
            rendered = self._alias_runtime_path(target)
            if not rendered or rendered in seen:
                continue
            seen.add(rendered)
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
            preview = extras[:MAX_APPROVAL_CONTEXT_PREVIEW]
            remaining = len(extras) - len(preview)
            if remaining > 0:
                preview.append(f"… and {remaining} more")
            rows.append(("Context paths", preview))

        if operation_value:
            rows.append(("Operation", operation_value.replace("_", " ")))
        return rows

    def _collect_approval_targets(
        self,
        event: Dict[str, Any],
        approval_detail: Dict[str, Any],
    ) -> List[str]:
        targets: List[str] = []
        seen: Set[str] = set()

        def add(value: Any) -> None:
            raw = optional_text(value)
            if not raw or raw in seen:
                return
            seen.add(raw)
            targets.append(raw)

        add(approval_detail.get("resolved_src"))
        add(approval_detail.get("resolved_dst"))
        action = approval_detail.get("action") if isinstance(approval_detail.get("action"), dict) else {}
        add(action.get("path"))
        add(action.get("secondary_path"))
        metadata = action.get("metadata") if isinstance(action.get("metadata"), dict) else {}
        for key in ("export_src", "export_dst", "import_src", "import_dst", "src", "dst", "target_path"):
            add(metadata.get(key))
        add(event.get("resolved_src"))
        add(event.get("resolved_dst"))
        add(event.get("path"))
        add(event.get("secondary_path"))
        event_metadata = event.get("metadata") if isinstance(event.get("metadata"), dict) else {}
        for key in ("export_src", "export_dst", "import_src", "import_dst", "src", "dst", "target_path"):
            add(event_metadata.get(key))
        return targets

    def _first_path_value(self, values: Sequence[Any], exclude: Optional[Set[str]] = None) -> str:
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

        runtime_root = self.config.runtime_dir.expanduser() if self.config.runtime_dir else None
        if path.is_absolute() and runtime_root is not None:
            workspace_root = runtime_root / "user_data" / "runners" / "openclaw" / "workspace"
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

    def _resolve_telegram_thread_id_for_pending(
        self, event: Dict[str, Any], session_hint: str
    ) -> Optional[int]:
        direct = self._parse_positive_int(
            event.get("message_thread_id") or event.get("messageThreadId") or event.get("thread_id")
        )
        if direct is not None:
            return direct

        metadata = event.get("metadata")
        if isinstance(metadata, dict):
            candidate = self._parse_positive_int(
                metadata.get("message_thread_id")
                or metadata.get("messageThreadId")
                or metadata.get("thread_id")
                or metadata.get("threadId")
            )
            if candidate is not None:
                return candidate

        hinted = self._thread_id_from_text(session_hint)
        if hinted is not None:
            return hinted

        return self._active_openclaw_session_thread_id()

    @staticmethod
    def _parse_positive_int(value: Any) -> Optional[int]:
        if isinstance(value, int) and value > 0:
            return value
        if isinstance(value, str):
            stripped = value.strip()
            if stripped.isdigit():
                parsed = int(stripped)
                if parsed > 0:
                    return parsed
        return None

    @staticmethod
    def _thread_id_from_text(value: str) -> Optional[int]:
        raw = (value or "").strip()
        if not raw:
            return None
        match = re.search(r"(?:topic|thread)[:#]([1-9][0-9]*)", raw, flags=re.IGNORECASE)
        if not match:
            return None
        return int(match.group(1))

    def _active_openclaw_session_thread_id(self) -> Optional[int]:
        if not self.config.openclaw_home:
            return None
        sessions_path = (
            Path(self.config.openclaw_home).expanduser()
            / ".openclaw"
            / "agents"
            / "main"
            / "sessions"
            / "sessions.json"
        )
        try:
            payload = json.loads(sessions_path.read_text(encoding="utf-8"))
        except Exception:
            return None
        if not isinstance(payload, dict):
            return None

        preferred = payload.get("agent:main:main")
        candidates: List[Dict[str, Any]] = []
        if isinstance(preferred, dict):
            candidates.append(preferred)
        candidates.extend(item for item in payload.values() if isinstance(item, dict))

        for entry in candidates:
            session_id = str(entry.get("sessionId", "")).strip()
            from_session = self._thread_id_from_text(session_id)
            if from_session is not None:
                return from_session
            last_thread = self._parse_positive_int(entry.get("lastThreadId"))
            if last_thread is not None:
                return last_thread
            delivery = entry.get("deliveryContext")
            if isinstance(delivery, dict):
                delivery_thread = self._parse_positive_int(delivery.get("threadId"))
                if delivery_thread is not None:
                    return delivery_thread
        return None

    def _make_deep_link(self, path: str) -> str:
        cleaned = path.strip() or "/approvals"
        if cleaned.startswith("http://") or cleaned.startswith("https://"):
            return cleaned
        if not cleaned.startswith("/"):
            cleaned = f"/{cleaned}"
        return f"{self.config.public_base_url.rstrip('/')}{cleaned}"

    def _reply(self, inbound: Dict[str, str], message: str, route: RouteConfig) -> None:
        reply_target = resolve_reply_target(inbound)
        # Extract thread_id from inbound if available for replies
        message_thread_id = None
        if inbound.get("message_thread_id"):
            try:
                message_thread_id = int(inbound["message_thread_id"])
            except (ValueError, TypeError):
                pass
        self.messenger.send_text(
            channel=inbound["channel"],
            target=reply_target,
            account=inbound.get("account") or route.account,
            message=message,
            message_thread_id=message_thread_id,
        )

    def _emit_feedback(
        self,
        inbound: Dict[str, str],
        message: str,
        route: RouteConfig,
    ) -> Optional[str]:
        suppressed = inbound.get("suppress_channel_reply") == "true"
        if not suppressed:
            self._reply(inbound, message, route)
            return None

        # In sync hook mode, prefer direct channel feedback so button clicks
        # show a visible confirmation immediately. If channel send fails, fall
        # back to hook-level feedback text injection.
        try:
            self._reply(inbound, message, route)
            return None
        except BridgeError as err:
            log_info(
                "inbound feedback direct send failed; using hook fallback: "
                f"channel={inbound.get('channel', '')} sender={inbound.get('sender', '')} reason={err}"
            )
            return message


def normalize_inbound_event(payload: Dict[str, Any]) -> Optional[Dict[str, str]]:
    if not isinstance(payload, dict):
        return None

    channel = normalize_channel(str(payload.get("channelId") or payload.get("channel") or ""))
    if not channel:
        return None

    raw_sender = str(payload.get("from") or "").strip()
    metadata = payload.get("metadata") if isinstance(payload.get("metadata"), dict) else {}
    sender = normalize_sender(channel, raw_sender, metadata)
    if not sender:
        return None

    content = str(payload.get("content") or "").strip()
    if not content:
        content = extract_command_hint_from_metadata(metadata)
    if not content:
        return None

    account = str(payload.get("accountId") or payload.get("account") or "").strip()
    conversation = str(payload.get("conversationId") or "").strip()
    suppress_channel_reply = bool(payload.get("suppress_channel_reply", False))
    message_id = str(payload.get("messageId") or payload.get("message_id") or "").strip()

    # Extract thread information from metadata for Telegram
    message_thread_id = None
    callback_query_id = ""
    if channel == "telegram":
        thread_raw = (
            metadata.get("message_thread_id")
            or metadata.get("messageThreadId")
            or metadata.get("threadId")
        )
        if isinstance(thread_raw, int) and thread_raw > 0:
            message_thread_id = thread_raw
        elif isinstance(thread_raw, str) and thread_raw.strip().isdigit():
            try:
                message_thread_id = int(thread_raw.strip())
            except ValueError:
                pass

        callback_query_id = extract_callback_query_id_from_metadata(metadata)
        if (
            not callback_query_id
            and message_id
            and should_fallback_callback_query_id(content, metadata)
        ):
            callback_query_id = message_id
            log_info(
                "callback_query_id fallback applied from messageId "
                f"for telegram decision callback: message_id={message_id}"
            )

    result = {
        "channel": channel,
        "sender": sender,
        "content": content,
        "account": account,
        "conversation_id": conversation,
        "suppress_channel_reply": "true" if suppress_channel_reply else "false",
    }
    if message_thread_id is not None:
        result["message_thread_id"] = str(message_thread_id)
    if callback_query_id:
        result["callback_query_id"] = callback_query_id

    return result


def normalize_channel(channel: str) -> str:
    return channel.strip().lower()


def normalize_sender(channel: str, sender: str, metadata: Dict[str, Any]) -> str:
    sender = sender.strip()
    if channel == "telegram":
        if not sender:
            sender = str(
                metadata.get("senderId")
                or metadata.get("sender_id")
                or metadata.get("fromId")
                or metadata.get("from_id")
                or metadata.get("userId")
                or metadata.get("user_id")
                or metadata.get("telegramUserId")
                or ""
            ).strip()
        s = sender.lower()
        if s.startswith("telegram:"):
            s = s.split(":", 1)[1]
        if s.startswith("tg:"):
            s = s.split(":", 1)[1]
        return s

    if channel == "whatsapp":
        e164 = str(metadata.get("senderE164") or "").strip()
        if e164:
            return e164
        if "@" in sender:
            base = sender.split("@", 1)[0]
            if base.isdigit():
                return f"+{base}"
        return sender

    if channel == "discord":
        s = sender.lower()
        if s.startswith("user:"):
            return s
        if s.startswith("<@") and s.endswith(">"):
            cleaned = s[2:-1].lstrip("!")
            if cleaned.isdigit():
                return f"user:{cleaned}"
        if s.isdigit():
            return f"user:{s}"
        return s

    return sender


def sender_allowed(route: RouteConfig, sender: str) -> bool:
    allow = [item for item in route.allow_from if item]
    if not allow:
        return False
    if "*" in allow:
        return True
    return sender in allow


def select_route(routes: Sequence[RouteConfig], inbound: Dict[str, str]) -> Optional[RouteConfig]:
    for route in routes:
        if route.channel != inbound["channel"]:
            continue
        if sender_allowed(route, inbound["sender"]):
            return route
    return None


def resolve_reply_target(inbound: Dict[str, str]) -> str:
    channel = inbound["channel"]
    conversation = inbound.get("conversation_id", "").strip()
    if conversation:
        if channel == "discord":
            lower = conversation.lower()
            if lower.startswith("channel:") or lower.startswith("user:"):
                return lower
            if conversation.isdigit():
                return f"channel:{conversation}"
        return conversation

    sender = inbound["sender"]
    if channel == "discord":
        if sender.startswith("user:"):
            return sender
        if sender.isdigit():
            return f"user:{sender}"
    return sender


def normalize_short_id(value: str) -> Optional[str]:
    candidate = value.strip().upper()
    if not candidate:
        return None
    if not _SHORT_ID_PATTERN.match(candidate):
        return None
    return candidate


def extract_command_hint_from_metadata(metadata: Dict[str, Any]) -> str:
    if not metadata:
        return ""

    direct_keys = [
        "callback_data",
        "callbackData",
        "callback",
        "buttonData",
        "button_data",
        "data",
        "pollOption",
        "poll_option",
        "selectedOption",
        "selected_option",
        "vote",
        "answer",
        "text",
    ]
    for key in direct_keys:
        value = metadata.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
        if isinstance(value, dict):
            nested = extract_command_hint_from_metadata(value)
            if nested:
                return nested

    poll_value = metadata.get("poll")
    if isinstance(poll_value, dict):
        for key in direct_keys:
            value = poll_value.get(key)
            if isinstance(value, str) and value.strip():
                return value.strip()

    for container_key in ["interaction", "telegram", "callbackQuery", "callback_query"]:
        nested_obj = metadata.get(container_key)
        if isinstance(nested_obj, dict):
            nested = extract_command_hint_from_metadata(nested_obj)
            if nested:
                return nested

    return ""


def extract_callback_payload_from_metadata(metadata: Dict[str, Any]) -> str:
    if not metadata:
        return ""

    direct_keys = [
        "callback_data",
        "callbackData",
        "callback",
        "buttonData",
        "button_data",
        "data",
    ]
    for key in direct_keys:
        value = metadata.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
        if isinstance(value, dict):
            nested = extract_callback_payload_from_metadata(value)
            if nested:
                return nested

    for container_key in ["interaction", "telegram", "callbackQuery", "callback_query"]:
        nested_obj = metadata.get(container_key)
        if isinstance(nested_obj, dict):
            nested = extract_callback_payload_from_metadata(nested_obj)
            if nested:
                return nested

    return ""


def has_callback_interaction_metadata(metadata: Dict[str, Any]) -> bool:
    if not metadata:
        return False

    if extract_callback_payload_from_metadata(metadata):
        return True

    callback_obj = metadata.get("callbackQuery")
    if isinstance(callback_obj, dict) and callback_obj:
        return True
    callback_obj = metadata.get("callback_query")
    if isinstance(callback_obj, dict) and callback_obj:
        return True

    action = str(metadata.get("event_action") or "").strip().lower()
    if "callback" in action or "button" in action:
        return True

    return False


def should_fallback_callback_query_id(content: str, metadata: Dict[str, Any]) -> bool:
    if has_callback_interaction_metadata(metadata):
        return True

    lowered = content.strip().lower()
    if not lowered:
        return False
    if lowered.startswith("callback_data:"):
        return True
    if lowered.startswith("ar:approve:") or lowered.startswith("ar:deny:"):
        return True
    if lowered.startswith("/arapprove ") or lowered.startswith("/ardeny "):
        return True
    return False


def extract_callback_query_id_from_metadata(metadata: Dict[str, Any]) -> str:
    if not metadata:
        return ""

    direct_keys = [
        "callback_query_id",
        "callbackQueryId",
        "callback_id",
        "callbackId",
        "query_id",
        "queryId",
    ]
    for key in direct_keys:
        value = metadata.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
        if isinstance(value, int):
            return str(value)

    for container_key in ["callbackQuery", "callback_query", "interaction", "telegram"]:
        nested = metadata.get(container_key)
        if not isinstance(nested, dict):
            continue
        for key in ["id", *direct_keys]:
            value = nested.get(key)
            if isinstance(value, str) and value.strip():
                return value.strip()
            if isinstance(value, int):
                return str(value)
        nested_found = extract_callback_query_id_from_metadata(nested)
        if nested_found:
            return nested_found

    return ""


def parse_decision_command(text: str) -> Optional[ParsedDecisionCommand]:
    raw = text.strip()
    if not raw:
        return None

    seen: Set[str] = set()
    candidates: List[str] = [raw]
    if "\n" in raw or "\r" in raw:
        for line in raw.splitlines():
            stripped = line.strip()
            if stripped:
                candidates.append(stripped)

    for candidate in candidates:
        key = candidate.lower()
        if key in seen:
            continue
        seen.add(key)

        lower = candidate.lower()
        if lower.startswith("callback_data:"):
            payload = candidate.split(":", 1)[1].strip()
            parsed = parse_callback_payload(payload)
            if parsed is not None:
                return parsed

        callback = parse_callback_payload(candidate)
        if callback is not None:
            return callback

        match = _COMMAND_PATTERN.match(candidate)
        if match:
            decision = match.group(1).lower()
            reference = match.group(2).strip()
            short = normalize_short_id(reference)
            if short is not None:
                reference = short
            return ParsedDecisionCommand(decision=decision, reference=reference)

        ar_match = _AR_COMMAND_PATTERN.match(candidate)
        if ar_match:
            decision = ar_match.group(1).lower()
            reference = ar_match.group(2).strip()
            short = normalize_short_id(reference)
            if short is not None:
                reference = short
            return ParsedDecisionCommand(decision=decision, reference=reference)

    return None


def parse_callback_payload(payload: str) -> Optional[ParsedDecisionCommand]:
    match = _CALLBACK_PATTERN.match(payload.strip())
    if not match:
        return None
    reference = match.group(2).strip()
    short = normalize_short_id(reference)
    if short is not None:
        reference = short
    return ParsedDecisionCommand(decision=match.group(1).lower(), reference=reference)


def parse_routes(routes_raw: Any) -> List[RouteConfig]:
    if not isinstance(routes_raw, list) or not routes_raw:
        raise BridgeError("bridge routes must be a non-empty array")

    routes: List[RouteConfig] = []
    for item in routes_raw:
        if not isinstance(item, dict):
            raise BridgeError(f"invalid route entry: {item!r}")
        channel = normalize_channel(str(item.get("channel", "")))
        if channel not in SUPPORTED_CHANNELS:
            raise BridgeError(f"unsupported channel in route: {channel}")

        target = str(item.get("target", "")).strip()
        if not target:
            raise BridgeError(f"route target missing for channel {channel}")

        message_thread_id: Optional[int] = None
        if channel == "telegram":
            thread_raw = item.get("message_thread_id")
            if thread_raw is None:
                thread_raw = item.get("messageThreadId")
            if isinstance(thread_raw, int) and thread_raw > 0:
                message_thread_id = thread_raw
            elif isinstance(thread_raw, str) and thread_raw.strip().isdigit():
                parsed = int(thread_raw.strip())
                if parsed > 0:
                    message_thread_id = parsed

            # Backward-compatible support for `target=chat_id#thread_id`.
            if message_thread_id is None and "#" in target:
                chat_id, _, raw_thread = target.partition("#")
                candidate = raw_thread.strip()
                if chat_id.strip() and candidate.isdigit():
                    parsed = int(candidate)
                    if parsed > 0:
                        target = chat_id.strip()
                        message_thread_id = parsed

        allow_from = item.get("allow_from")
        if not isinstance(allow_from, list) or not allow_from:
            raise BridgeError(f"route allow_from must be non-empty for channel {channel}")
        normalized_allow = [normalize_sender(channel, str(value), {}) for value in allow_from]

        routes.append(
            RouteConfig(
                channel=channel,
                target=target,
                allow_from=normalized_allow,
                account=str(item.get("account", "")).strip() or None,
                telegram_inline_buttons=bool(item.get("telegram_inline_buttons", False)),
                telegram_streaming_enabled=bool(
                    item.get(
                        "telegram_streaming_enabled",
                        item.get("telegramStreamingEnabled", False),
                    )
                ),
                whatsapp_use_poll=bool(item.get("whatsapp_use_poll", channel == "whatsapp")),
                message_thread_id=message_thread_id,
            )
        )

    return routes


def parse_routes_optional(routes_raw: Any) -> List[RouteConfig]:
    if routes_raw in (None, []):
        return []
    return parse_routes(routes_raw)


def route_signature(
    routes: Sequence[RouteConfig],
) -> Tuple[Tuple[str, str, str, Optional[int], Tuple[str, ...]], ...]:
    normalized = []
    for route in routes:
        normalized.append(
            (
                route.channel,
                route.account or "default",
                route.target,
                route.message_thread_id,
                tuple(sorted(route.allow_from)),
            )
        )
    return tuple(sorted(normalized))


def _route_slot_from_raw(route: Dict[str, Any]) -> Optional[Tuple[str, str, str, Optional[int]]]:
    if not isinstance(route, dict):
        return None
    channel = normalize_channel(str(route.get("channel", "")))
    target = str(route.get("target", "")).strip()
    account = str(route.get("account", "")).strip() or "default"
    message_thread_id: Optional[int] = None
    if channel == "telegram":
        thread_raw = route.get("message_thread_id")
        if thread_raw is None:
            thread_raw = route.get("messageThreadId")
        if isinstance(thread_raw, int) and thread_raw > 0:
            message_thread_id = thread_raw
        elif isinstance(thread_raw, str) and thread_raw.strip().isdigit():
            parsed = int(thread_raw.strip())
            if parsed > 0:
                message_thread_id = parsed

        # Backward-compatible support for `target=chat_id#thread_id`.
        if message_thread_id is None and "#" in target:
            chat_id, _, raw_thread = target.partition("#")
            candidate = raw_thread.strip()
            if chat_id.strip() and candidate.isdigit():
                parsed = int(candidate)
                if parsed > 0:
                    target = chat_id.strip()
                    message_thread_id = parsed

    if not channel or not target:
        return None
    return (channel, account, target, message_thread_id)


def merge_route_documents(
    managed_routes: Sequence[Dict[str, Any]], defaults_routes: Sequence[Dict[str, Any]]
) -> List[Dict[str, Any]]:
    merged: List[Dict[str, Any]] = [dict(route) for route in managed_routes if isinstance(route, dict)]
    defaults_by_slot: Dict[Tuple[str, str, str, Optional[int]], Dict[str, Any]] = {}
    for route in defaults_routes:
        if not isinstance(route, dict):
            continue
        slot = _route_slot_from_raw(route)
        if slot is None:
            continue
        defaults_by_slot[slot] = dict(route)

    for route in merged:
        slot = _route_slot_from_raw(route)
        if slot is None:
            continue
        default_route = defaults_by_slot.get(slot)
        if not default_route:
            continue
        if normalize_channel(str(route.get("channel", ""))) != "telegram":
            continue
        default_streaming = bool(
            default_route.get(
                "telegram_streaming_enabled",
                default_route.get("telegramStreamingEnabled", False),
            )
        )
        route["telegram_streaming_enabled"] = default_streaming

    existing_slots: Set[Tuple[str, str, str, Optional[int]]] = set()
    for route in merged:
        slot = _route_slot_from_raw(route)
        if slot is not None:
            existing_slots.add(slot)

    for route in defaults_routes:
        if not isinstance(route, dict):
            continue
        slot = _route_slot_from_raw(route)
        if slot is None or slot in existing_slots:
            continue
        merged.append(dict(route))
        existing_slots.add(slot)
    return merged


def write_routes_to_openclaw_config(
    *,
    openclaw_bin: str,
    openclaw_home: Optional[str],
    routes: Sequence[Dict[str, Any]],
) -> None:
    if not routes:
        return
    payload = json.dumps(list(routes), separators=(",", ":"))
    cmd = [
        openclaw_bin,
        "config",
        "set",
        OPENCLAW_BRIDGE_ROUTES_POINTER,
        payload,
        "--json",
    ]
    env = os.environ.copy()
    if openclaw_home:
        env["OPENCLAW_HOME"] = openclaw_home
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=False, env=env)
    except OSError as err:
        raise BridgeError(
            "failed to execute OpenClaw CLI while writing bridge routes: "
            f"`{' '.join(cmd)}` ({err})"
        ) from err
    if result.returncode != 0:
        stderr = (result.stderr or "").strip() or "unknown error"
        raise BridgeError(
            "failed to persist auto-discovered routes via "
            f"`{' '.join(cmd)}`: {stderr}"
        )


def resolve_openclaw_routes(
    *,
    openclaw_bin: str,
    openclaw_home: Optional[str],
    allow_persist: bool = True,
) -> Tuple[str, List[RouteConfig], bool]:
    managed_raw = read_routes_from_openclaw_config(
        openclaw_bin=openclaw_bin,
        openclaw_home=openclaw_home,
    )
    if managed_raw in (None, []):
        managed_docs: List[Dict[str, Any]] = []
    elif isinstance(managed_raw, list):
        managed_docs = [dict(item) for item in managed_raw if isinstance(item, dict)]
    else:
        raise BridgeError(
            f"expected list for `{OPENCLAW_BRIDGE_ROUTES_POINTER}`, got {type(managed_raw).__name__}"
        )

    channels = read_openclaw_channels_config(
        openclaw_bin=openclaw_bin,
        openclaw_home=openclaw_home,
    )
    allow_from_entries = read_allow_from_entries(openclaw_home)
    defaults_docs = discover_routes_from_channel_defaults(channels, allow_from_entries)

    merged_docs = merge_route_documents(managed_docs, defaults_docs)
    synced = False
    if allow_persist and merged_docs and merged_docs != managed_docs:
        write_routes_to_openclaw_config(
            openclaw_bin=openclaw_bin,
            openclaw_home=openclaw_home,
            routes=merged_docs,
        )
        managed_docs = merged_docs
        synced = True

    if managed_docs:
        return ("openclaw_managed_config", parse_routes(managed_docs), synced)
    if defaults_docs:
        return ("openclaw_channel_defaults", parse_routes(defaults_docs), synced)
    return ("openclaw_unconfigured", [], synced)


def discover_managed_openclaw_home(agent_ruler_bin: str) -> Optional[str]:
    cmd = [agent_ruler_bin, "status", "--json"]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=False)
    except OSError:
        return None
    if result.returncode != 0:
        return None
    try:
        payload = json.loads(result.stdout.strip() or "{}")
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    runner = payload.get("runner")
    if not isinstance(runner, dict):
        return None
    if str(runner.get("kind", "")).strip().lower() != "openclaw":
        return None
    managed_home = str(runner.get("managed_home", "")).strip()
    if not managed_home:
        return None
    return managed_home


def read_routes_from_openclaw_config(
    *,
    openclaw_bin: str,
    openclaw_home: Optional[str],
) -> Any:
    cmd = [
        openclaw_bin,
        "config",
        "get",
        OPENCLAW_BRIDGE_ROUTES_POINTER,
        "--json",
    ]
    env = os.environ.copy()
    if openclaw_home:
        env["OPENCLAW_HOME"] = openclaw_home
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=False, env=env)
    except OSError as err:
        raise BridgeError(
            "failed to execute OpenClaw CLI while loading bridge routes: "
            f"`{' '.join(cmd)}` ({err})"
        ) from err
    if result.returncode != 0:
        stderr = (result.stderr or "").strip() or "unknown error"
        if "config path not found" in stderr.lower():
            return None
        raise BridgeError(
            "failed to read OpenClaw bridge routes via "
            f"`{' '.join(cmd)}`: {stderr}"
        )
    raw = result.stdout.strip()
    if not raw:
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError as err:
        raise BridgeError(
            f"invalid JSON from `{' '.join(cmd)}`: {raw[:160]}"
        ) from err


def read_openclaw_channels_config(
    *,
    openclaw_bin: str,
    openclaw_home: Optional[str],
) -> Dict[str, Any]:
    cmd = [openclaw_bin, "config", "get", "channels", "--json"]
    env = os.environ.copy()
    if openclaw_home:
        env["OPENCLAW_HOME"] = openclaw_home
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=False, env=env)
    except OSError as err:
        raise BridgeError(
            "failed to execute OpenClaw CLI while loading channels config: "
            f"`{' '.join(cmd)}` ({err})"
        ) from err
    if result.returncode != 0:
        stderr = (result.stderr or "").strip() or "unknown error"
        if "config path not found" in stderr.lower():
            return {}
        raise BridgeError(
            f"failed to read OpenClaw channels config via `{' '.join(cmd)}`: {stderr}"
        )
    raw = result.stdout.strip()
    if not raw:
        return {}
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as err:
        raise BridgeError(
            f"invalid JSON from `{' '.join(cmd)}`: {raw[:160]}"
        ) from err
    if not isinstance(parsed, dict):
        return {}
    return parsed


def read_allow_from_entries(openclaw_home: Optional[str]) -> Dict[str, Dict[str, List[str]]]:
    if not openclaw_home:
        return {}
    credentials_dir = Path(openclaw_home).expanduser() / ".openclaw" / "credentials"
    if not credentials_dir.exists():
        return {}

    collected: Dict[str, Dict[str, List[str]]] = {}
    for file_path in credentials_dir.glob("*-allowFrom.json"):
        name = file_path.name
        if not name.endswith("-allowFrom.json"):
            continue
        base = name[: -len("-allowFrom.json")]
        if "-" in base:
            channel, account = base.split("-", 1)
        else:
            channel, account = base, "default"
        channel = channel.strip().lower()
        account = account.strip() or "default"
        if channel not in SUPPORTED_CHANNELS:
            continue

        try:
            parsed = json.loads(file_path.read_text(encoding="utf-8"))
        except Exception:
            continue
        allow_raw = parsed.get("allowFrom") if isinstance(parsed, dict) else None
        if not isinstance(allow_raw, list):
            continue

        values = [str(item).strip() for item in allow_raw if str(item).strip()]
        if not values:
            continue
        bucket = collected.setdefault(channel, {})
        bucket[account] = sorted(set(values))

    return collected


def discover_routes_from_channel_defaults(
    channels: Dict[str, Any],
    allow_from: Dict[str, Dict[str, List[str]]],
) -> List[Dict[str, Any]]:
    routes: List[Dict[str, Any]] = []
    seen: Set[Tuple[str, str, str]] = set()
    for channel in sorted(SUPPORTED_CHANNELS):
        cfg = channels.get(channel)
        if not isinstance(cfg, dict):
            continue
        if not bool(cfg.get("enabled")):
            continue

        accounts: Dict[str, List[str]] = {
            account: list(values)
            for account, values in allow_from.get(channel, {}).items()
            if isinstance(account, str) and isinstance(values, list)
        }
        cfg_allow_from = cfg.get("allowFrom")
        if isinstance(cfg_allow_from, list):
            config_values = sorted(
                {
                    str(item).strip()
                    for item in cfg_allow_from
                    if str(item).strip()
                }
            )
            if config_values:
                existing = accounts.get("default", [])
                accounts["default"] = sorted(set(existing) | set(config_values))
        for account, senders in accounts.items():
            for sender in senders:
                key = (channel, account, sender)
                if key in seen:
                    continue
                seen.add(key)
                route: Dict[str, Any] = {
                    "channel": channel,
                    "target": sender,
                    "allow_from": [sender],
                }
                if account:
                    route["account"] = account
                if channel == "telegram":
                    route["telegram_inline_buttons"] = True
                    route["telegram_streaming_enabled"] = bool(cfg.get("streaming", False))
                if channel == "whatsapp":
                    route["whatsapp_use_poll"] = True
                routes.append(route)
    return routes


def load_config(path: Path, args: argparse.Namespace) -> BridgeConfig:
    raw = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        raise BridgeError(f"invalid config file: {path}")

    openclaw_bin = str(getattr(args, "openclaw_bin", None) or raw.get("openclaw_bin") or "openclaw")
    agent_ruler_bin = str(
        getattr(args, "agent_ruler_bin", None)
        or raw.get("agent_ruler_bin")
        or DEFAULT_AGENT_RULER_BIN
    )
    openclaw_home = (
        str(getattr(args, "openclaw_home", None) or raw.get("openclaw_home") or "").strip()
        or None
    )
    if openclaw_home is None:
        openclaw_home = (os.environ.get("OPENCLAW_HOME") or "").strip() or None
    if openclaw_home is None:
        openclaw_home = discover_managed_openclaw_home(agent_ruler_bin)

    routes_raw = raw.get("routes")
    routes_source = "bridge_config"
    if routes_raw in (None, []):
        routes_source, routes, synced = resolve_openclaw_routes(
            openclaw_bin=openclaw_bin,
            openclaw_home=openclaw_home,
            allow_persist=True,
        )
        if synced:
            log_info(
                "routes auto-synced into OpenClaw config at "
                f"`{OPENCLAW_BRIDGE_ROUTES_POINTER}`."
            )
    else:
        routes = parse_routes_optional(routes_raw)

    state_file = Path(str(args.state_file or raw.get("state_file") or DEFAULT_STATE_FILE)).expanduser()
    runtime_dir_raw = str(raw.get("runtime_dir") or "").strip()
    runtime_dir = Path(runtime_dir_raw).expanduser() if runtime_dir_raw else derive_runtime_dir(state_file)

    poll_interval = int(args.poll_interval or raw.get("poll_interval_seconds") or 8)
    decision_ttl = int(raw.get("decision_ttl_seconds") or DEFAULT_DECISION_TTL_SECONDS)
    short_id_length = int(raw.get("short_id_length") or DEFAULT_SHORT_ID_LENGTH)
    telegram_typing_keepalive = bool(raw.get("telegram_typing_keepalive", True))
    telegram_typing_interval_seconds = int(
        raw.get("telegram_typing_interval_seconds") or DEFAULT_TELEGRAM_TYPING_KEEPALIVE_SECONDS
    )

    ruler_url = str(args.ruler_url or raw.get("ruler_url") or DEFAULT_RULER_URL).strip()
    public_base_url = str(args.public_base_url or raw.get("public_base_url") or ruler_url).strip()

    inbound_bind = str(args.inbound_bind or raw.get("inbound_bind") or DEFAULT_BRIDGE_BIND)

    return BridgeConfig(
        ruler_url=ruler_url,
        public_base_url=public_base_url,
        poll_interval_seconds=max(1, poll_interval),
        decision_ttl_seconds=max(60, decision_ttl),
        inbound_bind=inbound_bind,
        state_file=state_file,
        openclaw_bin=openclaw_bin,
        openclaw_home=openclaw_home,
        agent_ruler_bin=agent_ruler_bin,
        runtime_dir=runtime_dir,
        dry_run_send=bool(args.dry_run_send or raw.get("dry_run_send", False)),
        short_id_length=max(4, min(short_id_length, 10)),
        telegram_typing_keepalive=telegram_typing_keepalive,
        telegram_typing_interval_seconds=max(3, telegram_typing_interval_seconds),
        routes_source=routes_source,
        routes=routes,
    )


def derive_runtime_dir(state_file: Path) -> Optional[Path]:
    try:
        bridge_dir = state_file.parent
        user_data_dir = bridge_dir.parent
    except Exception:
        return None
    if bridge_dir.name != "bridge" or user_data_dir.name != "user_data":
        return None
    runtime_dir = user_data_dir.parent
    if not runtime_dir:
        return None
    return runtime_dir


def humanize_label(value: str) -> str:
    cleaned = value.strip().replace("_", " ").replace("-", " ")
    if not cleaned:
        return "Unknown"
    return " ".join(part.capitalize() for part in cleaned.split())


def optional_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value.strip()
    return str(value).strip()


def parse_iso8601_epoch(value: str) -> Optional[int]:
    text = optional_text(value)
    if not text:
        return None
    try:
        parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return int(parsed.timestamp())


def extract_assistant_text(message: Dict[str, Any]) -> str:
    content = message.get("content")
    if not isinstance(content, list):
        return ""
    for item in content:
        if not isinstance(item, dict):
            continue
        if optional_text(item.get("type")).lower() != "text":
            continue
        text = optional_text(item.get("text"))
        if text:
            return text[:MAX_PREAPPROVAL_ASSISTANT_TEXT_CHARS]
    return ""


def runner_display_label(runner_id: str) -> str:
    cleaned = optional_text(runner_id).lower()
    if cleaned in RUNNER_LABELS:
        return RUNNER_LABELS[cleaned]
    return humanize_label(cleaned) if cleaned else "OpenClaw"


def describe_approval_reason(reason_code: str, category: str) -> str:
    reason = optional_text(reason_code).lower() or "approval_required"
    category_clean = optional_text(category).lower() or "approval_required"
    if reason == "approval_required_export" and category_clean == "shared_zone_stage":
        return "Staging files from workspace to shared zone requires approval."
    if reason == "approval_required_export" and category_clean == "deliver":
        return "Delivering files from shared zone to a user destination requires approval."
    message = REASON_DESCRIPTIONS.get(reason)
    if not message:
        message = CATEGORY_DESCRIPTIONS.get(
            category_clean, "A protected action needs confirmation before the runner can continue."
        )
    return message


class InboundHandler(BaseHTTPRequestHandler):
    runtime: ApprovalBridgeRuntime

    def do_POST(self) -> None:  # noqa: N802
        if self.path != "/inbound":
            self._send_json(404, {"error": "not found"})
            return

        length = int(self.headers.get("Content-Length", "0") or "0")
        payload = self.rfile.read(length) if length > 0 else b"{}"
        try:
            decoded = json.loads(payload.decode("utf-8"))
        except json.JSONDecodeError:
            self._send_json(400, {"error": "invalid json"})
            return

        try:
            sync = bool(decoded.get("sync", False))
            if sync:
                result = self.runtime.handle_inbound_event(decoded)
                self._send_json(200, result)
                return

            self.runtime.enqueue_inbound_event(decoded)
        except Exception as err:  # pragma: no cover - defensive
            self._send_json(500, {"error": str(err)})
            return

        self._send_json(202, {"status": "accepted"})

    def log_message(self, fmt: str, *args: Any) -> None:  # noqa: D401
        """Silence default HTTP server logs."""
        return

    def _send_json(self, code: int, payload: Dict[str, Any]) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run_server(runtime: ApprovalBridgeRuntime, bind: str) -> ThreadingHTTPServer:
    host, port_str = bind.rsplit(":", 1)
    server = ThreadingHTTPServer((host, int(port_str)), InboundHandler)
    InboundHandler.runtime = runtime
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


def replay_inbound_file(runtime: ApprovalBridgeRuntime, file_path: Path) -> None:
    raw = file_path.read_text(encoding="utf-8").strip()
    if not raw:
        print("no inbound events to replay")
        return

    events: List[Dict[str, Any]] = []
    if raw.startswith("["):
        parsed = json.loads(raw)
        if isinstance(parsed, list):
            events = [item for item in parsed if isinstance(item, dict)]
    else:
        for line in raw.splitlines():
            line = line.strip()
            if not line:
                continue
            item = json.loads(line)
            if isinstance(item, dict):
                events.append(item)

    for event in events:
        result = runtime.handle_inbound_event(event)
        print(json.dumps(result, indent=2))


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Agent Ruler <-> OpenClaw channel bridge for Telegram/WhatsApp/Discord approvals"
        )
    )
    parser.add_argument("--config", required=True, help="Bridge config JSON file")
    parser.add_argument("--ruler-url", help="Override Agent Ruler base URL")
    parser.add_argument("--public-base-url", help="Override public Control Panel base URL")
    parser.add_argument("--poll-interval", type=int, help="Override poll interval seconds")
    parser.add_argument("--inbound-bind", help="Override inbound bind host:port")
    parser.add_argument("--state-file", help="Override state file path")
    parser.add_argument("--openclaw-bin", help="OpenClaw CLI path (default: openclaw)")
    parser.add_argument(
        "--openclaw-home",
        help="Managed OPENCLAW_HOME to use when loading bridge routes from OpenClaw config",
    )
    parser.add_argument(
        "--agent-ruler-bin",
        help="Agent Ruler CLI path (default: agent-ruler) used to discover managed OPENCLAW_HOME",
    )
    parser.add_argument("--dry-run-send", action="store_true", help="Print send commands only")
    parser.add_argument("--once", action="store_true", help="Poll once and exit")
    parser.add_argument(
        "--replay-inbound",
        help="Replay inbound JSON/JSONL events from file and exit",
    )
    return parser


def main(argv: Optional[List[str]] = None) -> int:
    parser = build_arg_parser()
    args = parser.parse_args(argv)

    try:
        config = load_config(Path(args.config), args)
    except Exception as err:
        print(f"config error: {err}", file=sys.stderr)
        return 2

    client = AgentRulerClient(config.ruler_url)
    messenger = OpenClawMessenger(config.openclaw_bin, config.dry_run_send, config.openclaw_home)
    runtime = ApprovalBridgeRuntime(config, client, messenger)
    log_info(
        f"config loaded: routes_source={config.routes_source} routes={len(config.routes)}"
    )

    if args.replay_inbound:
        replay_inbound_file(runtime, Path(args.replay_inbound))
        return 0

    if args.once:
        try:
            result = runtime.poll_once()
        except Exception as err:
            print(f"[bridge] one-shot poll failed: {err}", file=sys.stderr)
            return 1
        print(json.dumps(result, indent=2))
        return 0

    server = run_server(runtime, config.inbound_bind)
    log_info(f"listening on http://{config.inbound_bind}/inbound")
    log_info(
        f"polling {config.ruler_url}/api/status/feed every {config.poll_interval_seconds}s for pending approvals"
    )

    stop = False

    def handle_signal(_sig: int, _frame: Any) -> None:
        nonlocal stop
        stop = True

    signal.signal(signal.SIGINT, handle_signal)
    signal.signal(signal.SIGTERM, handle_signal)

    try:
        while not stop:
            try:
                runtime.poll_once()
            except Exception as err:  # pragma: no cover - runtime protection
                print(f"[bridge] poll error: {err}", file=sys.stderr)
            time.sleep(config.poll_interval_seconds)
    finally:
        runtime.persist_state()
        server.shutdown()
        server.server_close()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
