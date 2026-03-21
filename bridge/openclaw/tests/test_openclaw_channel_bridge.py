import argparse
import json
import re
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from subprocess import CompletedProcess
from unittest.mock import patch
from urllib.parse import parse_qs, urlparse

from bridge.openclaw.channel_bridge import (
    AgentRulerClient,
    ApprovalBridgeRuntime,
    BridgeError,
    BridgeConfig,
    OpenClawMessenger,
    RouteConfig,
    load_config,
    normalize_inbound_event,
    parse_decision_command,
)


class FakeMessenger(OpenClawMessenger):
    def __init__(self):
        super().__init__(openclaw_bin="openclaw", dry_run_send=True)
        self.text_messages = []
        self.poll_messages = []
        self.typing_events = []
        self.callback_answers = []

    def send_text(
        self,
        *,
        channel,
        target,
        message,
        account=None,
        telegram_buttons=None,
        message_thread_id=None,
    ):
        payload = {
            "channel": channel,
            "target": target,
            "message": message,
            "account": account,
            "telegram_buttons": telegram_buttons,
            "message_thread_id": message_thread_id,
        }
        self.text_messages.append(payload)
        return payload

    def send_poll(self, *, channel, target, question, options, account=None):
        payload = {
            "channel": channel,
            "target": target,
            "question": question,
            "options": list(options),
            "account": account,
        }
        self.poll_messages.append(payload)
        return payload

    def send_typing(self, *, channel, target, account=None):
        payload = {
            "channel": channel,
            "target": target,
            "account": account,
            "action": "typing",
        }
        self.typing_events.append(payload)
        return payload

    def answer_callback(self, *, channel, callback_query_id, text, account=None):
        payload = {
            "channel": channel,
            "callback_query_id": callback_query_id,
            "text": text,
            "account": account,
        }
        self.callback_answers.append(payload)
        return payload


class MockAgentRulerHandler(BaseHTTPRequestHandler):
    pending_event = {
        "approval_id": "approval-123",
        "verdict": "pending",
        "reason_code": "approval_required_export",
        "category": "deliver",
        "guidance": "waiting for approval; open /approvals/approval-123 in WebUI",
        "open_in_webui": "/approvals/approval-123",
        "updated_at": "2026-02-21T00:00:00Z",
    }
    decisions = []
    approval_status = "pending"

    def do_GET(self):  # noqa: N802
        parsed = urlparse(self.path)
        if parsed.path == "/api/status/feed":
            if type(self).approval_status == "pending":
                self._send_json(200, [type(self).pending_event])
            else:
                self._send_json(200, [])
            return

        if parsed.path.startswith("/api/approvals/") and parsed.path.endswith("/wait"):
            approval_id = parsed.path.split("/")[3]
            query = parse_qs(parsed.query)
            timeout = int(query.get("timeout", ["30"])[0])
            if type(self).approval_status == "pending":
                self._send_json(
                    200,
                    {
                        "approval_id": approval_id,
                        "status": "timeout",
                        "timeout": timeout,
                    },
                )
                return

            self._send_json(
                200,
                {
                    "approval_id": approval_id,
                    "status": type(self).approval_status,
                },
            )
            return

        self._send_json(404, {"error": "not found"})

    def do_POST(self):  # noqa: N802
        if self.path.startswith("/api/approvals/") and self.path.endswith("/approve"):
            approval_id = self.path.split("/")[3]
            type(self).decisions.append(("approve", approval_id))
            type(self).approval_status = "approved"
            self._send_json(200, {"id": approval_id, "status": "approved"})
            return
        if self.path.startswith("/api/approvals/") and self.path.endswith("/deny"):
            approval_id = self.path.split("/")[3]
            type(self).decisions.append(("deny", approval_id))
            type(self).approval_status = "denied"
            self._send_json(200, {"id": approval_id, "status": "denied"})
            return
        self._send_json(404, {"error": "not found"})

    def log_message(self, fmt, *args):
        return

    def _send_json(self, code, payload):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class DecisionParseTests(unittest.TestCase):
    def test_parse_telegram_callback_command(self):
        cmd = parse_decision_command("callback_data: ar:approve:AR7K2P")
        self.assertIsNotNone(cmd)
        assert cmd is not None
        self.assertEqual(cmd.decision, "approve")
        self.assertEqual(cmd.reference, "AR7K2P")

    def test_parse_telegram_control_callback_command(self):
        cmd = parse_decision_command("/arapprove AR7K2P")
        self.assertIsNotNone(cmd)
        assert cmd is not None
        self.assertEqual(cmd.decision, "approve")
        self.assertEqual(cmd.reference, "AR7K2P")

    def test_parse_whatsapp_text_command(self):
        cmd = parse_decision_command("approve ar7k2p")
        self.assertIsNotNone(cmd)
        assert cmd is not None
        self.assertEqual(cmd.decision, "approve")
        self.assertEqual(cmd.reference, "AR7K2P")

    def test_parse_discord_slash_command(self):
        cmd = parse_decision_command("/deny approval-123")
        self.assertIsNotNone(cmd)
        assert cmd is not None
        self.assertEqual(cmd.decision, "deny")
        self.assertEqual(cmd.reference, "approval-123")

    def test_parse_queued_wrapper_command(self):
        cmd = parse_decision_command(
            "[Queued messages while agent was busy]\n\n---\nQueued #1\nar:approve:KCNKY4"
        )
        self.assertIsNotNone(cmd)
        assert cmd is not None
        self.assertEqual(cmd.decision, "approve")
        self.assertEqual(cmd.reference, "KCNKY4")


class OpenClawMessengerTests(unittest.TestCase):
    def test_answer_callback_swallows_transport_error_like_shared_bridge(self):
        messenger = OpenClawMessenger(openclaw_bin="openclaw", dry_run_send=False)
        with patch.object(messenger, "_resolve_telegram_bot_token", return_value="123:ABC"), patch.object(
            messenger,
            "_telegram_request_json",
            side_effect=BridgeError("boom"),
        ):
            messenger.answer_callback(
                channel="telegram",
                callback_query_id="cbq-1",
                text="processing",
                account="default",
            )

    def test_answer_callback_posts_callback_query_id_and_text(self):
        messenger = OpenClawMessenger(openclaw_bin="openclaw", dry_run_send=False)
        with patch.object(messenger, "_resolve_telegram_bot_token", return_value="123:ABC"), patch.object(
            messenger, "_telegram_request_json", return_value={"ok": True}
        ) as request_json:
            messenger.answer_callback(
                channel="telegram",
                callback_query_id="cbq-2",
                text="done",
                account="default",
            )

        request_json.assert_called_once_with(
            token="123:ABC",
            method="answerCallbackQuery",
            payload={"callback_query_id": "cbq-2", "text": "done"},
        )


class InboundNormalizeTests(unittest.TestCase):
    def test_telegram_callback_falls_back_to_message_id_when_query_id_is_missing(self):
        payload = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "-10012345",
            "messageId": "msg-123",
            "from": "988088319",
            "content": "",
            "metadata": {
                "callback_data": "/arapprove KCNKY4",
            },
        }

        normalized = normalize_inbound_event(payload)
        self.assertIsNotNone(normalized)
        assert normalized is not None
        self.assertEqual(normalized.get("callback_query_id"), "msg-123")

    def test_telegram_control_callback_command_falls_back_without_metadata(self):
        payload = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "-10012345",
            "messageId": "msg-456",
            "from": "988088319",
            "content": "/arapprove KCNKY4",
            "metadata": {},
        }

        normalized = normalize_inbound_event(payload)
        self.assertIsNotNone(normalized)
        assert normalized is not None
        self.assertEqual(normalized.get("callback_query_id"), "msg-456")

    def test_telegram_callback_uses_real_callback_query_id_when_present(self):
        payload = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "-10012345",
            "messageId": "msg-123",
            "from": "988088319",
            "content": "",
            "metadata": {
                "callback_data": "/arapprove KCNKY4",
                "callback_query_id": "cbq-777",
            },
        }

        normalized = normalize_inbound_event(payload)
        self.assertIsNotNone(normalized)
        assert normalized is not None
        self.assertEqual(normalized.get("callback_query_id"), "cbq-777")

    def test_telegram_text_command_does_not_map_message_id_without_callback_metadata(self):
        payload = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "-10012345",
            "messageId": "msg-888",
            "from": "988088319",
            "content": "approve KCNKY4",
            "metadata": {},
        }

        normalized = normalize_inbound_event(payload)
        self.assertIsNotNone(normalized)
        assert normalized is not None
        self.assertNotIn("callback_query_id", normalized)


class BridgeIntegrationTests(unittest.TestCase):
    def setUp(self):
        MockAgentRulerHandler.decisions = []
        MockAgentRulerHandler.approval_status = "pending"
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), MockAgentRulerHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base_url = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.tempdir = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.tempdir.cleanup()

    def _runtime(self, routes, *, agent_ruler_bin="", openclaw_home=None):
        state_file = Path(self.tempdir.name) / "bridge-state.json"
        config = BridgeConfig(
            ruler_url=self.base_url,
            public_base_url=self.base_url,
            poll_interval_seconds=1,
            decision_ttl_seconds=3600,
            inbound_bind="127.0.0.1:4661",
            state_file=state_file,
            openclaw_bin="openclaw",
            openclaw_home=openclaw_home,
            agent_ruler_bin=agent_ruler_bin,
            runtime_dir=None,
            dry_run_send=True,
            short_id_length=6,
            telegram_typing_keepalive=True,
            telegram_typing_interval_seconds=5,
            routes_source="test",
            routes=routes,
        )

        client = AgentRulerClient(self.base_url)
        messenger = FakeMessenger()
        runtime = ApprovalBridgeRuntime(config, client, messenger)
        return runtime, messenger

    def test_telegram_payload_has_buttons_and_deep_link(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ]
        )

        poll_result = runtime.poll_once()
        self.assertEqual(poll_result["notified"], 1)
        self.assertEqual(len(messenger.text_messages), 1)

        outbound = messenger.text_messages[0]
        self.assertTrue(outbound["message"].startswith("/stop\n\n"))
        self.assertIn("Open in Control Panel", outbound["message"])
        self.assertIn(f"{self.base_url}/approvals/approval-123", outbound["message"])

        buttons = outbound["telegram_buttons"]
        self.assertIsNotNone(buttons)
        assert buttons is not None
        self.assertEqual(buttons[0][0]["text"], "✅ Approve")
        self.assertEqual(buttons[0][1]["text"], "🚫 Deny")
        self.assertRegex(buttons[0][0]["callback_data"], r"^/arapprove [A-Z2-9]{6}$")
        self.assertRegex(buttons[0][1]["callback_data"], r"^/ardeny [A-Z2-9]{6}$")

    def test_telegram_sends_preapproval_assistant_text_before_approval_card(self):
        original_event = dict(MockAgentRulerHandler.pending_event)
        session_hint = "session-preapproval-1"
        assistant_reply = "Mars is the fourth planet from the Sun."
        openclaw_home = Path(self.tempdir.name) / "openclaw-home"
        session_dir = openclaw_home / ".openclaw" / "agents" / "test-agent" / "sessions"
        session_dir.mkdir(parents=True, exist_ok=True)
        session_file = session_dir / f"{session_hint}.jsonl"
        session_file.write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "type": "message",
                            "timestamp": "2026-03-20T05:00:00Z",
                            "message": {
                                "role": "user",
                                "content": [{"type": "text", "text": "answer then test"}],
                            },
                        }
                    ),
                    json.dumps(
                        {
                            "type": "message",
                            "timestamp": "2026-03-20T05:00:02Z",
                            "message": {
                                "role": "assistant",
                                "content": [{"type": "text", "text": assistant_reply}],
                            },
                        }
                    ),
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        MockAgentRulerHandler.pending_event = {
            **original_event,
            "session_hint": session_hint,
        }
        try:
            runtime, messenger = self._runtime(
                [
                    RouteConfig(
                        channel="telegram",
                        target="123456789",
                        allow_from=["123456789"],
                        account="default",
                        telegram_inline_buttons=True,
                        whatsapp_use_poll=False,
                    )
                ],
                openclaw_home=str(openclaw_home),
            )

            poll_result = runtime.poll_once()
            self.assertEqual(poll_result["notified"], 1)
            self.assertEqual(len(messenger.text_messages), 2)
            self.assertEqual(messenger.text_messages[0]["message"], assistant_reply)
            self.assertIsNone(messenger.text_messages[0]["telegram_buttons"])
            self.assertTrue(messenger.text_messages[1]["message"].startswith("/stop\n\n"))
            self.assertIsNotNone(messenger.text_messages[1]["telegram_buttons"])
        finally:
            MockAgentRulerHandler.pending_event = original_event

    def test_telegram_skips_preapproval_replay_when_streaming_enabled(self):
        original_event = dict(MockAgentRulerHandler.pending_event)
        session_hint = "session-preapproval-streaming"
        assistant_reply = "This should not be replayed after live streaming."
        openclaw_home = Path(self.tempdir.name) / "openclaw-home-streaming"
        session_dir = openclaw_home / ".openclaw" / "agents" / "test-agent" / "sessions"
        session_dir.mkdir(parents=True, exist_ok=True)
        session_file = session_dir / f"{session_hint}.jsonl"
        session_file.write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "type": "message",
                            "timestamp": "2026-03-20T07:10:00Z",
                            "message": {
                                "role": "user",
                                "content": [{"type": "text", "text": "reply then stage"}],
                            },
                        }
                    ),
                    json.dumps(
                        {
                            "type": "message",
                            "timestamp": "2026-03-20T07:10:02Z",
                            "message": {
                                "role": "assistant",
                                "content": [{"type": "text", "text": assistant_reply}],
                            },
                        }
                    ),
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        MockAgentRulerHandler.pending_event = {
            **original_event,
            "session_hint": session_hint,
        }
        try:
            runtime, messenger = self._runtime(
                [
                    RouteConfig(
                        channel="telegram",
                        target="123456789",
                        allow_from=["123456789"],
                        account="default",
                        telegram_inline_buttons=True,
                        telegram_streaming_enabled=True,
                        whatsapp_use_poll=False,
                    )
                ],
                openclaw_home=str(openclaw_home),
            )

            poll_result = runtime.poll_once()
            self.assertEqual(poll_result["notified"], 1)
            self.assertEqual(len(messenger.text_messages), 1)
            self.assertTrue(messenger.text_messages[0]["message"].startswith("/stop\n\n"))
            self.assertNotIn(assistant_reply, messenger.text_messages[0]["message"])
        finally:
            MockAgentRulerHandler.pending_event = original_event

    def test_telegram_sends_preapproval_assistant_text_without_session_hint_using_action_paths(self):
        original_event = dict(MockAgentRulerHandler.pending_event)
        assistant_reply = "ORDER-FIRST fallback path check."
        source_path = (
            "/home/panda/.local/share/agent-ruler/projects/installs-76e0d3e7d834/"
            "workspace/fallback-order-proof.txt"
        )
        destination_path = (
            "/home/panda/.local/share/agent-ruler/projects/installs-76e0d3e7d834/"
            "shared-zone/fallback-order-proof.txt"
        )

        openclaw_home = Path(self.tempdir.name) / "openclaw-home-fallback"
        session_dir = openclaw_home / ".openclaw" / "agents" / "test-agent" / "sessions"
        session_dir.mkdir(parents=True, exist_ok=True)
        session_file = session_dir / "no-session-hint.jsonl"
        session_file.write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "type": "message",
                            "timestamp": "2026-03-20T06:00:00Z",
                            "message": {
                                "role": "user",
                                "content": [{"type": "text", "text": "answer first"}],
                            },
                        }
                    ),
                    json.dumps(
                        {
                            "type": "message",
                            "timestamp": "2026-03-20T06:00:01Z",
                            "message": {
                                "role": "assistant",
                                "content": [{"type": "text", "text": assistant_reply}],
                            },
                        }
                    ),
                    json.dumps(
                        {
                            "type": "tool_use",
                            "timestamp": "2026-03-20T06:00:02Z",
                            "tool_name": "agent_ruler_request_export_stage",
                            "params": {"source": source_path, "destination": destination_path},
                        }
                    ),
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        MockAgentRulerHandler.pending_event = {
            **original_event,
            "session_hint": "",
        }
        try:
            runtime, messenger = self._runtime(
                [
                    RouteConfig(
                        channel="telegram",
                        target="123456789",
                        allow_from=["123456789"],
                        account="default",
                        telegram_inline_buttons=True,
                        whatsapp_use_poll=False,
                    )
                ],
                openclaw_home=str(openclaw_home),
            )

            with patch.object(
                runtime.client,
                "approval",
                return_value={
                    "resolved_src": source_path,
                    "resolved_dst": destination_path,
                    "action": {
                        "path": destination_path,
                        "secondary_path": source_path,
                        "metadata": {
                            "export_src": source_path,
                            "export_dst": destination_path,
                        },
                    },
                },
            ):
                poll_result = runtime.poll_once()
            self.assertEqual(poll_result["notified"], 1)
            self.assertEqual(len(messenger.text_messages), 2)
            self.assertEqual(messenger.text_messages[0]["message"], assistant_reply)
            self.assertTrue(messenger.text_messages[1]["message"].startswith("/stop\n\n"))
        finally:
            MockAgentRulerHandler.pending_event = original_event

    def test_telegram_payload_uses_file_involved_and_destination_labels(self):
        original_event = dict(MockAgentRulerHandler.pending_event)
        source = str(Path.home() / "agent-ruler" / "workspace" / "report.txt")
        destination = str(Path.home() / "agent-ruler" / "deliveries" / "report.txt")
        MockAgentRulerHandler.pending_event = {
            **original_event,
            "operation": "deliver_commit",
            "path": source,
            "secondary_path": destination,
        }
        try:
            runtime, messenger = self._runtime(
                [
                    RouteConfig(
                        channel="telegram",
                        target="123456789",
                        allow_from=["123456789"],
                        account="default",
                        telegram_inline_buttons=True,
                        whatsapp_use_poll=False,
                    )
                ]
            )
            poll_result = runtime.poll_once()
            self.assertEqual(poll_result["notified"], 1)
            text_msg = messenger.text_messages[0]["message"]
            self.assertIn("*File involved:*", text_msg)
            self.assertIn("*Destination:*", text_msg)
            self.assertNotIn("Primary Path", text_msg)
            self.assertNotIn("Secondary Path", text_msg)
            self.assertIn("~/agent-ruler/workspace/report.txt", text_msg)
            self.assertIn("~/agent-ruler/deliveries/report.txt", text_msg)
        finally:
            MockAgentRulerHandler.pending_event = original_event

    def test_telegram_typing_keepalive_runs_while_approval_pending(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ]
        )

        runtime._typing_keepalive_interval_seconds = 0
        first = runtime.poll_once()
        self.assertEqual(first["notified"], 1)
        self.assertGreaterEqual(len(messenger.typing_events), 1)

        second = runtime.poll_once()
        self.assertEqual(second["events_seen"], 1)
        self.assertGreaterEqual(
            len(messenger.typing_events),
            2,
            "typing keepalive should continue while approval stays pending",
        )

    def test_resolved_approval_is_reconciled_and_typing_stops(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ]
        )

        runtime._typing_keepalive_interval_seconds = 0
        first = runtime.poll_once()
        self.assertEqual(first["notified"], 1)
        self.assertIn("approval-123", runtime.pending_by_approval)
        self.assertGreaterEqual(len(messenger.typing_events), 1)

        MockAgentRulerHandler.approval_status = "approved"
        second = runtime.poll_once()
        self.assertEqual(second["events_seen"], 0)
        self.assertNotIn("approval-123", runtime.pending_by_approval)
        typing_after_reconcile = len(messenger.typing_events)

        runtime.poll_once()
        self.assertEqual(
            len(messenger.typing_events),
            typing_after_reconcile,
            "typing keepalive should stop once approvals are no longer pending",
        )

    def test_whatsapp_poll_and_command_fallback_payload(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="whatsapp",
                    target="+15555550123",
                    allow_from=["+15555550123"],
                    account="default",
                    telegram_inline_buttons=False,
                    whatsapp_use_poll=True,
                )
            ]
        )

        poll_result = runtime.poll_once()
        self.assertEqual(poll_result["notified"], 1)
        self.assertEqual(len(messenger.poll_messages), 1)
        self.assertEqual(len(messenger.text_messages), 1)

        poll_msg = messenger.poll_messages[0]
        self.assertEqual(poll_msg["channel"], "whatsapp")
        self.assertEqual(len(poll_msg["options"]), 2)
        self.assertRegex(poll_msg["options"][0], r"^approve [A-Z2-9]{6}$")
        self.assertRegex(poll_msg["options"][1], r"^deny [A-Z2-9]{6}$")

        text_msg = messenger.text_messages[0]["message"]
        self.assertIn("WhatsApp poll sent", text_msg)
        self.assertIn(f"{self.base_url}/approvals/approval-123", text_msg)
        self.assertIn("Approval required", text_msg)
        self.assertNotIn("File involved:", text_msg)

    def test_pending_approval_retries_notification_after_routes_become_available(self):
        runtime, messenger = self._runtime([])
        runtime.config.routes_source = "openclaw_unconfigured"

        first = runtime.poll_once()
        self.assertEqual(first["notified"], 0)
        self.assertEqual(len(messenger.text_messages), 0)
        self.assertIn("approval-123", runtime.pending_by_approval)
        self.assertFalse(runtime.pending_by_approval["approval-123"].notified)

        runtime.config.routes = [
            RouteConfig(
                channel="telegram",
                target="123456789",
                allow_from=["123456789"],
                account="default",
                telegram_inline_buttons=True,
                whatsapp_use_poll=False,
            )
        ]
        runtime.config.routes_source = "test"

        second = runtime.poll_once()
        self.assertEqual(second["notified"], 1)
        self.assertEqual(len(messenger.text_messages), 1)
        self.assertTrue(runtime.pending_by_approval["approval-123"].notified)

    def test_seen_pending_is_rehydrated_and_notified_again(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ]
        )
        runtime.seen_approvals.add("approval-123")

        poll_result = runtime.poll_once()
        self.assertEqual(poll_result["notified"], 1)
        self.assertEqual(len(messenger.text_messages), 1)
        self.assertIn("approval-123", runtime.pending_by_approval)

    def test_whatsapp_poll_vote_resolves_and_wait_returns_approved(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="whatsapp",
                    target="+15555550123",
                    allow_from=["+15555550123"],
                    account="default",
                    telegram_inline_buttons=False,
                    whatsapp_use_poll=True,
                )
            ]
        )

        runtime.poll_once()
        self.assertEqual(len(messenger.poll_messages), 1)
        short_id = messenger.poll_messages[0]["options"][0].split()[1]

        inbound = {
            "channelId": "whatsapp",
            "accountId": "default",
            "conversationId": "+15555550123",
            "from": "+15555550123",
            "content": "",
            "metadata": {
                "senderE164": "+15555550123",
                "pollOption": f"approve {short_id}",
            },
        }

        result = runtime.handle_inbound_event(inbound)
        self.assertEqual(result["status"], "resolved")
        self.assertEqual(result["decision"], "approve")
        self.assertEqual(result["approval_id"], "approval-123")

        self.assertIn(("approve", "approval-123"), MockAgentRulerHandler.decisions)

        wait_result = runtime.client.wait_for_approval("approval-123", timeout_seconds=5, poll_ms=250)
        self.assertEqual(wait_result["status"], "approved")

    def test_telegram_callback_data_metadata_resolves_approval(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ]
        )
        runtime.poll_once()
        self.assertEqual(len(messenger.text_messages), 1)
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]

        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
        }
        result = runtime.handle_inbound_event(inbound)
        self.assertEqual(result["status"], "resolved")
        self.assertEqual(result["decision"], "approve")
        self.assertEqual(result["approval_id"], "approval-123")
        self.assertIn(("approve", "approval-123"), MockAgentRulerHandler.decisions)

        wait_result = runtime.client.wait_for_approval("approval-123", timeout_seconds=5, poll_ms=250)
        self.assertEqual(wait_result["status"], "approved")

    def test_telegram_callback_data_metadata_denies_approval(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ]
        )
        runtime.poll_once()
        self.assertEqual(len(messenger.text_messages), 1)
        callback = messenger.text_messages[0]["telegram_buttons"][0][1]["callback_data"]

        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
        }
        result = runtime.handle_inbound_event(inbound)
        self.assertEqual(result["status"], "resolved")
        self.assertEqual(result["decision"], "deny")
        self.assertEqual(result["approval_id"], "approval-123")
        self.assertIn(("deny", "approval-123"), MockAgentRulerHandler.decisions)

        wait_result = runtime.client.wait_for_approval("approval-123", timeout_seconds=5, poll_ms=250)
        self.assertEqual(wait_result["status"], "denied")

    def test_telegram_callback_feedback_replies_in_same_thread(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="-10055667788",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ]
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]

        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "-10055667788",
            "from": "123456789",
            "content": "",
            "metadata": {
                "callback_data": callback,
                "callback_query_id": "cbq-thread-77",
                "message_thread_id": 77,
            },
        }
        result = runtime.handle_inbound_event(inbound)
        self.assertEqual(result["status"], "resolved")
        self.assertGreaterEqual(len(messenger.text_messages), 2)
        self.assertEqual(messenger.text_messages[-1]["target"], "-10055667788")
        self.assertEqual(messenger.text_messages[-1]["message_thread_id"], 77)
        self.assertEqual(len(messenger.callback_answers), 1)
        self.assertIn("Processing approval decision", messenger.callback_answers[0]["text"])

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_telegram_callback_prefers_api_even_with_cli_configured(self, run_mock):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ],
            agent_ruler_bin="agent-ruler",
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]
        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
        }
        result = runtime.handle_inbound_event(inbound)
        self.assertEqual(result["status"], "resolved")
        self.assertEqual(MockAgentRulerHandler.decisions[-1], ("approve", "approval-123"))
        run_mock.assert_not_called()

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_telegram_callback_uses_cli_fallback_when_api_fails(self, run_mock):
        run_mock.side_effect = [
            CompletedProcess(
                args=[],
                returncode=0,
                stdout="approved approval-123\n",
                stderr="",
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps({"approval_id": "approval-123", "status": "approved"}),
                stderr="",
            ),
        ]
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ],
            agent_ruler_bin="agent-ruler",
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]
        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
        }
        with patch.object(runtime.client, "approve", side_effect=BridgeError("api down")):
            result = runtime.handle_inbound_event(inbound)

        self.assertEqual(result["status"], "resolved")
        self.assertEqual(run_mock.call_count, 2)
        approve_cmd = run_mock.call_args_list[0].args[0]
        self.assertIn("approve", approve_cmd)
        self.assertIn("--decision", approve_cmd)
        self.assertIn("approval-123", approve_cmd)
        wait_cmd = run_mock.call_args_list[1].args[0]
        self.assertIn("wait", wait_cmd)
        self.assertIn("approval-123", wait_cmd)

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_cli_fallback_errors_when_no_approvals_are_matched(self, run_mock):
        run_mock.return_value = CompletedProcess(
            args=[],
            returncode=0,
            stdout="no approvals matched\n",
            stderr="",
        )
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ],
            agent_ruler_bin="agent-ruler",
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]
        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
        }

        with patch.object(runtime.client, "approve", side_effect=BridgeError("api down")):
            result = runtime.handle_inbound_event(inbound)

        self.assertEqual(result["status"], "error")
        self.assertIn("no approvals matched", result["reason"])
        self.assertIn("approval-123", runtime.pending_by_approval)
        run_mock.assert_called_once()

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_cli_fallback_errors_when_wait_still_pending(self, run_mock):
        run_mock.side_effect = [
            CompletedProcess(
                args=[],
                returncode=0,
                stdout="approved approval-123\n",
                stderr="",
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps({"approval_id": "approval-123", "status": "timeout"}),
                stderr="",
            ),
        ]
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ],
            agent_ruler_bin="agent-ruler",
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]
        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
        }

        with patch.object(runtime.client, "approve", side_effect=BridgeError("api down")):
            result = runtime.handle_inbound_event(inbound)

        self.assertEqual(result["status"], "error")
        self.assertIn("still pending", result["reason"])
        self.assertIn("approval-123", runtime.pending_by_approval)
        self.assertEqual(run_mock.call_count, 2)

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_cli_fallback_treats_already_approved_as_resolved(self, run_mock):
        run_mock.return_value = CompletedProcess(
            args=[],
            returncode=1,
            stdout="",
            stderr="error: approval approval-123 is not pending (status: Approved)",
        )
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ],
            agent_ruler_bin="agent-ruler",
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]
        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
        }

        with patch.object(runtime.client, "approve", side_effect=BridgeError("api down")):
            result = runtime.handle_inbound_event(inbound)

        self.assertEqual(result["status"], "resolved")
        self.assertEqual(result["decision"], "approve")
        self.assertEqual(result["approval_id"], "approval-123")
        run_mock.assert_called_once()

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_api_duplicate_resolution_skips_cli_fallback(self, run_mock):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ],
            agent_ruler_bin="agent-ruler",
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]
        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
        }

        with patch.object(
            runtime.client,
            "approve",
            side_effect=BridgeError(
                "POST /api/approvals/approval-123/approve failed (400): "
                '{"error":"approval approval-123 is not pending (status: Approved)"}'
            ),
        ):
            result = runtime.handle_inbound_event(inbound)

        self.assertEqual(result["status"], "resolved")
        self.assertEqual(result["decision"], "approve")
        self.assertEqual(result["approval_id"], "approval-123")
        run_mock.assert_not_called()

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_repeated_callback_after_resolution_is_ignored_without_cli_fallback(self, run_mock):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ],
            agent_ruler_bin="agent-ruler",
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]
        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
        }

        first = runtime.handle_inbound_event(inbound)
        second = runtime.handle_inbound_event(inbound)

        self.assertEqual(first["status"], "resolved")
        self.assertEqual(second["status"], "ignored")
        self.assertEqual(second["reason"], "approval already resolved")
        run_mock.assert_not_called()

    def test_suppressed_channel_reply_sends_direct_feedback(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ]
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]
        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {
                "callback_data": callback,
                "callback_query_id": "cbq-suppressed-1",
            },
            "suppress_channel_reply": True,
            "messageId": "msg-suppressed-1",
        }

        result = runtime.handle_inbound_event(inbound)
        self.assertEqual(result["status"], "resolved")
        self.assertEqual(result.get("feedback_message"), None)
        # First message is the approval card; second is final decision feedback.
        self.assertEqual(len(messenger.text_messages), 2)
        self.assertIn("Approved", messenger.text_messages[-1]["message"])
        self.assertEqual(len(messenger.callback_answers), 1)
        self.assertIn("Processing approval decision", messenger.callback_answers[0]["text"])

    def test_suppressed_channel_reply_falls_back_to_hook_feedback(self):
        runtime, messenger = self._runtime(
            [
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                )
            ]
        )
        runtime.poll_once()
        callback = messenger.text_messages[0]["telegram_buttons"][0][0]["callback_data"]
        inbound = {
            "channelId": "telegram",
            "accountId": "default",
            "conversationId": "123456789",
            "from": "123456789",
            "content": "",
            "metadata": {"callback_data": callback},
            "suppress_channel_reply": True,
        }

        with patch.object(messenger, "send_text", side_effect=BridgeError("send failed")):
            result = runtime.handle_inbound_event(inbound)

        self.assertEqual(result["status"], "resolved")
        self.assertIn("Approved", result.get("feedback_message", ""))


class BridgeConfigLoadTests(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory()
        self.config_path = Path(self.tempdir.name) / "bridge.json"

    def tearDown(self):
        self.tempdir.cleanup()

    def _write_config(self, payload):
        self.config_path.write_text(json.dumps(payload), encoding="utf-8")

    def _args(self, **overrides):
        defaults = {
            "ruler_url": None,
            "public_base_url": None,
            "poll_interval": None,
            "inbound_bind": None,
            "state_file": None,
            "openclaw_bin": None,
            "openclaw_home": None,
            "agent_ruler_bin": None,
            "dry_run_send": False,
        }
        defaults.update(overrides)
        return argparse.Namespace(**defaults)

    def _write_allow_from(self, channel, account, values):
        credentials = Path(self.tempdir.name) / ".openclaw" / "credentials"
        credentials.mkdir(parents=True, exist_ok=True)
        payload = {"version": 1, "allowFrom": values}
        (credentials / f"{channel}-{account}-allowFrom.json").write_text(
            json.dumps(payload),
            encoding="utf-8",
        )

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_load_config_uses_openclaw_route_config_when_routes_omitted(self, run_mock):
        self._write_config(
            {
                "ruler_url": "http://127.0.0.1:4622",
                "public_base_url": "http://127.0.0.1:4622",
                "openclaw_home": "/tmp/managed-openclaw-home",
            }
        )
        run_mock.return_value = CompletedProcess(
            args=[],
            returncode=0,
            stdout=json.dumps(
                [
                    {
                        "channel": "telegram",
                        "target": "123456789",
                        "allow_from": ["123456789"],
                        "account": "default",
                        "telegram_inline_buttons": True,
                    }
                ]
            ),
            stderr="",
        )

        config = load_config(self.config_path, self._args())
        self.assertEqual(len(config.routes), 1)
        self.assertEqual(config.routes[0].channel, "telegram")
        self.assertEqual(config.routes[0].allow_from, ["123456789"])

        kwargs = run_mock.call_args.kwargs
        self.assertEqual(kwargs["env"]["OPENCLAW_HOME"], "/tmp/managed-openclaw-home")

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_load_config_discovers_openclaw_home_from_agent_ruler_status(self, run_mock):
        self._write_config(
            {
                "ruler_url": "http://127.0.0.1:4622",
                "public_base_url": "http://127.0.0.1:4622",
            }
        )
        run_mock.side_effect = [
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(
                    {
                        "runner": {
                            "kind": "openclaw",
                            "managed_home": "/tmp/discovered-openclaw-home",
                        }
                    }
                ),
                stderr="",
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(
                    [
                        {
                            "channel": "whatsapp",
                            "target": "+15555550123",
                            "allow_from": ["+15555550123"],
                        }
                    ]
                ),
                stderr="",
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps({}),
                stderr="",
            ),
        ]

        config = load_config(self.config_path, self._args())
        self.assertEqual(config.routes[0].channel, "whatsapp")
        self.assertEqual(config.routes[0].allow_from, ["+15555550123"])

        second_call_kwargs = run_mock.call_args_list[1].kwargs
        self.assertEqual(
            second_call_kwargs["env"]["OPENCLAW_HOME"],
            "/tmp/discovered-openclaw-home",
        )

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_load_config_allows_startup_when_openclaw_routes_are_missing(self, run_mock):
        self._write_config(
            {
                "ruler_url": "http://127.0.0.1:4622",
                "public_base_url": "http://127.0.0.1:4622",
                "openclaw_home": "/tmp/managed-openclaw-home",
            }
        )
        run_mock.return_value = CompletedProcess(
            args=[],
            returncode=0,
            stdout="null\n",
            stderr="",
        )

        config = load_config(self.config_path, self._args())
        self.assertEqual(config.routes_source, "openclaw_unconfigured")
        self.assertEqual(config.routes, [])

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_load_config_autodiscovers_telegram_route_from_channel_defaults(self, run_mock):
        self._write_allow_from("telegram", "default", ["123456789"])
        self._write_config(
            {
                "ruler_url": "http://127.0.0.1:4622",
                "public_base_url": "http://127.0.0.1:4622",
                "openclaw_home": self.tempdir.name,
            }
        )
        run_mock.side_effect = [
            CompletedProcess(
                args=[],
                returncode=1,
                stdout="",
                stderr=(
                    "Config path not found: "
                    "plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes"
                ),
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps({"telegram": {"enabled": True}}),
                stderr="",
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps({"ok": True}),
                stderr="",
            ),
        ]

        config = load_config(self.config_path, self._args())
        self.assertEqual(config.routes_source, "openclaw_managed_config")
        self.assertEqual(len(config.routes), 1)
        self.assertEqual(config.routes[0].channel, "telegram")
        self.assertEqual(config.routes[0].target, "123456789")
        self.assertEqual(config.routes[0].allow_from, ["123456789"])
        self.assertTrue(config.routes[0].telegram_inline_buttons)

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_load_config_autodiscovers_telegram_route_from_channel_allow_from(self, run_mock):
        self._write_config(
            {
                "ruler_url": "http://127.0.0.1:4622",
                "public_base_url": "http://127.0.0.1:4622",
                "openclaw_home": self.tempdir.name,
            }
        )
        run_mock.side_effect = [
            CompletedProcess(
                args=[],
                returncode=1,
                stdout="",
                stderr=(
                    "Config path not found: "
                    "plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes"
                ),
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(
                    {"telegram": {"enabled": True, "allowFrom": [123456789]}}
                ),
                stderr="",
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps({"ok": True}),
                stderr="",
            ),
        ]

        config = load_config(self.config_path, self._args())
        self.assertEqual(config.routes_source, "openclaw_managed_config")
        self.assertEqual(len(config.routes), 1)
        self.assertEqual(config.routes[0].channel, "telegram")
        self.assertEqual(config.routes[0].target, "123456789")
        self.assertEqual(config.routes[0].allow_from, ["123456789"])

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_load_config_autosyncs_discovered_channel_defaults_into_openclaw_config(self, run_mock):
        self._write_allow_from("telegram", "default", ["123456789"])
        self._write_config(
            {
                "ruler_url": "http://127.0.0.1:4622",
                "public_base_url": "http://127.0.0.1:4622",
                "openclaw_home": self.tempdir.name,
            }
        )
        run_mock.side_effect = [
            CompletedProcess(
                args=[],
                returncode=1,
                stdout="",
                stderr=(
                    "Config path not found: "
                    "plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes"
                ),
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps({"telegram": {"enabled": True}}),
                stderr="",
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps({"ok": True}),
                stderr="",
            ),
        ]

        config = load_config(self.config_path, self._args())
        self.assertEqual(config.routes_source, "openclaw_managed_config")
        self.assertEqual(len(config.routes), 1)
        self.assertEqual(config.routes[0].channel, "telegram")
        self.assertEqual(config.routes[0].target, "123456789")

        set_call = run_mock.call_args_list[2]
        set_cmd = set_call.args[0]
        self.assertEqual(
            set_cmd[:4],
            [
                "openclaw",
                "config",
                "set",
                "plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes",
            ],
        )

    @patch("bridge.openclaw.channel_bridge.subprocess.run")
    def test_load_config_backfills_telegram_streaming_flag_from_channel_defaults(self, run_mock):
        self._write_config(
            {
                "ruler_url": "http://127.0.0.1:4622",
                "public_base_url": "http://127.0.0.1:4622",
                "openclaw_home": self.tempdir.name,
            }
        )
        run_mock.side_effect = [
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(
                    [
                        {
                            "channel": "telegram",
                            "target": "123456789",
                            "allow_from": ["123456789"],
                            "account": "default",
                            "telegram_inline_buttons": True,
                        }
                    ]
                ),
                stderr="",
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(
                    {
                        "telegram": {
                            "enabled": True,
                            "allowFrom": [123456789],
                            "streaming": True,
                        }
                    }
                ),
                stderr="",
            ),
            CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps({"ok": True}),
                stderr="",
            ),
        ]

        config = load_config(self.config_path, self._args())
        self.assertEqual(config.routes_source, "openclaw_managed_config")
        self.assertEqual(len(config.routes), 1)
        self.assertTrue(config.routes[0].telegram_streaming_enabled)

        set_call = run_mock.call_args_list[2]
        set_cmd = set_call.args[0]
        set_payload = json.loads(set_cmd[4])
        self.assertEqual(len(set_payload), 1)
        self.assertTrue(set_payload[0]["telegram_streaming_enabled"])


if __name__ == "__main__":
    unittest.main()
