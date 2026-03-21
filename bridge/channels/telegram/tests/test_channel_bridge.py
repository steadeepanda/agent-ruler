#!/usr/bin/env python3
"""Unit tests for shared Telegram bridge behavior."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[1] / "channel_bridge.py"
SPEC = importlib.util.spec_from_file_location("agent_ruler_telegram_bridge", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"unable to load module spec for {MODULE_PATH}")
BRIDGE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = BRIDGE
SPEC.loader.exec_module(BRIDGE)


class FakeRulerClient:
    def __init__(self):
        self.resolved = []
        self.session_calls = []
        self.sessions = {}
        self.command_calls = []
        self.command_responses = []
        self.status_events = []
        self.approval_records = {}
        self.ui_log_calls = []
        self.command_started = None
        self.command_release = None

    def status_feed(self):
        return list(self.status_events)

    def resolve(self, approval_id, decision):
        self.resolved.append({"approval_id": approval_id, "decision": decision})
        return {"status": "approved" if decision == "approve" else "denied"}

    def resolve_telegram_session(
        self,
        *,
        runner_kind,
        chat_id,
        thread_id,
        message_anchor_id=None,
        title=None,
        bind_session_id=None,
        bind_runner_session_key=None,
        prefer_existing_runner_session=False,
    ):
        key = (runner_kind, chat_id, thread_id)
        session = self.sessions.get(key)
        created = False
        if session is None:
            created = True
            suffix = max(1, int(thread_id))
            session = {
                "id": f"11111111-2222-3333-4444-{suffix:012d}",
                "display_label": title or f"Telegram thread {thread_id}",
                "telegram_thread_id": thread_id,
                "last_active_at": "2026-03-11T12:00:00Z",
                "runner_session_key": None,
            }
            if bind_runner_session_key:
                session["runner_session_key"] = bind_runner_session_key
            self.sessions[key] = session
        if bind_session_id:
            session["id"] = bind_session_id
        if bind_runner_session_key:
            session["runner_session_key"] = bind_runner_session_key
        self.session_calls.append(
            {
                "runner_kind": runner_kind,
                "chat_id": chat_id,
                "thread_id": thread_id,
                "message_anchor_id": message_anchor_id,
                "title": title,
                "bind_session_id": bind_session_id,
                "bind_runner_session_key": bind_runner_session_key,
                "prefer_existing_runner_session": prefer_existing_runner_session,
                "created": created,
            }
        )
        return {"created": created, "session": dict(session)}

    def run_command(self, cmd):
        self.command_calls.append(list(cmd))
        if self.command_started is not None:
            self.command_started.set()
        if self.command_release is not None:
            self.command_release.wait(timeout=2.0)
        if self.command_responses:
            return dict(self.command_responses.pop(0))
        if cmd and cmd[0] == "claude":
            return {
                "status": "completed",
                "stdout": json.dumps(
                    {
                        "type": "result",
                        "subtype": "success",
                        "is_error": False,
                        "result": "Simulated runner reply",
                        "session_id": "claude-session-abc123",
                    }
                ),
                "stderr": "",
            }
        if cmd and cmd[0] == "opencode":
            return {
                "status": "completed",
                "stdout": "\n".join(
                    [
                        json.dumps(
                            {
                                "type": "step_start",
                                "sessionID": "ses_default",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "text",
                                "sessionID": "ses_default",
                                "part": {"text": "Simulated runner reply"},
                            }
                        ),
                    ]
                ),
                "stderr": "",
            }
        return {"status": "completed", "stdout": "Simulated runner reply", "stderr": ""}

    def approval_get(self, approval_id):
        return dict(self.approval_records.get(approval_id, {}))

    def append_ui_log(self, *, level, source, message, details=None):
        self.ui_log_calls.append(
            {
                "level": level,
                "source": source,
                "message": message,
                "details": details,
            }
        )


class FakePendingApprovalRulerClient(FakeRulerClient):
    def has_pending_approvals(self):
        return True


class FakeTelegramClient:
    def __init__(self):
        self.send_attempts = []
        self.sent = []
        self.edits = []
        self.typing_actions = []
        self.callback_answers = []
        self.created_topics = []
        self.file_downloads = {}
        self.fail_create_topic = False
        self.fail_thread_and_reply = False
        self.wrap_create_topic_response = False
        self._next_message_id = 900

    def send_text(self, **kwargs):
        self.send_attempts.append(dict(kwargs))
        if (
            self.fail_thread_and_reply
            and kwargs.get("message_thread_id") is not None
            and kwargs.get("reply_to_message_id") is not None
        ):
            raise BRIDGE.BridgeError("telegram sendMessage error: Bad Request: message thread not found")
        message_id = self._next_message_id
        self._next_message_id += 1
        payload = dict(kwargs)
        payload["message_id"] = message_id
        self.sent.append(payload)
        return {"ok": True, "result": {"message_id": message_id}}

    def edit_text(self, **kwargs):
        self.edits.append(dict(kwargs))
        return {"ok": True, "result": {"message_id": kwargs.get("message_id")}}

    def send_typing(self, *, chat_id, message_thread_id=None):
        self.typing_actions.append(
            {"chat_id": chat_id, "message_thread_id": message_thread_id}
        )
        return {"ok": True}

    def create_forum_topic(self, *, chat_id, name):
        self.created_topics.append({"chat_id": chat_id, "name": name})
        if self.fail_create_topic:
            raise BRIDGE.BridgeError("topic creation disabled")
        if self.wrap_create_topic_response:
            return {"ok": True, "result": {"message_thread_id": 777, "name": name}}
        return {"message_thread_id": 777, "name": name}

    def get_updates(self, *, offset, timeout_seconds):
        return []

    def answer_callback(self, callback_id, text):
        self.callback_answers.append({"id": callback_id, "text": text})

    def get_file(self, file_id):
        return {"file_path": f"telegram/{file_id}"}

    def download_file(self, file_path):
        return self.file_downloads.get(file_path, b"test-attachment")


def make_config(
    chat_ids,
    allow_from,
    *,
    enabled=True,
    answer_streaming_enabled=True,
    runner_kind="claudecode",
):
    temp = tempfile.NamedTemporaryFile(prefix="telegram-bridge-state-", suffix=".json", delete=False)
    temp.close()
    runtime_dir = tempfile.mkdtemp(prefix="telegram-bridge-runtime-")
    return BRIDGE.BridgeConfig(
        runner_kind=runner_kind,
        enabled=enabled,
        answer_streaming_enabled=answer_streaming_enabled,
        ruler_url="http://127.0.0.1:4622",
        public_base_url="http://127.0.0.1:4622",
        poll_interval_seconds=8,
        decision_ttl_seconds=7200,
        short_id_length=6,
        state_file=Path(temp.name),
        runtime_dir=runtime_dir,
        bot_token="test-token",
        chat_ids=chat_ids,
        allow_from=allow_from,
    )


def wait_for(predicate, *, timeout=2.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    raise AssertionError("condition not met before timeout")


class TelegramBridgeThreadTargetTests(unittest.TestCase):
    def test_parse_chat_target_supports_thread_suffix(self):
        parsed = BRIDGE.parse_chat_target("-1001122334455#17")
        self.assertEqual(parsed.chat_id, "-1001122334455")
        self.assertEqual(parsed.message_thread_id, 17)

    def test_chat_target_filter_does_not_gate_on_configured_chat_targets(self):
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10099887766#8"], ["42"]),
            FakeRulerClient(),
            FakeTelegramClient(),
        )

        self.assertTrue(runtime._chat_allowed("-10099887766", 8))
        self.assertTrue(runtime._chat_allowed("-10099887766", None))
        self.assertTrue(runtime._chat_allowed("-10099887766", 9))

    def test_chat_target_filter_allows_any_chat_when_targets_are_empty(self):
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config([], ["42"]),
            FakeRulerClient(),
            FakeTelegramClient(),
        )

        self.assertTrue(runtime._chat_allowed("-10099887766", None))
        self.assertTrue(runtime._chat_allowed("-10099887766", 19))

    def test_notify_pending_sends_to_bound_thread(self):
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config([], ["42"]),
            FakeRulerClient(),
            telegram,
        )
        runtime.chat_thread_bindings["-10055667788"] = 19
        pending = BRIDGE.PendingApproval(
            approval_id="approval-1",
            short_id="ABCD12",
            created_at=int(time.time()),
            notified=False,
        )
        event = {
            "runner_id": "claudecode",
            "reason_code": "approval_required",
            "category": "filesystem_write",
            "open_in_webui": "/approvals/approval-1",
        }

        delivered = runtime._notify_pending(event, pending)
        self.assertEqual(delivered, 1)
        self.assertEqual(len(telegram.sent), 1)
        self.assertEqual(telegram.sent[0]["chat_id"], "-10055667788")
        self.assertEqual(telegram.sent[0]["message_thread_id"], 19)
        message = telegram.sent[0]["message"]
        self.assertIn("🚨 Approval required", message)
        self.assertIn("Approval ID:\napproval-1", message)
        self.assertIn(
            "Reason:\nA protected action needs confirmation before the runner can continue.",
            message,
        )
        self.assertIn("🔗 Control Panel:", message)
        self.assertNotIn("File involved:", message)
        self.assertIn(
            "Reply with `approve ABCD12` or `deny ABCD12`",
            message,
        )
        keyboard = telegram.sent[0].get("inline_buttons", [])
        self.assertEqual(keyboard[0][0]["text"], "✅ Approve")
        self.assertEqual(keyboard[0][1]["text"], "🚫 Deny")

    def test_notify_pending_uses_chat_thread_binding_when_chat_ids_are_empty(self):
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config([], ["42"]),
            FakeRulerClient(),
            telegram,
        )
        runtime.chat_thread_bindings["-10055667788"] = 31
        pending = BRIDGE.PendingApproval(
            approval_id="approval-2",
            short_id="ABCD34",
            created_at=int(time.time()),
            notified=False,
        )
        event = {
            "runner_id": "opencode",
            "reason_code": "approval_required_network_upload",
            "category": "network_upload",
            "open_in_webui": "/approvals/approval-2",
        }

        delivered = runtime._notify_pending(event, pending)
        self.assertEqual(delivered, 1)
        self.assertEqual(len(telegram.sent), 1)
        self.assertEqual(telegram.sent[0]["chat_id"], "-10055667788")
        self.assertEqual(telegram.sent[0]["message_thread_id"], 31)
        self.assertIn("Reason:\nUploading data to a network destination requires approval.", telegram.sent[0]["message"])

    def test_notify_pending_shows_compact_target_preview(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config([], ["42"]),
            ruler,
            telegram,
        )
        runtime.chat_thread_bindings["-10055667788"] = 31
        runtime_root = Path(runtime.config.runtime_dir)
        workspace_root = runtime_root / "user_data" / "runners" / "claudecode" / "workspace"
        shared_root = runtime_root / "shared-zone"
        ruler.approval_records["approval-3"] = {
            "why": "Export or delivery crosses guarded boundary | stage export requires approval",
            "resolved_src": str(workspace_root / "one.txt"),
            "resolved_dst": str(shared_root / "two.txt"),
            "action": {
                "path": str(shared_root / "three.txt"),
                "secondary_path": str(workspace_root / "four.txt"),
                "metadata": {"target_path": "/home/panda/Documents/agent-ruler-deliveries/five.txt"},
            },
        }
        pending = BRIDGE.PendingApproval(
            approval_id="approval-3",
            short_id="ABCD56",
            created_at=int(time.time()),
            notified=False,
        )
        event = {
            "runner_id": "claudecode",
            "reason_code": "approval_required_export",
            "category": "shared_zone_stage",
            "open_in_webui": "/approvals/approval-3",
        }

        delivered = runtime._notify_pending(event, pending)
        self.assertEqual(delivered, 1)
        message = telegram.sent[0]["message"]
        self.assertIn("File involved:\nworkspace/one.txt", message)
        self.assertIn("Destination:\nshared-zone/two.txt", message)
        self.assertIn("Context paths:", message)
        self.assertIn("~/Documents/agent-ruler-deliveries/five.txt", message)

    def test_decision_allows_mismatched_thread_when_sender_is_allowed(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-100123123123#5"], ["777"]),
            ruler,
            telegram,
        )
        pending = runtime._register_pending("approval-thread-test")

        runtime._handle_decision_command(
            parsed=BRIDGE.ParsedDecisionCommand(decision="approve", reference=pending.short_id),
            user_id="777",
            chat_id="-100123123123",
            thread_id=9,
            callback_id="",
            reply_to_message_id=101,
        )
        self.assertEqual(len(ruler.resolved), 1)
        self.assertEqual(ruler.resolved[0]["approval_id"], "approval-thread-test")
        self.assertIn("✅ Approved :", telegram.sent[-1]["message"])

    def test_threaded_message_relays_prompt_to_runner_and_replies(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 501,
                "text": "Ship today status note",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 33,
            }
        )

        wait_for(lambda: len(ruler.command_calls) == 1 and len(telegram.sent) == 1)
        self.assertEqual(len(ruler.session_calls), 1)
        self.assertEqual(ruler.session_calls[0]["runner_kind"], "claudecode")
        self.assertEqual(ruler.session_calls[0]["thread_id"], 33)
        self.assertEqual(ruler.session_calls[0]["message_anchor_id"], 501)
        self.assertEqual(ruler.session_calls[0]["title"], "Ship today status note")
        self.assertEqual(len(ruler.command_calls), 1)
        self.assertEqual(
            ruler.command_calls[0],
            [
                "claude",
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "Ship today status note",
            ],
        )
        self.assertEqual(telegram.sent[0]["message"], "Simulated runner reply")
        agent_session_id = ruler.sessions[("claudecode", "-10055667788", 33)]["id"]
        self.assertEqual(runtime.session_runner_keys.get(agent_session_id), "claude-session-abc123")
        self.assertGreaterEqual(len(telegram.typing_actions), 1)

    def test_status_command_reuses_existing_session(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 502,
                "text": "hello thread",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 41,
            }
        )
        wait_for(lambda: len(telegram.sent) == 1)
        telegram.sent.clear()

        runtime._handle_message(
            {
                "message_id": 503,
                "text": "/status",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 41,
            }
        )

        self.assertEqual(len(ruler.session_calls), 2)
        self.assertFalse(ruler.session_calls[-1]["created"])
        self.assertEqual(len(telegram.sent), 1)
        self.assertIn("Thread status", telegram.sent[0]["message"])
        self.assertIn("Runner: Claude Code", telegram.sent[0]["message"])

    def test_threaded_replies_prefer_reply_to_message_anchor(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788#41"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 503,
                "text": "/status",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 41,
            }
        )

        self.assertEqual(len(telegram.sent), 1)
        self.assertEqual(telegram.sent[0]["reply_to_message_id"], 503)
        self.assertEqual(telegram.sent[0]["message_thread_id"], 41)

    def test_threaded_replies_fallback_to_reply_anchor_when_thread_send_rejected(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        telegram.fail_thread_and_reply = True
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788#41"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 504,
                "text": "/status",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 41,
            }
        )

        self.assertEqual(len(telegram.send_attempts), 2)
        self.assertEqual(telegram.send_attempts[0]["message_thread_id"], 41)
        self.assertEqual(telegram.send_attempts[0]["reply_to_message_id"], 504)
        self.assertEqual(len(telegram.sent), 1)
        self.assertIsNone(telegram.sent[0]["message_thread_id"])
        self.assertEqual(telegram.sent[0]["reply_to_message_id"], 504)

    def test_threadless_message_bootstraps_topic_and_sends_in_new_thread(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 510,
                "text": "Create kickoff thread",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
            }
        )

        self.assertEqual(len(telegram.created_topics), 1)
        self.assertEqual(telegram.created_topics[0]["chat_id"], "-10055667788")
        self.assertEqual(len(ruler.session_calls), 1)
        self.assertEqual(ruler.session_calls[0]["thread_id"], 777)
        self.assertTrue(
            ruler.session_calls[0]["prefer_existing_runner_session"],
            "threadless normal messages should prefer recent computer-started sessions",
        )
        wait_for(lambda: len(telegram.sent) == 1)
        self.assertEqual(len(telegram.sent), 1)
        self.assertEqual(telegram.sent[0]["message_thread_id"], 777)
        self.assertIsNone(telegram.sent[0]["reply_to_message_id"])

        runtime._handle_message(
            {
                "message_id": 511,
                "text": "/status",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
            }
        )

        self.assertEqual(
            len(telegram.created_topics),
            1,
            "expected threadless follow-up messages to reuse the existing bootstrap thread",
        )
        self.assertEqual(len(ruler.session_calls), 2)
        self.assertEqual(ruler.session_calls[1]["thread_id"], 777)

    def test_continue_command_with_session_id_passes_explicit_bind(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 520,
                "text": "/continue 11111111-2222-3333-4444-555555555555",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 53,
            }
        )

        self.assertEqual(len(ruler.session_calls), 1)
        self.assertEqual(
            ruler.session_calls[0]["bind_session_id"],
            "11111111-2222-3333-4444-555555555555",
        )
        self.assertIsNone(ruler.session_calls[0]["bind_runner_session_key"])
        self.assertIn("Session linked", telegram.sent[0]["message"])

    def test_continue_command_without_argument_prefers_existing_runner_session(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 521,
                "text": "/continue",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 54,
            }
        )

        self.assertEqual(len(ruler.session_calls), 1)
        self.assertTrue(ruler.session_calls[0]["prefer_existing_runner_session"])

    def test_claude_plain_text_learns_and_reuses_runner_session_key(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 530,
                "text": "initial request",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 55,
            }
        )
        wait_for(lambda: len(telegram.sent) >= 1)
        runtime._handle_message(
            {
                "message_id": 531,
                "text": "please summarize this thread",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 55,
            }
        )
        wait_for(lambda: len(ruler.command_calls) == 2 and len(telegram.sent) >= 2)

        self.assertEqual(len(ruler.command_calls), 2)
        self.assertEqual(
            ruler.command_calls[0],
            [
                "claude",
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "initial request",
            ],
        )
        self.assertEqual(
            ruler.command_calls[1],
            [
                "claude",
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "-r",
                "claude-session-abc123",
                "please summarize this thread",
            ],
        )
        self.assertEqual(telegram.sent[-1]["message"], "Simulated runner reply")

    def test_streaming_enabled_emits_suffix_chunks_without_duplicate_final(self):
        ruler = FakeRulerClient()
        ruler.command_responses = [
            {
                "status": "completed",
                "stdout": "\n".join(
                    [
                        json.dumps(
                            {
                                "type": "content_block_delta",
                                "delta": {"text": "Hello"},
                                "session_id": "claude-session-stream",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "content_block_delta",
                                "delta": {"text": " world"},
                                "session_id": "claude-session-stream",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "result",
                                "result": "Hello world",
                                "session_id": "claude-session-stream",
                                "is_error": False,
                            }
                        ),
                    ]
                ),
                "stderr": "",
            }
        ]
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"], answer_streaming_enabled=True),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 532,
                "text": "stream this",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 56,
            }
        )

        wait_for(lambda: len(telegram.sent) == 2)
        self.assertEqual(telegram.sent[0]["message"], "Hello")
        self.assertEqual(telegram.sent[1]["message"], "world")
        self.assertEqual(telegram.edits, [])

    def test_claude_streaming_never_edits_prior_bubble(self):
        ruler = FakeRulerClient()
        ruler.command_responses = [
            {
                "status": "completed",
                "stdout": "\n".join(
                    [
                        json.dumps(
                            {
                                "type": "content_block_delta",
                                "delta": {"text": "First chunk."},
                                "session_id": "claude-session-stream",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "content_block_delta",
                                "delta": {"text": " Second chunk."},
                                "session_id": "claude-session-stream",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "content_block_delta",
                                "delta": {"text": " Third chunk."},
                                "session_id": "claude-session-stream",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "result",
                                "result": "Final chunk.",
                                "session_id": "claude-session-stream",
                                "is_error": False,
                            }
                        ),
                    ]
                ),
                "stderr": "",
            }
        ]
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"], answer_streaming_enabled=True),
            ruler,
            telegram,
        )

        original_interval = BRIDGE.DEFAULT_STREAM_EDIT_INTERVAL_SECONDS
        BRIDGE.DEFAULT_STREAM_EDIT_INTERVAL_SECONDS = 0.0
        try:
            runtime._handle_message(
                {
                    "message_id": 543,
                    "text": "stream chunks",
                    "from": {"id": "42"},
                    "chat": {"id": "-10055667788"},
                    "message_thread_id": 63,
                }
            )
            wait_for(lambda: len(telegram.sent) == 4)
        finally:
            BRIDGE.DEFAULT_STREAM_EDIT_INTERVAL_SECONDS = original_interval

        self.assertEqual(telegram.edits, [])
        self.assertEqual(telegram.sent[0]["message"], "First chunk.")
        self.assertEqual(telegram.sent[1]["message"], "Second chunk.")
        self.assertEqual(telegram.sent[2]["message"], "Third chunk.")
        self.assertEqual(telegram.sent[3]["message"], "Final chunk.")

    def test_streaming_final_whitespace_duplicate_is_not_resent(self):
        ruler = FakeRulerClient()
        ruler.command_responses = [
            {
                "status": "completed",
                "stdout": "\n".join(
                    [
                        json.dumps(
                            {
                                "type": "content_block_delta",
                                "delta": {"text": "Done!"},
                                "session_id": "claude-session-stream",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "content_block_delta",
                                "delta": {"text": "\\n\\n1. Step"},
                                "session_id": "claude-session-stream",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "result",
                                "result": "Done! 1. Step",
                                "session_id": "claude-session-stream",
                                "is_error": False,
                            }
                        ),
                    ]
                ),
                "stderr": "",
            }
        ]
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"], answer_streaming_enabled=True),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 545,
                "text": "stream near duplicate final",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 65,
            }
        )

        wait_for(lambda: len(telegram.sent) == 2)
        self.assertEqual(telegram.edits, [])
        self.assertEqual(telegram.sent[0]["message"], "Done!")
        self.assertEqual(telegram.sent[1]["message"], "1. Step")

    def test_streaming_final_exact_duplicate_is_not_resent(self):
        ruler = FakeRulerClient()
        ruler.command_responses = [
            {
                "status": "completed",
                "stdout": "\n".join(
                    [
                        json.dumps(
                            {
                                "type": "content_block_delta",
                                "delta": {"text": "Done! Final summary."},
                                "session_id": "claude-session-stream",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "result",
                                "result": "Done! Final summary.",
                                "session_id": "claude-session-stream",
                                "is_error": False,
                            }
                        ),
                    ]
                ),
                "stderr": "",
            }
        ]
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"], answer_streaming_enabled=True),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 546,
                "text": "stream exact duplicate final",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 66,
            }
        )

        wait_for(lambda: len(telegram.sent) == 1)
        self.assertEqual(telegram.edits, [])
        self.assertEqual(telegram.sent[0]["message"], "Done! Final summary.")

    def test_opencode_streaming_never_edits_prior_bubble(self):
        ruler = FakeRulerClient()
        ruler.command_responses = [
            {
                "status": "completed",
                "stdout": "\n".join(
                    [
                        json.dumps({"type": "step_start", "sessionID": "ses_stream"}),
                        json.dumps(
                            {
                                "type": "text",
                                "sessionID": "ses_stream",
                                "part": {"text": "Part one"},
                            }
                        ),
                        json.dumps(
                            {
                                "type": "text",
                                "sessionID": "ses_stream",
                                "part": {"text": "Part two"},
                            }
                        ),
                        json.dumps(
                            {
                                "type": "text",
                                "sessionID": "ses_stream",
                                "part": {"text": "Part three"},
                            }
                        ),
                    ]
                ),
                "stderr": "",
            }
        ]
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"], answer_streaming_enabled=True, runner_kind="opencode"),
            ruler,
            telegram,
        )

        original_interval = BRIDGE.DEFAULT_STREAM_EDIT_INTERVAL_SECONDS
        BRIDGE.DEFAULT_STREAM_EDIT_INTERVAL_SECONDS = 0.0
        try:
            runtime._handle_message(
                {
                    "message_id": 544,
                    "text": "stream opencode chunks",
                    "from": {"id": "42"},
                    "chat": {"id": "-10055667788"},
                    "message_thread_id": 64,
                }
            )
            wait_for(lambda: len(telegram.sent) == 3)
        finally:
            BRIDGE.DEFAULT_STREAM_EDIT_INTERVAL_SECONDS = original_interval

        self.assertEqual(telegram.edits, [])
        self.assertEqual(telegram.sent[0]["message"], "Part one")
        self.assertEqual(telegram.sent[1]["message"], "Part two")
        self.assertEqual(telegram.sent[2]["message"], "Part three")

    def test_streaming_disabled_sends_only_final_reply(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"], answer_streaming_enabled=False),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 533,
                "text": "no stream",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 57,
            }
        )

        wait_for(lambda: len(telegram.sent) == 1)
        self.assertEqual(telegram.sent[0]["message"], "Simulated runner reply")
        self.assertEqual(telegram.edits, [])

    def test_pending_approval_still_delivers_while_runner_request_is_waiting(self):
        ruler = FakeRulerClient()
        ruler.command_started = threading.Event()
        ruler.command_release = threading.Event()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 534,
                "text": "wait for approval",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 58,
            }
        )
        self.assertTrue(ruler.command_started.wait(timeout=1.0))

        ruler.status_events = [
            {
                "approval_id": "approval-blocked",
                "verdict": "pending",
                "runner_id": "claudecode",
                "reason_code": "approval_required_network_upload",
                "category": "network_upload",
                "open_in_webui": "/approvals/approval-blocked",
            }
        ]
        runtime._poll_pending_approvals()
        wait_for(lambda: any("approval-blocked" in item["message"] for item in telegram.sent))
        ruler.command_release.set()
        wait_for(lambda: any(item["message"] == "Simulated runner reply" for item in telegram.sent))

        approval_messages = [
            item["message"] for item in telegram.sent if "approval-blocked" in item["message"]
        ]
        self.assertEqual(len(approval_messages), 1)

    def test_progress_notice_requires_pending_approval(self):
        ruler = FakeRulerClient()
        ruler.command_started = threading.Event()
        ruler.command_release = threading.Event()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        original_delay = BRIDGE.DEFAULT_PROGRESS_NOTICE_DELAY_SECONDS
        BRIDGE.DEFAULT_PROGRESS_NOTICE_DELAY_SECONDS = 0.05
        try:
            runtime._handle_message(
                {
                    "message_id": 535,
                    "text": "slow response",
                    "from": {"id": "42"},
                    "chat": {"id": "-10055667788"},
                    "message_thread_id": 59,
                }
            )
            self.assertTrue(ruler.command_started.wait(timeout=1.0))
            time.sleep(0.12)
            ruler.command_release.set()
            wait_for(lambda: any(item["message"] == "Simulated runner reply" for item in telegram.sent))
        finally:
            BRIDGE.DEFAULT_PROGRESS_NOTICE_DELAY_SECONDS = original_delay

        self.assertFalse(
            any("Working on it" in item["message"] for item in telegram.sent),
            telegram.sent,
        )

    def test_progress_notice_emits_when_pending_approval_exists(self):
        ruler = FakePendingApprovalRulerClient()
        ruler.command_started = threading.Event()
        ruler.command_release = threading.Event()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        original_delay = BRIDGE.DEFAULT_PROGRESS_NOTICE_DELAY_SECONDS
        BRIDGE.DEFAULT_PROGRESS_NOTICE_DELAY_SECONDS = 0.05
        try:
            runtime._handle_message(
                {
                    "message_id": 536,
                    "text": "approval wait",
                    "from": {"id": "42"},
                    "chat": {"id": "-10055667788"},
                    "message_thread_id": 60,
                }
            )
            self.assertTrue(ruler.command_started.wait(timeout=1.0))
            wait_for(lambda: any("Working on it" in item["message"] for item in telegram.sent))
            ruler.command_release.set()
            wait_for(lambda: any(item["message"] == "Simulated runner reply" for item in telegram.sent))
        finally:
            BRIDGE.DEFAULT_PROGRESS_NOTICE_DELAY_SECONDS = original_delay

    def test_opencode_plain_text_learns_and_reuses_session_key(self):
        ruler = FakeRulerClient()
        ruler.command_responses = [
            {
                "status": "completed",
                "stdout": "\n".join(
                    [
                        json.dumps({"type": "step_start", "sessionID": "ses_abc123"}),
                        json.dumps(
                            {
                                "type": "text",
                                "sessionID": "ses_abc123",
                                "part": {"text": "OpenCode first reply"},
                            }
                        ),
                    ]
                ),
                "stderr": "",
            },
            {
                "status": "completed",
                "stdout": "\n".join(
                    [
                        json.dumps({"type": "step_start", "sessionID": "ses_abc123"}),
                        json.dumps(
                            {
                                "type": "text",
                                "sessionID": "ses_abc123",
                                "part": {"text": "OpenCode follow-up reply"},
                            }
                        ),
                    ]
                ),
                "stderr": "",
            },
        ]
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"], runner_kind="opencode"),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 540,
                "text": "first request",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 61,
            }
        )
        wait_for(lambda: len(telegram.sent) >= 1)
        first_session_id = ruler.sessions[("opencode", "-10055667788", 61)]["id"]

        runtime._handle_message(
            {
                "message_id": 541,
                "text": "second request",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 61,
            }
        )
        wait_for(lambda: len(ruler.command_calls) == 2 and len(telegram.sent) >= 2)

        self.assertEqual(ruler.command_calls[0], ["opencode", "run", "--format", "json", "first request"])
        self.assertEqual(
            ruler.command_calls[1],
            ["opencode", "run", "--format", "json", "--session", "ses_abc123", "second request"],
        )
        self.assertEqual(runtime.session_runner_keys.get(first_session_id), "ses_abc123")
        self.assertEqual(telegram.sent[-1]["message"], "OpenCode follow-up reply")

    def test_opencode_without_text_returns_completion_fallback(self):
        ruler = FakeRulerClient()
        ruler.command_responses = [
            {
                "status": "completed",
                "stdout": json.dumps(
                    {
                        "type": "step_finish",
                        "sessionID": "ses_summary_only",
                        "part": {"type": "step-finish", "reason": "stop"},
                    }
                ),
                "stderr": "",
            }
        ]
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"], runner_kind="opencode"),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 542,
                "text": "summary only",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 62,
            }
        )
        wait_for(lambda: len(telegram.sent) >= 1)
        self.assertIn("did not emit a text summary", telegram.sent[-1]["message"])

    def test_new_command_threadless_forces_new_topic(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )
        runtime.chat_thread_bindings["-10055667788"] = 333

        runtime._handle_message(
            {
                "message_id": 522,
                "text": "/new Daily digest",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
            }
        )

        self.assertEqual(len(telegram.created_topics), 1)
        self.assertEqual(telegram.created_topics[0]["name"], "Daily digest")
        self.assertEqual(len(ruler.session_calls), 1)
        self.assertEqual(
            ruler.session_calls[0]["thread_id"],
            777,
            "expected /new to create and use a fresh thread instead of prior chat binding",
        )

    def test_attachment_message_stages_files_and_forwards_workspace_paths(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        telegram.file_downloads["telegram/doc-1"] = b"hello from telegram"
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 523,
                "caption": "Please review the attached report",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 70,
                "document": {
                    "file_id": "doc-1",
                    "file_name": "quarterly-report.pdf",
                    "mime_type": "application/pdf",
                    "file_size": 21,
                },
            }
        )
        wait_for(lambda: len(ruler.command_calls) == 1)

        self.assertEqual(len(ruler.command_calls), 1)
        forwarded_prompt = ruler.command_calls[0][-1]
        self.assertIn("Please review the attached report", forwarded_prompt)
        self.assertIn("Telegram attachments saved in the workspace:", forwarded_prompt)
        self.assertIn(".agent-ruler-telegram/claudecode/chat-10055667788/thread-70/msg-523/", forwarded_prompt)
        staged_root = Path(runtime.config.runtime_dir) / "user_data" / "runners" / "claudecode" / "workspace"
        staged_files = list(staged_root.rglob("*.pdf"))
        self.assertEqual(len(staged_files), 1)
        self.assertEqual(staged_files[0].read_bytes(), b"hello from telegram")

    def test_threadless_message_bootstraps_even_when_target_is_thread_scoped(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788#41"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 512,
                "text": "Bootstrap from All tab",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
            }
        )

        self.assertEqual(len(telegram.created_topics), 1)
        self.assertEqual(telegram.created_topics[0]["chat_id"], "-10055667788")
        self.assertEqual(len(ruler.session_calls), 1)
        self.assertEqual(ruler.session_calls[0]["thread_id"], 777)
        wait_for(lambda: len(telegram.sent) == 1)
        self.assertEqual(len(telegram.sent), 1)
        self.assertEqual(telegram.sent[0]["message_thread_id"], 777)

    def test_threadless_bootstrap_accepts_telegram_result_wrapper_payload(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        telegram.wrap_create_topic_response = True
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "text": "create wrapped topic",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
            }
        )

        self.assertEqual(len(telegram.created_topics), 1)
        self.assertEqual(len(ruler.session_calls), 1)
        self.assertEqual(ruler.session_calls[0]["thread_id"], 777)
        wait_for(lambda: len(telegram.sent) == 1)
        self.assertEqual(len(telegram.sent), 1)
        self.assertEqual(telegram.sent[0]["message_thread_id"], 777)

    def test_help_without_thread_mentions_botfather_threaded_mode(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        telegram.fail_create_topic = True
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 504,
                "text": "/help",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
            }
        )

        self.assertEqual(ruler.session_calls, [])
        self.assertEqual(len(telegram.sent), 1)
        self.assertIn("Threaded Mode", telegram.sent[0]["message"])
        self.assertIn("BotFather", telegram.sent[0]["message"])

    def test_help_in_thread_lists_commands_and_attachment_support(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config(["-10055667788"], ["42"]),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 505,
                "text": "/help",
                "from": {"id": "42"},
                "chat": {"id": "-10055667788"},
                "message_thread_id": 80,
            }
        )

        self.assertEqual(len(telegram.sent), 1)
        self.assertIn("/new [topic]", telegram.sent[0]["message"])
        self.assertIn("/link - alias for /continue.", telegram.sent[0]["message"])
        self.assertIn("approve SHORTID", telegram.sent[0]["message"])
        self.assertIn("Photos, videos, documents", telegram.sent[0]["message"])

    def test_whoami_reply_works_without_allow_from_or_chat_targets(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config([], []),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 701,
                "text": "/whoami",
                "from": {"id": "424242"},
                "chat": {"id": "-10055667788"},
            }
        )

        self.assertEqual(len(telegram.sent), 1)
        self.assertIn("Sender ID: 424242", telegram.sent[0]["message"])
        self.assertEqual(ruler.session_calls, [])

    def test_disabled_mode_allows_only_whoami(self):
        ruler = FakeRulerClient()
        telegram = FakeTelegramClient()
        runtime = BRIDGE.TelegramBridgeRuntime(
            make_config([], [], enabled=False),
            ruler,
            telegram,
        )

        runtime._handle_message(
            {
                "message_id": 710,
                "text": "/status",
                "from": {"id": "424242"},
                "chat": {"id": "-10055667788"},
            }
        )
        self.assertEqual(ruler.session_calls, [])
        self.assertEqual(len(telegram.sent), 1)
        self.assertIn("Only /whoami is available", telegram.sent[0]["message"])

        runtime._handle_message(
            {
                "message_id": 711,
                "text": "/whoami",
                "from": {"id": "424242"},
                "chat": {"id": "-10055667788"},
            }
        )
        self.assertEqual(len(telegram.sent), 2)
        self.assertIn("Sender ID: 424242", telegram.sent[-1]["message"])

    def test_describe_approval_reason_prefers_reason_code_mapping(self):
        described = BRIDGE.describe_approval_reason("approval_required_network_upload", "approval_required")
        self.assertEqual(
            described,
            "Uploading data to a network destination requires approval.",
        )

    def test_describe_approval_reason_falls_back_to_category_mapping(self):
        described = BRIDGE.describe_approval_reason("approval_required_custom", "deliver")
        self.assertEqual(
            described,
            "Delivering files from shared zone to user destination requires approval.",
        )


if __name__ == "__main__":
    unittest.main()
