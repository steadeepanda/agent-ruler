const DEFAULT_BRIDGE_URL = "http://127.0.0.1:4661/inbound";
const CALLBACK_APPROVAL_PATTERN =
  /^ar:(approve|deny):([A-Za-z0-9._:-]+)(?::[A-Za-z0-9._~-]+)?$/i;
const COMMAND_APPROVAL_PATTERN =
  /^\/?(approve|deny)\s+([A-Za-z0-9._:-]+)(?:\s+[A-Za-z0-9._~-]+)?\s*$/i;
const CONTROL_APPROVAL_PATTERN =
  /^\/?ar(approve|deny)\s+([A-Za-z0-9._:-]+)(?:\s+[A-Za-z0-9._~-]+)?\s*$/i;

type MessageContext = {
  channelId?: string;
  accountId?: string;
  conversationId?: string;
  messageId?: string;
  callbackQueryId?: string;
  callback_query_id?: string;
  queryId?: string;
  query_id?: string;
  from?: string;
  content?: string;
  metadata?: Record<string, unknown>;
};

type HookEvent = {
  type?: string;
  action?: string;
  messages?: string[];
  context?: MessageContext;
  callbackQueryId?: string;
  callback_query_id?: string;
  queryId?: string;
  query_id?: string;
  content?: string;
  metadata?: Record<string, unknown>;
  callbackData?: string;
  callback_data?: string;
};

export default async function handler(event: HookEvent) {
  const context = event.context ?? {};
  const endpoint = process.env.AR_OPENCLAW_BRIDGE_URL || DEFAULT_BRIDGE_URL;
  const metadata: Record<string, unknown> = {
    ...(event.metadata ?? {}),
    ...(context.metadata ?? {}),
  };
  const callbackData =
    pickString(
      context.metadata?.callback_data,
      context.metadata?.callbackData,
      context.metadata?.data,
      event.callback_data,
      event.callbackData
    ) || "";
  if (callbackData) {
    metadata.callback_data = callbackData;
  }
  const callbackQueryId = extractCallbackQueryId(event, context, metadata);
  if (callbackQueryId) {
    metadata.callback_query_id = callbackQueryId;
  }
  if (event?.action) {
    metadata.event_action = event.action;
  }

  const channelId =
    pickString(
      context.channelId,
      readMetadataString(metadata, "channelId"),
      readMetadataString(metadata, "channel"),
      readMetadataString(metadata, "provider")
    ) || "";
  const accountId =
    pickString(
      context.accountId,
      readMetadataString(metadata, "accountId"),
      readMetadataString(metadata, "account")
    ) || "";
  const conversationId =
    pickString(
      context.conversationId,
      readMetadataString(metadata, "conversationId"),
      readMetadataString(metadata, "threadId"),
      readMetadataString(metadata, "chatId")
    ) || "";
  const from =
    pickString(
      context.from,
      readMetadataString(metadata, "from"),
      readMetadataString(metadata, "senderId"),
      readMetadataString(metadata, "sender_id"),
      readMetadataString(metadata, "userId"),
      readMetadataString(metadata, "user_id"),
      readMetadataString(metadata, "telegramUserId")
    ) || "";
  const messageId =
    pickString(
      context.messageId,
      readMetadataString(metadata, "messageId"),
      readMetadataString(metadata, "message_id")
    ) || "";

  const payload = {
    channelId,
    accountId,
    conversationId,
    messageId,
    from,
    content: pickString(context.content, event.content) || "",
    metadata,
    sync: true,
    suppress_channel_reply: true,
  };
  const approvalCommands = collectApprovalCommands(event, callbackData, payload.content);
  const isApprovalCommandInput = approvalCommands.length > 0;

  try {
    if (isApprovalCommandInput) {
      for (const command of approvalCommands) {
        const commandMetadata: Record<string, unknown> = { ...metadata };
        if (CALLBACK_APPROVAL_PATTERN.test(command)) {
          commandMetadata.callback_data = command;
        }
        await fetch(endpoint, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            ...payload,
            content: command,
            metadata: commandMetadata,
          }),
        });
      }
    } else {
      const response = await fetch(endpoint, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
      });
      const result = await safeReadJson(response);
      const feedback = pickString(result?.feedback_message);
      if (feedback && Array.isArray(event.messages)) {
        event.messages.push(feedback);
      }
    }
  } catch (error) {
    console.error("[agent-ruler-approvals-hook] failed to forward inbound message", error);
  } finally {
    if (isApprovalCommandInput) {
      consumeInboundApprovalCommand(event);
    }
  }
}

function collectApprovalCommands(
  event: HookEvent,
  callbackData: string,
  payloadContent: string
): string[] {
  const candidates: string[] = [
    callbackData,
    payloadContent,
    pickString(event.content) || "",
    pickString(event.context?.content) || "",
  ];
  if (Array.isArray(event.messages)) {
    for (const message of event.messages) {
      if (typeof message === "string") {
        candidates.push(message);
      }
    }
  }
  const seen = new Set<string>();
  const commands: string[] = [];
  for (const candidate of candidates) {
    for (const command of extractApprovalDecisionCommands(candidate)) {
      const key = command.toLowerCase();
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      commands.push(command);
    }
  }
  return commands;
}

function extractApprovalDecisionCommands(value: string): string[] {
  const commands: string[] = [];
  const raw = pickString(value) || "";
  if (!raw) {
    return commands;
  }
  if (isApprovalDecisionCommand(raw)) {
    commands.push(raw);
  }
  for (const line of raw.split(/\r?\n/g)) {
    const candidate = line.trim();
    if (!candidate) {
      continue;
    }
    if (isApprovalDecisionCommand(candidate)) {
      commands.push(candidate);
    }
  }
  return commands;
}

function isApprovalDecisionCommand(value: string): boolean {
  const text = value.trim();
  if (!text) {
    return false;
  }
  return (
    CALLBACK_APPROVAL_PATTERN.test(text) ||
    COMMAND_APPROVAL_PATTERN.test(text) ||
    CONTROL_APPROVAL_PATTERN.test(text)
  );
}

function consumeInboundApprovalCommand(event: HookEvent): void {
  if (Array.isArray(event.messages)) {
    event.messages.length = 0;
  }
  if (typeof event.content === "string") {
    event.content = "";
  }
  if (event.context && typeof event.context === "object") {
    event.context.content = "";
  }
}

function pickString(...values: unknown[]): string | null {
  for (const value of values) {
    if (typeof value !== "string") continue;
    const trimmed = value.trim();
    if (trimmed) return trimmed;
  }
  return null;
}

function readMetadataString(metadata: Record<string, unknown>, key: string): string | null {
  return pickString(metadata[key]);
}

function extractCallbackQueryId(
  event: HookEvent,
  context: MessageContext,
  metadata: Record<string, unknown>
): string {
  const direct = pickString(
    context.callback_query_id,
    context.callbackQueryId,
    context.query_id,
    context.queryId,
    event.callback_query_id,
    event.callbackQueryId,
    event.query_id,
    event.queryId,
    readMetadataString(metadata, "callback_query_id"),
    readMetadataString(metadata, "callbackQueryId"),
    readMetadataString(metadata, "query_id"),
    readMetadataString(metadata, "queryId")
  );
  if (direct) {
    return direct;
  }

  return (
    readNestedString(metadata, ["callbackQuery", "id"]) ||
    readNestedString(metadata, ["callback_query", "id"]) ||
    readNestedString(metadata, ["interaction", "id"]) ||
    ""
  );
}

function readNestedString(obj: Record<string, unknown>, path: string[]): string | null {
  let current: unknown = obj;
  for (const segment of path) {
    if (!current || typeof current !== "object" || Array.isArray(current)) {
      return null;
    }
    current = (current as Record<string, unknown>)[segment];
  }
  return pickString(current);
}

async function safeReadJson(response: Response): Promise<Record<string, unknown>> {
  try {
    const raw = await response.text();
    if (!raw.trim()) return {};
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
    return {};
  } catch {
    return {};
  }
}
