# OpenClaw Agent Ruler Tools Plugin

Local OpenClaw plugin that exposes deterministic Agent Ruler tools:

- `agent_ruler_capabilities`
- `agent_ruler_status_feed`
- `agent_ruler_wait_for_approval`
- `agent_ruler_request_export_stage`
- `agent_ruler_request_delivery`
- `agent_ruler_request_import`

The plugin also registers a `before_tool_call` hook for core file/exec tools (`write/edit/delete/move/read/exec`) and asks Agent Ruler for preflight policy decisions so blocked actions show up in receipts/timeline with reason codes.
When preflight returns `pending_approval`, the hook automatically waits for resolution (bounded timeout) and resumes the blocked tool call after approval.
If preflight API calls are unavailable, the hook fails closed (blocks the tool call) to preserve policy enforcement.
Agents should read `agent_ruler_capabilities` before boundary operations and use the runtime contract it returns instead of guessing paths or approval semantics.
For delivery requests, `dst` is optional. When user destination is not explicitly specified, omit `dst` so Agent Ruler uses the runtime default user destination directory.

Recommended companion skill text:

- `bridge/openclaw/openclaw-agent-ruler-tools/skills/agent-ruler-safe-runtime.md`

## Configure in OpenClaw

Use the same config keys described in OpenClaw docs:
- <https://docs.openclaw.ai/plugins>
- <https://docs.openclaw.ai/plugins/guides/plugin-agent-tools>
- <https://docs.openclaw.ai/cli/commands/config>

```bash
AR_DIR="$HOME/agent-ruler"
```

```bash
openclaw config set plugins.load.paths "[\"$AR_DIR/bridge/openclaw/openclaw-agent-ruler-tools\"]" --json
openclaw config set plugins.entries.openclaw-agent-ruler-tools.enabled true
openclaw config set plugins.entries.openclaw-agent-ruler-tools.config.baseUrl "http://127.0.0.1:4622"
openclaw config set plugins.entries.openclaw-agent-ruler-tools.config.approvalWaitTimeoutSecs "90"
openclaw config set plugins.entries.openclaw-agent-ruler-tools.config.autoWaitForApprovals true
openclaw config set agents.list[0].tools.allow "[\"openclaw-agent-ruler-tools\"]" --json
```

Optional environment override:

```bash
export AGENT_RULER_BASE_URL="http://127.0.0.1:4622"
export AGENT_RULER_APPROVAL_WAIT_TIMEOUT_SECS="90"
export AGENT_RULER_AUTO_WAIT_FOR_APPROVALS="true"
```

## Sanity check

```bash
node "$AR_DIR/bridge/openclaw/openclaw-agent-ruler-tools/sanity-check.mjs"
```
