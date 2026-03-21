import test from "node:test";
import assert from "node:assert/strict";

import handler from "./handler.ts";

test("consumes callback approval commands so they are not queued as chat input", async () => {
  const calls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init });
    return new Response(
      JSON.stringify({
        status: "resolved",
      }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      }
    );
  };

  try {
    const event = {
      action: "message",
      messages: ["ar:approve:KCNKY4"],
      content: "ar:approve:KCNKY4",
      context: {
        channelId: "telegram",
        accountId: "default",
        conversationId: "-10012345",
        from: "988088319",
        content: "ar:approve:KCNKY4",
        metadata: {
          callback_data: "ar:approve:KCNKY4",
        },
      },
      metadata: {},
    };

    await handler(event);

    assert.equal(calls.length, 1);
    assert.equal(event.messages.length, 0);
    assert.equal(event.content, "");
    assert.equal(event.context.content, "");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("forwards callback_query_id when callback approvals are forwarded", async () => {
  const calls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init });
    return new Response(JSON.stringify({ status: "resolved" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  try {
    const event = {
      action: "message",
      messages: ["ar:approve:KCNKY4"],
      content: "ar:approve:KCNKY4",
      context: {
        channelId: "telegram",
        accountId: "default",
        conversationId: "-10012345",
        from: "988088319",
        content: "ar:approve:KCNKY4",
        callbackQueryId: "cbq-forward-42",
        metadata: {
          callback_data: "ar:approve:KCNKY4",
        },
      },
      metadata: {},
    };

    await handler(event);

    assert.equal(calls.length, 1);
    const payload = JSON.parse(String(calls[0]?.init?.body ?? "{}"));
    assert.equal(payload?.metadata?.callback_query_id, "cbq-forward-42");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("consumes text approval commands so they are not queued as chat input", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () =>
    new Response(JSON.stringify({ status: "resolved" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });

  try {
    const event = {
      action: "message",
      messages: ["approve KCNKY4"],
      content: "approve KCNKY4",
      context: {
        channelId: "telegram",
        accountId: "default",
        conversationId: "-10012345",
        from: "988088319",
        content: "approve KCNKY4",
        metadata: {},
      },
      metadata: {},
    };

    await handler(event);

    assert.equal(event.messages.length, 0);
    assert.equal(event.content, "");
    assert.equal(event.context.content, "");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("consumes plugin control approval commands so they are not queued as chat input", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () =>
    new Response(JSON.stringify({ status: "resolved" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });

  try {
    const event = {
      action: "message",
      messages: ["/arapprove KCNKY4"],
      content: "/arapprove KCNKY4",
      context: {
        channelId: "telegram",
        accountId: "default",
        conversationId: "-10012345",
        from: "988088319",
        content: "/arapprove KCNKY4",
        metadata: {
          callback_data: "/arapprove KCNKY4",
        },
      },
      metadata: {},
    };

    await handler(event);

    assert.equal(event.messages.length, 0);
    assert.equal(event.content, "");
    assert.equal(event.context.content, "");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("consumes queued wrapper text and forwards embedded approval commands", async () => {
  const calls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init });
    return new Response(JSON.stringify({ status: "resolved" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  try {
    const wrapped = `[Queued messages while agent was busy]

---
Queued #1
ar:approve:KCNKY4

---
Queued #2
approve KAZ7MJ`;
    const event = {
      action: "message",
      messages: [wrapped],
      content: wrapped,
      context: {
        channelId: "telegram",
        accountId: "default",
        conversationId: "-10012345",
        from: "988088319",
        content: wrapped,
        metadata: {},
      },
      metadata: {},
    };

    await handler(event);

    assert.equal(event.messages.length, 0);
    assert.equal(event.content, "");
    assert.equal(event.context.content, "");
    assert.equal(calls.length, 2);
    const payloads = calls.map((entry) => JSON.parse(String(entry.init?.body ?? "{}")));
    const forwarded = payloads.map((p) => p.content).sort();
    assert.deepEqual(forwarded, ["approve KAZ7MJ", "ar:approve:KCNKY4"]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("does not consume regular chat messages", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () =>
    new Response(JSON.stringify({ status: "ignored", reason: "not an approval command" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });

  try {
    const event = {
      action: "message",
      messages: ["hello bot"],
      content: "hello bot",
      context: {
        channelId: "telegram",
        accountId: "default",
        conversationId: "-10012345",
        from: "988088319",
        content: "hello bot",
        metadata: {},
      },
      metadata: {},
    };

    await handler(event);

    assert.deepEqual(event.messages, ["hello bot"]);
    assert.equal(event.content, "hello bot");
    assert.equal(event.context.content, "hello bot");
  } finally {
    globalThis.fetch = originalFetch;
  }
});
