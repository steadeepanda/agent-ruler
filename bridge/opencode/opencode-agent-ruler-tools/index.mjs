const DEFAULT_BASE_URL = "http://127.0.0.1:4622";
const DEFAULT_TIMEOUT_MS = 10_000;
const DEFAULT_APPROVAL_WAIT_TIMEOUT_SECS = 90;

export default async function agentRulerOpenCodeGovernancePlugin(_input) {
  return {
    "tool.execute.before": async (input, output) => {
      const toolName = normalizeToolName(input?.tool);
      if (!toolName || toolName.startsWith("agent_ruler_")) {
        return;
      }

      const params = asRecord(output?.args);
      const requestBody = {
        tool_name: toolName,
        params,
        context: {
          session_key: asString(input?.sessionID),
        },
      };

      let response;
      try {
        response = await callAgentRulerJson(
          "POST",
          "/api/opencode/tool/preflight",
          requestBody,
        );
      } catch (error) {
        const detail = asErrorMessage(error);
        throw new Error(
          `Agent Ruler preflight unavailable; blocked ${toolName} to preserve policy enforcement (${detail}). Remain in workspace-safe mode until Agent Ruler connectivity is restored.`,
        );
      }

      if (!asBoolean(response?.blocked, false)) {
        return;
      }

      if (
        shouldAutoWaitForApprovals() &&
        asString(response?.status) === "pending_approval" &&
        asString(response?.approval_id)
      ) {
        const approvalId = asString(response.approval_id);
        const wait = await waitForApprovalDecision(approvalId);
        if (wait.status === "approved") {
          return;
        }
        throw new Error(buildPendingApprovalFailureMessage(toolName, approvalId, wait.status));
      }

      throw new Error(buildToolBlockedMessage(toolName, response));
    },
  };
}

async function waitForApprovalDecision(approvalId) {
  const timeoutSecs = resolveApprovalWaitTimeoutSecs();
  const waitResponse = await callAgentRulerJson(
    "GET",
    `/api/approvals/${encodeURIComponent(approvalId)}/wait?timeout=${timeoutSecs}&poll_ms=500`,
    undefined,
    resolveWaitTimeoutMs(timeoutSecs),
  );
  const event = asRecord(waitResponse?.event);
  const verdict = (
    asString(waitResponse?.verdict) || asString(event?.verdict)
  ).toLowerCase();
  const resolved = asBoolean(
    waitResponse?.resolved,
    asBoolean(event?.resolved, false),
  );
  const timeout = asBoolean(
    waitResponse?.timeout,
    asBoolean(event?.timeout, false),
  );
  if (resolved) {
    if (verdict === "approved") return { status: "approved" };
    if (verdict === "denied") return { status: "denied" };
    if (verdict === "expired") return { status: "expired" };
    return { status: "unknown" };
  }
  if (timeout) return { status: "timeout" };
  return { status: "pending" };
}

function buildPendingApprovalFailureMessage(toolName, approvalId, status) {
  const approvalHint = buildApprovalWorkflowHint(approvalId);
  if (status === "denied" || status === "expired") {
    return `Agent Ruler approval ${approvalId} ${status}; ${toolName} remains blocked.${approvalHint}`;
  }
  if (status === "timeout" || status === "pending") {
    const waitTimeout = resolveApprovalWaitTimeoutSecs();
    return `Agent Ruler approval ${approvalId} is still pending after waiting ${waitTimeout}s; ${toolName} remains blocked until operator resolution.${approvalHint}`;
  }
  return `Agent Ruler approval ${approvalId} unresolved; ${toolName} remains blocked.${approvalHint}`;
}

function buildToolBlockedMessage(toolName, response) {
  const status = asString(response?.status).toLowerCase();
  const reason = asString(response?.reason);
  const detail = asString(response?.detail);
  const approvalId = asString(response?.approval_id);

  if (status === "pending_approval") {
    const suffix = approvalId ? ` (approval_id=${approvalId})` : "";
    return `Agent Ruler approval required for ${toolName}${suffix}.${buildApprovalWorkflowHint(approvalId)}`;
  }
  if (reason && detail) {
    return `Agent Ruler blocked ${toolName}: ${reason} (${detail})${buildBoundaryWorkflowHint(reason)}`;
  }
  if (detail) {
    return `Agent Ruler blocked ${toolName}: ${detail}${buildBoundaryWorkflowHint(reason)}`;
  }
  return `Agent Ruler blocked ${toolName}${buildBoundaryWorkflowHint(reason)}`;
}

function buildApprovalWorkflowHint(approvalId) {
  const id = asString(approvalId);
  const suffix = id ? ` (approval_id=${id})` : "";
  return ` Read agent_ruler_capabilities if you have not done so, then wait for operator resolution${suffix} with agent_ruler_wait_for_approval; do not loop-retry the same blocked action.`;
}

function buildBoundaryWorkflowHint(reasonCode) {
  const reason = asString(reasonCode).toLowerCase();
  if (reason === "deny_user_data_write") {
    return " Use stage+deliver/import Agent Ruler tools for cross-zone transfers instead of direct destination writes. If the user did not specify a destination, omit dst so Agent Ruler uses its default user destination directory.";
  }
  if (reason === "deny_system_critical" || reason === "deny_secrets") {
    return " Stay within workspace-safe paths, read agent_ruler_capabilities, and use Agent Ruler transfer tools.";
  }
  return " Follow Agent Ruler safe runtime workflow for boundary operations and use agent_ruler_capabilities for runtime discovery instead of guessing.";
}

async function callAgentRulerJson(method, path, body, timeoutMsOverride) {
  const timeoutMs = timeoutMsOverride ?? resolveTimeoutMs();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  try {
    const response = await fetch(`${resolveBaseUrl()}${path}`, {
      method,
      headers: { "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined,
      signal: controller.signal,
    });
    const text = await response.text();
    const payload = parseJsonPayload(text);
    if (!response.ok) {
      const detail = asString(payload?.error) || asString(payload?.detail) || text;
      throw new Error(`${method} ${path} failed (${response.status}): ${detail}`);
    }
    return payload;
  } catch (error) {
    if (error?.name === "AbortError") {
      throw new Error(`${method} ${path} timed out after ${timeoutMs}ms`);
    }
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

function parseJsonPayload(text) {
  if (!asString(text).trim()) return {};
  try {
    return JSON.parse(text);
  } catch {
    return { raw: text };
  }
}

function shouldAutoWaitForApprovals() {
  const env = asString(process?.env?.AGENT_RULER_AUTO_WAIT_FOR_APPROVALS).toLowerCase();
  if (env) {
    return env !== "0" && env !== "false" && env !== "no";
  }
  return true;
}

function resolveApprovalWaitTimeoutSecs() {
  const env = Number(process?.env?.AGENT_RULER_APPROVAL_WAIT_TIMEOUT_SECS);
  if (Number.isFinite(env) && env > 0) {
    return clampInteger(env, 1, 300);
  }
  return DEFAULT_APPROVAL_WAIT_TIMEOUT_SECS;
}

function resolveTimeoutMs() {
  const env = Number(process?.env?.AGENT_RULER_TIMEOUT_MS);
  if (Number.isFinite(env) && env > 0) {
    return clampInteger(env, 1_000, 60_000);
  }
  return DEFAULT_TIMEOUT_MS;
}

function resolveWaitTimeoutMs(waitTimeoutSecs) {
  const baseline = resolveTimeoutMs();
  const waitMs = clampInteger(waitTimeoutSecs * 1000 + 5_000, 1_000, 600_000);
  return Math.max(baseline, waitMs);
}

function resolveBaseUrl() {
  const fromEnv = asString(process?.env?.AGENT_RULER_BASE_URL);
  const raw = fromEnv || DEFAULT_BASE_URL;
  return raw.endsWith("/") ? raw.slice(0, -1) : raw;
}

function normalizeToolName(value) {
  return asString(value).trim().toLowerCase();
}

function asBoolean(value, defaultValue) {
  return typeof value === "boolean" ? value : defaultValue;
}

function asString(value) {
  return typeof value === "string" ? value : "";
}

function asRecord(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }
  return value;
}

function asErrorMessage(error) {
  if (error instanceof Error && error.message) return error.message;
  return String(error);
}

function clampInteger(value, min, max) {
  return Math.max(min, Math.min(max, Math.floor(value)));
}
