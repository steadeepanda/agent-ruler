#!/usr/bin/env python3
"""Manual smoke replay for OpenClaw channel bridge.

Runs a local mock Agent Ruler API, emits one pending approval notification,
and replays a sample inbound WhatsApp poll vote locally.
"""

from __future__ import annotations

import json
import re
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from bridge.openclaw.channel_bridge import (  # noqa: E402
    AgentRulerClient,
    ApprovalBridgeRuntime,
    BridgeConfig,
    OpenClawMessenger,
    RouteConfig,
)


class MockHandler(BaseHTTPRequestHandler):
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


class PrintMessenger(OpenClawMessenger):
    def __init__(self):
        super().__init__(openclaw_bin="openclaw", dry_run_send=True)
        self.messages = []

    def send_text(self, *, channel, target, message, account=None, telegram_buttons=None):
        payload = {
            "kind": "text",
            "channel": channel,
            "target": target,
            "account": account,
            "message": message,
            "telegram_buttons": telegram_buttons,
        }
        self.messages.append(payload)
        print("\n--- outbound text ---")
        print(json.dumps(payload, indent=2))
        return payload

    def send_poll(self, *, channel, target, question, options, account=None):
        payload = {
            "kind": "poll",
            "channel": channel,
            "target": target,
            "account": account,
            "question": question,
            "options": list(options),
        }
        self.messages.append(payload)
        print("\n--- outbound poll ---")
        print(json.dumps(payload, indent=2))
        return payload


def main() -> int:
    MockHandler.decisions = []
    MockHandler.approval_status = "pending"

    server = ThreadingHTTPServer(("127.0.0.1", 0), MockHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    base_url = f"http://127.0.0.1:{server.server_address[1]}"
    with tempfile.TemporaryDirectory() as tmpdir:
        config = BridgeConfig(
            ruler_url=base_url,
            public_base_url=base_url,
            poll_interval_seconds=1,
            decision_ttl_seconds=3600,
            inbound_bind="127.0.0.1:4661",
            state_file=Path(tmpdir) / "bridge-state.json",
            openclaw_bin="openclaw",
            dry_run_send=True,
            short_id_length=6,
            routes=[
                RouteConfig(
                    channel="telegram",
                    target="123456789",
                    allow_from=["123456789"],
                    account="default",
                    telegram_inline_buttons=True,
                    whatsapp_use_poll=False,
                ),
                RouteConfig(
                    channel="whatsapp",
                    target="+15555550123",
                    allow_from=["+15555550123"],
                    account="default",
                    telegram_inline_buttons=False,
                    whatsapp_use_poll=True,
                ),
            ],
        )

        runtime = ApprovalBridgeRuntime(config, AgentRulerClient(base_url), PrintMessenger())

        print("\n[1/4] Polling mock Agent Ruler for pending approvals")
        print(runtime.poll_once())

        whatsapp_text = next(
            (msg for msg in runtime.messenger.messages if msg.get("kind") == "text" and msg["channel"] == "whatsapp"),
            None,
        )
        if whatsapp_text is None:
            raise RuntimeError("expected WhatsApp fallback text message")

        match = re.search(r"short id:\s+([A-Z2-9]{4,10})", whatsapp_text["message"])
        if not match:
            raise RuntimeError("could not find short id in outbound message")
        short_id = match.group(1)

        print("\n[2/4] Replaying sample inbound WhatsApp poll vote")
        inbound_path = Path("bridge/openclaw/samples/inbound-whatsapp-poll.json")
        sample = json.loads(inbound_path.read_text(encoding="utf-8"))
        sample["metadata"]["pollOption"] = f"approve {short_id}"
        result = runtime.handle_inbound_event(sample)
        print(json.dumps(result, indent=2))

        print("\n[3/4] Checking wait endpoint after approval")
        wait_result = runtime.client.wait_for_approval("approval-123", timeout_seconds=5, poll_ms=250)
        print(json.dumps(wait_result, indent=2))

        print("\n[4/4] Final mock decisions")
        print(MockHandler.decisions)

        ok = (
            ("approve", "approval-123") in MockHandler.decisions
            and wait_result.get("status") == "approved"
        )
        print("\nSMOKE RESULT:", "PASS" if ok else "FAIL")

    server.shutdown()
    server.server_close()
    thread.join(timeout=2)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
