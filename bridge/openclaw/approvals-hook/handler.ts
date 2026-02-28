const DEFAULT_BRIDGE_URL = "http://127.0.0.1:4661/inbound";

type MessageContext = {
  channelId?: string;
  accountId?: string;
  conversationId?: string;
  messageId?: string;
  from?: string;
  content?: string;
  metadata?: Record<string, unknown>;
};

type HookEvent = {
  type?: string;
  action?: string;
  messages?: string[];
  context?: MessageContext;
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

  try {
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
  } catch (error) {
    console.error("[agent-ruler-approvals-hook] failed to forward inbound message", error);
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
