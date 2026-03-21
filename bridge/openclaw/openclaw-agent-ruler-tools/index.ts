const PLUGIN_ID = "openclaw-agent-ruler-tools";

type JsonObject = Record<string, unknown>;
type ToolResult = { content: Array<{ type: "text"; text: string }> };
type OptionalToolDefinition = {
  name: string;
  description: string;
  parameters: JsonObject;
  execute: (_id: string, params?: JsonObject) => Promise<ToolResult>;
};
type ToolPreflightResponse = {
  status?: string;
  blocked?: boolean;
  reason?: string;
  detail?: string;
  approval_id?: string;
};
type ApprovalWaitResponse = {
  resolved?: boolean;
  timeout?: boolean;
  event?: {
    approval_id?: string;
    verdict?: string;
    reason_code?: string;
    guidance?: string;
    updated_at?: string;
  };
};
type ApprovalWaitOutcome = {
  status: "approved" | "denied" | "expired" | "pending" | "timeout" | "unknown";
  approvalId: string;
  response: ApprovalWaitResponse;
};
const MISSING_EXPORT_SOURCE_SNIPPET = "export source does not exist:";
const UUID_LIKE_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

export default function registerAgentRulerTools(api: any) {
  registerApprovalQueueBypassCommands(api);

  registerOptionalTool(api, {
    name: "agent_ruler_capabilities",
    description:
      "Read the Agent Ruler safe runtime/capabilities contract before boundary operations.",
    parameters: {
      type: "object",
      properties: {},
      additionalProperties: false,
    },
    async execute() {
      const data = await callAgentRulerJson(api, "GET", "/api/capabilities");
      return asTextResult(data);
    },
  });

  registerOptionalTool(api, {
    name: "agent_ruler_status_feed",
    description:
      "Read Agent Ruler redacted approval status feed for safe polling in autonomous loops.",
    parameters: {
      type: "object",
      properties: {
        include_resolved: { type: "boolean", default: false },
        limit: { type: "integer", minimum: 1, maximum: 500, default: 100 },
      },
      additionalProperties: false,
    },
    async execute(_id: string, params: JsonObject = {}) {
      const includeResolved =
        typeof params.include_resolved === "boolean"
          ? params.include_resolved
          : false;
      const limitRaw = Number(params.limit ?? 100);
      const limit = Number.isFinite(limitRaw)
        ? Math.min(Math.max(Math.floor(limitRaw), 1), 500)
        : 100;
      const data = await callAgentRulerJson(
        api,
        "GET",
        `/api/status/feed?include_resolved=${includeResolved}&limit=${limit}`
      );
      return asTextResult(data);
    },
  });

  registerOptionalTool(api, {
    name: "agent_ruler_wait_for_approval",
    description:
      "Wait for an Agent Ruler approval decision by id. Returns resolved/timeout plus redacted event.",
    parameters: {
      type: "object",
      properties: {
        approval_id: { type: "string" },
        timeout: { type: "integer", minimum: 1, maximum: 300, default: 90 },
        poll_ms: { type: "integer", minimum: 100, maximum: 2000, default: 500 },
      },
      required: ["approval_id"],
      additionalProperties: false,
    },
    async execute(_id: string, params: JsonObject = {}) {
      const approvalId = String(params.approval_id ?? "").trim();
      if (!approvalId) {
        throw new Error("approval_id is required");
      }
      if (!isLikelyApprovalId(approvalId)) {
        throw new Error(
          "approval_id looks invalid. Use the full Agent Ruler approval id from API responses or status feed (short references are not supported here)."
        );
      }

      const timeoutDefault = resolveApprovalWaitTimeoutSecs(api);
      const timeoutRaw = Number(params.timeout ?? timeoutDefault);
      const pollRaw = Number(params.poll_ms ?? 500);
      const timeout = Number.isFinite(timeoutRaw)
        ? Math.min(Math.max(Math.floor(timeoutRaw), 1), 300)
        : timeoutDefault;
      const pollMs = Number.isFinite(pollRaw)
        ? Math.min(Math.max(Math.floor(pollRaw), 100), 2000)
        : 500;

      const data = await callAgentRulerJson(
        api,
        "GET",
        `/api/approvals/${encodeURIComponent(
          approvalId
        )}/wait?timeout=${timeout}&poll_ms=${pollMs}`,
        undefined,
        resolveWaitRequestTimeoutMs(api, timeout)
      );
      return asTextResult(data);
    },
  });

  registerOptionalTool(api, {
    name: "agent_ruler_request_export_stage",
    description:
      "Request stage export via Agent Ruler (/api/export/request). Returns staged, blocked, or pending_approval. Omit dst when no destination is specified so Agent Ruler uses runtime defaults.",
    parameters: {
      type: "object",
      properties: {
        src: { type: "string" },
        dst: { type: "string" },
      },
      required: ["src"],
      additionalProperties: false,
    },
    async execute(_id: string, params: JsonObject = {}) {
      const src = String(params.src ?? "").trim();
      if (!src) {
        throw new Error("src is required");
      }
      const body: JsonObject = { src };
      if (typeof params.dst === "string" && params.dst.trim()) {
        body.dst = params.dst.trim();
      }

      return executeApprovalAwareRequest(api, "/api/export/request", body);
    },
  });

  registerOptionalTool(api, {
    name: "agent_ruler_request_delivery",
    description:
      "Request delivery via Agent Ruler (/api/export/deliver/request). Returns delivered, blocked, or pending_approval. Omit dst when user destination is unspecified so Agent Ruler uses its default user destination directory.",
    parameters: {
      type: "object",
      properties: {
        stage_ref: { type: "string" },
        dst: { type: "string" },
        move_artifact: { type: "boolean", default: false },
      },
      required: ["stage_ref"],
      additionalProperties: false,
    },
    async execute(_id: string, params: JsonObject = {}) {
      const stageRef = String(params.stage_ref ?? "").trim();
      if (!stageRef) {
        throw new Error("stage_ref is required");
      }
      const body: JsonObject = { stage_ref: stageRef };
      if (typeof params.dst === "string" && params.dst.trim()) {
        body.dst = params.dst.trim();
      }
      if (typeof params.move_artifact === "boolean") {
        body.move_artifact = params.move_artifact;
      }

      return executeDeliveryRequestWithAutoStage(api, stageRef, body);
    },
  });

  registerOptionalTool(api, {
    name: "agent_ruler_request_import",
    description:
      "Request import via Agent Ruler (/api/import/request). Returns completed, blocked, or pending_approval.",
    parameters: {
      type: "object",
      properties: {
        src: { type: "string" },
        dst: { type: "string" },
      },
      required: ["src"],
      additionalProperties: false,
    },
    async execute(_id: string, params: JsonObject = {}) {
      const src = String(params.src ?? "").trim();
      if (!src) {
        throw new Error("src is required");
      }
      const body: JsonObject = { src };
      if (typeof params.dst === "string" && params.dst.trim()) {
        body.dst = params.dst.trim();
      }

      return executeApprovalAwareRequest(api, "/api/import/request", body);
    },
  });

  registerToolPreflightHook(api);
}

function registerOptionalTool(api: any, tool: OptionalToolDefinition) {
  api.registerTool(tool, { optional: true });
}

function registerApprovalQueueBypassCommands(api: any) {
  if (typeof api?.registerCommand !== "function") {
    return;
  }

  for (const commandName of ["arapprove", "ardeny"]) {
    api.registerCommand({
      name: commandName,
      description:
        "Internal Agent Ruler approval callback command used by Telegram inline buttons.",
      acceptsArgs: true,
      requireAuth: false,
      handler: async () => undefined as any,
    });
  }
}

function registerToolPreflightHook(api: any) {
  if (typeof api?.on !== "function") {
    return;
  }

  api.on("before_tool_call", async (event: any, ctx: any) => {
    const toolName = resolveHookToolName(event, ctx);
    if (!toolName || toolName.startsWith("agent_ruler_")) {
      return;
    }

    const requestBody: JsonObject = {
      tool_name: toolName,
      params: resolveHookToolParams(event),
      context: {
        agent_id: typeof ctx?.agentId === "string" ? ctx.agentId : undefined,
        session_key: typeof ctx?.sessionKey === "string" ? ctx.sessionKey : undefined,
      },
    };

    try {
      const response = (await callAgentRulerJson(
        api,
        "POST",
        "/api/openclaw/tool/preflight",
        requestBody
      )) as ToolPreflightResponse;

      if (response?.blocked) {
        if (
          resolveAutoWaitForApprovals(api) &&
          response.status === "pending_approval" &&
          typeof response.approval_id === "string" &&
          response.approval_id.trim()
        ) {
          const wait = await waitForApprovalDecision(
            api,
            response.approval_id.trim(),
            resolveApprovalWaitTimeoutSecs(api)
          );
          if (wait.status === "approved") {
            logToolHookInfo(
              api,
              `approval ${wait.approvalId} approved; resuming blocked tool ${toolName}`
            );
            return;
          }
          return {
            block: true,
            blockReason: buildPendingApprovalWaitFailureReason(toolName, wait),
          };
        }
        return {
          block: true,
          blockReason: buildToolBlockReason(toolName, response),
        };
      }
    } catch (err) {
      const detail = (err as Error).message || String(err);
      logToolHookWarning(api, `tool preflight unavailable for ${toolName}: ${detail}`);
      return {
        block: true,
        blockReason: `Agent Ruler preflight unavailable; blocked ${toolName} to preserve policy enforcement (${detail})`,
      };
    }
  });
}

function resolveHookToolName(event: any, ctx: any): string {
  for (const candidate of [
    event?.toolName,
    event?.tool_name,
    event?.tool,
    event?.name,
    ctx?.toolName,
    ctx?.tool_name,
    ctx?.tool,
    ctx?.name,
  ]) {
    if (typeof candidate !== "string") {
      continue;
    }
    const normalized = candidate.trim().toLowerCase();
    if (normalized) {
      return normalized;
    }
  }
  return "";
}

function resolveHookToolParams(event: any): JsonObject {
  return asObject(event?.params ?? event?.arguments ?? event?.args ?? event?.input);
}

function buildToolBlockReason(
  toolName: string,
  response: ToolPreflightResponse
): string {
  const status = String(response.status || "").toLowerCase();
  const reason = typeof response.reason === "string" ? response.reason : "";
  const detail = typeof response.detail === "string" ? response.detail : "";
  const approvalId =
    typeof response.approval_id === "string" ? response.approval_id : "";

  if (status === "pending_approval") {
    const suffix = approvalId ? ` (approval_id=${approvalId})` : "";
    return `Agent Ruler approval required for ${toolName}${suffix}.${buildApprovalWorkflowHint(
      approvalId
    )}`;
  }
  if (reason && detail) {
    return `Agent Ruler blocked ${toolName}: ${reason} (${detail})${buildBoundaryWorkflowHint(
      reason
    )}`;
  }
  if (detail) {
    return `Agent Ruler blocked ${toolName}: ${detail}${buildBoundaryWorkflowHint(reason)}`;
  }
  return `Agent Ruler blocked ${toolName}${buildBoundaryWorkflowHint(reason)}`;
}

function buildPendingApprovalWaitFailureReason(
  toolName: string,
  wait: ApprovalWaitOutcome
): string {
  if (wait.status === "denied" || wait.status === "expired") {
    return `Agent Ruler approval ${wait.approvalId} ${wait.status}; ${toolName} remains blocked`;
  }
  if (wait.status === "timeout" || wait.status === "pending") {
    return `Agent Ruler approval ${wait.approvalId} still pending after wait timeout; ${toolName} remains blocked`;
  }
  return `Agent Ruler approval ${wait.approvalId} unresolved; ${toolName} remains blocked`;
}

function buildApprovalWorkflowHint(approvalId: string): string {
  const suffix = approvalId ? ` (approval_id=${approvalId})` : "";
  return ` Read agent_ruler_capabilities if you have not done so, then wait for operator resolution${suffix} with agent_ruler_wait_for_approval; do not loop-retry the same blocked action.`;
}

function buildBoundaryWorkflowHint(reasonCode: string): string {
  const reason = String(reasonCode || "").toLowerCase();
  if (reason === "deny_user_data_write") {
    return " Use stage+deliver/import Agent Ruler tools for cross-zone transfers instead of direct destination writes. If the user did not specify a destination, omit dst so Agent Ruler uses its default user destination directory.";
  }
  if (reason === "deny_system_critical" || reason === "deny_secrets") {
    return " Stay within workspace-safe paths, read agent_ruler_capabilities, and use Agent Ruler transfer tools.";
  }
  return " Follow Agent Ruler safe runtime workflow for boundary operations and use agent_ruler_capabilities for runtime discovery instead of guessing.";
}

async function executeApprovalAwareRequest(
  api: any,
  path: string,
  body: JsonObject
): Promise<ToolResult> {
  const initial = await callAgentRulerJson(api, "POST", path, body);
  const initialApprovalId = extractPendingApprovalId(initial);
  if (!initialApprovalId || !resolveAutoWaitForApprovals(api)) {
    return asTextResult(initial);
  }

  const wait = await waitForApprovalDecision(
    api,
    initialApprovalId,
    resolveApprovalWaitTimeoutSecs(api)
  );
  if (wait.status !== "approved") {
    return asTextResult({
      ...(asObject(initial) as JsonObject),
      wait_status: wait.status,
      resumed_from_approval_id: initialApprovalId,
      resumed: false,
      status: wait.status === "timeout" ? "pending_approval" : wait.status,
    });
  }

  // Request endpoints are already applied by Agent Ruler when approval resolves.
  // Re-posting the same request can create duplicate approvals/operations.
  const resolvedPayload: JsonObject = {
    ...(asObject(initial) as JsonObject),
    resumed_from_approval_id: initialApprovalId,
    resumed: false,
    wait_status: "approved",
    status: "approved",
    approval_resolution: asObject(wait.response),
  };

  return asTextResult(resolvedPayload);
}

async function executeDeliveryRequestWithAutoStage(
  api: any,
  stageRef: string,
  body: JsonObject
): Promise<ToolResult> {
  try {
    return await executeApprovalAwareRequest(api, "/api/export/deliver/request", body);
  } catch (err) {
    const detail = (err as Error)?.message || String(err);
    if (!shouldAutoStageBeforeDelivery(detail, stageRef)) {
      throw err;
    }

    const stageResult = await executeApprovalAwareRequest(api, "/api/export/request", {
      src: stageRef,
    });
    const retriedResult = await executeApprovalAwareRequest(
      api,
      "/api/export/deliver/request",
      body
    );
    return withAutoStageMetadata(retriedResult, stageRef, stageResult);
  }
}

function shouldAutoStageBeforeDelivery(errorDetail: string, stageRef: string): boolean {
  const detail = String(errorDetail || "").toLowerCase();
  const reference = String(stageRef || "").trim();
  if (!detail.includes(MISSING_EXPORT_SOURCE_SNIPPET)) {
    return false;
  }
  if (!reference || UUID_LIKE_PATTERN.test(reference)) {
    return false;
  }
  return true;
}

function withAutoStageMetadata(
  deliveryResult: ToolResult,
  stageRef: string,
  stageResult: ToolResult
): ToolResult {
  const deliveryPayload = parseToolResultPayload(deliveryResult);
  const stagePayload = parseToolResultPayload(stageResult);
  if (!deliveryPayload) {
    return deliveryResult;
  }
  return asTextResult({
    ...deliveryPayload,
    auto_staged_from: stageRef,
    auto_stage_result: stagePayload ?? {},
  });
}

function parseToolResultPayload(result: ToolResult): JsonObject | null {
  const text = result?.content?.[0]?.text;
  if (typeof text !== "string" || !text.trim()) {
    return null;
  }
  try {
    const parsed = JSON.parse(text);
    return asObject(parsed);
  } catch {
    return null;
  }
}

function extractPendingApprovalId(payload: unknown): string {
  const obj = asObject(payload);
  const status = String(obj.status ?? "").toLowerCase();
  const approvalId = String(obj.approval_id ?? "").trim();
  if (status !== "pending_approval" || !approvalId) {
    return "";
  }
  return approvalId;
}

async function waitForApprovalDecision(
  api: any,
  approvalId: string,
  timeoutSecs: number
): Promise<ApprovalWaitOutcome> {
  const timeout = Math.min(Math.max(Math.floor(timeoutSecs), 1), 300);
  const response = (await callAgentRulerJson(
    api,
    "GET",
    `/api/approvals/${encodeURIComponent(
      approvalId
    )}/wait?timeout=${timeout}&poll_ms=500`,
    undefined,
    resolveWaitRequestTimeoutMs(api, timeout)
  )) as ApprovalWaitResponse;

  const verdict = String(response?.event?.verdict ?? "").toLowerCase();
  if (response?.resolved) {
    if (verdict === "approved") {
      return { status: "approved", approvalId, response };
    }
    if (verdict === "denied") {
      return { status: "denied", approvalId, response };
    }
    if (verdict === "expired") {
      return { status: "expired", approvalId, response };
    }
    return { status: "unknown", approvalId, response };
  }

  if (response?.timeout) {
    return { status: "timeout", approvalId, response };
  }
  if (verdict === "pending") {
    return { status: "pending", approvalId, response };
  }
  return { status: "unknown", approvalId, response };
}

function asObject(value: unknown): JsonObject {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return value as JsonObject;
  }
  return {};
}

function logToolHookWarning(api: any, message: string) {
  const logger = api?.logger;
  if (logger && typeof logger.warn === "function") {
    logger.warn(message);
    return;
  }
  console.warn(`[${PLUGIN_ID}] ${message}`);
}

function logToolHookInfo(api: any, message: string) {
  const logger = api?.logger;
  if (logger && typeof logger.info === "function") {
    logger.info(message);
    return;
  }
  console.log(`[${PLUGIN_ID}] ${message}`);
}

async function callAgentRulerJson(
  api: any,
  method: "GET" | "POST",
  path: string,
  body?: JsonObject,
  timeoutMsOverride?: number
): Promise<unknown> {
  const controller = new AbortController();
  const timeoutMs = timeoutMsOverride ?? resolveTimeoutMs(api);
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  const url = `${resolveBaseUrl(api)}${path}`;

  try {
    const res = await fetch(url, {
      method,
      headers: { "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined,
      signal: controller.signal,
    });

    const text = await res.text();
    const payload = parsePayload(text);
    if (!res.ok) {
      const detail =
        (payload as JsonObject)?.error ??
        (payload as JsonObject)?.detail ??
        JSON.stringify(payload);
      throw new Error(`${method} ${path} failed (${res.status}): ${detail}`);
    }
    return payload;
  } catch (err) {
    if ((err as Error).name === "AbortError") {
      throw new Error(`${method} ${path} timed out after ${timeoutMs}ms`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}

function resolveWaitRequestTimeoutMs(api: any, waitTimeoutSecs: number): number {
  const base = resolveTimeoutMs(api);
  const waitMs = Math.min(
    Math.max(Math.floor(waitTimeoutSecs), 1) * 1000 + 5000,
    600000
  );
  return Math.max(base, waitMs);
}

function parsePayload(text: string): unknown {
  if (!text.trim()) return {};
  try {
    return JSON.parse(text);
  } catch {
    return { raw: text };
  }
}

function isLikelyApprovalId(value: string): boolean {
  const trimmed = String(value || "").trim();
  if (trimmed.length < 8) {
    return false;
  }
  return /^[A-Za-z0-9-]+$/.test(trimmed);
}

function resolveBaseUrl(api: any): string {
  const fromEnv = process.env.AGENT_RULER_BASE_URL;
  const fromPluginConfig = api?.config?.plugins?.entries?.[PLUGIN_ID]?.config?.baseUrl;
  const raw = String(fromEnv || fromPluginConfig || "http://127.0.0.1:4622").trim();
  return raw.endsWith("/") ? raw.slice(0, -1) : raw;
}

function resolveTimeoutMs(api: any): number {
  const raw = Number(api?.config?.plugins?.entries?.[PLUGIN_ID]?.config?.timeoutMs ?? 10000);
  if (!Number.isFinite(raw)) return 10000;
  return Math.min(Math.max(Math.floor(raw), 1000), 60000);
}

function resolveApprovalWaitTimeoutSecs(api: any): number {
  const fromEnv = Number(process.env.AGENT_RULER_APPROVAL_WAIT_TIMEOUT_SECS ?? "");
  const fromPluginConfig = Number(
    api?.config?.plugins?.entries?.[PLUGIN_ID]?.config?.approvalWaitTimeoutSecs ?? ""
  );
  const raw = Number.isFinite(fromEnv) && fromEnv > 0 ? fromEnv : fromPluginConfig;
  if (!Number.isFinite(raw) || raw <= 0) return 90;
  return Math.min(Math.max(Math.floor(raw), 1), 300);
}

function resolveAutoWaitForApprovals(api: any): boolean {
  const fromEnv = process.env.AGENT_RULER_AUTO_WAIT_FOR_APPROVALS;
  if (typeof fromEnv === "string" && fromEnv.trim()) {
    const value = fromEnv.trim().toLowerCase();
    if (value === "0" || value === "false" || value === "no") {
      return false;
    }
    return true;
  }

  const raw = api?.config?.plugins?.entries?.[PLUGIN_ID]?.config?.autoWaitForApprovals;
  if (typeof raw === "boolean") {
    return raw;
  }
  return true;
}

function asTextResult(payload: unknown): ToolResult {
  return {
    content: [{ type: "text", text: JSON.stringify(payload, null, 2) }],
  };
}
