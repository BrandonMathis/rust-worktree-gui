// Tests for the SDK-event normalizers. They import no SDK, so they run with
// plain `node --test` and no `npm ci`.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  CLIP,
  ClaudeNormalizer,
  PiNormalizer,
  claudeHistory,
  claudeModels,
  clip,
  codexEvents,
  codexModels,
  piHistory,
  piModels,
  resultText,
} from "../normalize.mjs";

const all = (normalizer, messages) => messages.flatMap((message) => normalizer.events(message));

test("long strings are clipped wherever they sit in a tool's input", () => {
  const long = "x".repeat(CLIP + 50);
  const clipped = clip({ content: long, nested: [{ text: long }], count: 3 });
  assert.equal(clipped.content.length, CLIP + 1);
  assert.ok(clipped.content.endsWith("…"));
  assert.equal(clipped.nested[0].text.length, CLIP + 1);
  assert.equal(clipped.count, 3);
  assert.equal(clip("short"), "short");
});

test("tool results are read from strings and from content blocks", () => {
  assert.equal(resultText("plain"), "plain");
  assert.equal(resultText([{ type: "text", text: "a" }, { type: "image" }, { type: "text", text: "b" }]), "a\nb");
  assert.equal(resultText({ content: [{ type: "text", text: "nested" }] }), "nested");
  assert.equal(resultText(undefined), "");
});

test("Claude: text and thinking stream as deltas, tools as calls then results", () => {
  const normalizer = new ClaudeNormalizer();
  const stream = (event) => ({ type: "stream_event", parent_tool_use_id: null, event });
  const events = all(normalizer, [
    stream({ type: "message_start", message: { id: "msg_1" } }),
    stream({ type: "content_block_start", index: 0, content_block: { type: "thinking" } }),
    stream({ type: "content_block_delta", index: 0, delta: { type: "thinking_delta", thinking: "Plan" } }),
    stream({ type: "content_block_start", index: 1, content_block: { type: "text" } }),
    stream({ type: "content_block_delta", index: 1, delta: { type: "text_delta", text: "Hi" } }),
    stream({ type: "content_block_start", index: 2, content_block: { type: "tool_use", id: "toolu_1", name: "Read" } }),
    {
      type: "assistant",
      parent_tool_use_id: null,
      message: {
        id: "msg_1",
        content: [
          { type: "text", text: "Hi" },
          { type: "tool_use", id: "toolu_1", name: "Read", input: { file_path: "/w/a.rs" } },
        ],
      },
    },
    {
      type: "user",
      parent_tool_use_id: null,
      message: { role: "user", content: [{ type: "tool_result", tool_use_id: "toolu_1", content: "1\tfn main() {}" }] },
    },
    { type: "result", subtype: "success", is_error: false },
  ]);
  assert.deepEqual(events, [
    { type: "thinking", id: "msg_1:0", delta: "" },
    { type: "thinking", id: "msg_1:0", delta: "Plan" },
    { type: "text", id: "msg_1:1", delta: "Hi" },
    { type: "tool", id: "toolu_1", name: "Read", status: "running" },
    { type: "tool", id: "toolu_1", name: "Read", input: { file_path: "/w/a.rs" }, status: "running" },
    { type: "tool", id: "toolu_1", status: "done", output: "1\tfn main() {}" },
    { type: "turn_end", status: "done" },
  ]);
});

test("Claude: subagent chatter is dropped and failed tools are marked", () => {
  const normalizer = new ClaudeNormalizer();
  const events = all(normalizer, [
    { type: "assistant", parent_tool_use_id: "toolu_parent", message: { id: "sub", content: [{ type: "text", text: "inner" }] } },
    {
      type: "user",
      parent_tool_use_id: null,
      message: { role: "user", content: [{ type: "tool_result", tool_use_id: "t", is_error: true, content: [{ type: "text", text: "denied" }] }] },
    },
    { type: "system", subtype: "init" },
  ]);
  assert.deepEqual(events, [{ type: "tool", id: "t", status: "failed", output: "denied" }]);
});

test("Claude: the session id is reported when it first appears and when it changes", () => {
  const normalizer = new ClaudeNormalizer();
  const events = all(normalizer, [
    { type: "system", subtype: "init", parent_tool_use_id: null, session_id: "s1" },
    { type: "result", subtype: "success", is_error: false, session_id: "s1" },
    { type: "system", subtype: "init", parent_tool_use_id: null, session_id: "s2" },
  ]);
  assert.deepEqual(events, [
    { type: "session", id: "s1" },
    { type: "turn_end", status: "done" },
    { type: "session", id: "s2" },
  ]);
});

test("Claude: a message that never streamed (an API error) still shows its text", () => {
  const normalizer = new ClaudeNormalizer();
  const events = normalizer.events({
    type: "assistant",
    parent_tool_use_id: null,
    message: { id: "synthetic", content: [{ type: "text", text: "API Error: overloaded" }] },
  });
  assert.deepEqual(events, [{ type: "text", id: "synthetic:0", text: "API Error: overloaded" }]);
});

test("Claude: a turn ends as failed with its errors, or as interrupted after Stop", () => {
  const normalizer = new ClaudeNormalizer();
  assert.deepEqual(normalizer.events({ type: "result", subtype: "error_max_turns", is_error: true, errors: ["Too many turns"] }), [
    { type: "turn_end", status: "failed", error: "Too many turns" },
  ]);
  assert.deepEqual(normalizer.events({ type: "result", subtype: "error_during_execution", is_error: true }), [
    { type: "turn_end", status: "failed", error: "error during execution" },
  ]);
  normalizer.interrupting = true;
  assert.deepEqual(normalizer.events({ type: "result", subtype: "error_during_execution", is_error: true }), [
    { type: "turn_end", status: "interrupted" },
  ]);
  // The flag covers one turn only.
  assert.deepEqual(normalizer.events({ type: "result", subtype: "success", is_error: false }), [
    { type: "turn_end", status: "done" },
  ]);
});

test("Codex: items become text, thinking and tool rows", () => {
  const item = (type, item) => codexEvents({ type, item });
  assert.deepEqual(item("item.completed", { id: "item_0", type: "agent_message", text: "Done" }), [
    { type: "text", id: "item_0", text: "Done" },
  ]);
  assert.deepEqual(item("item.updated", { id: "item_1", type: "reasoning", text: "Checking" }), [
    { type: "thinking", id: "item_1", text: "Checking" },
  ]);
  assert.deepEqual(
    item("item.started", { id: "item_2", type: "command_execution", command: "ls", aggregated_output: "", status: "in_progress" }),
    [{ type: "tool", id: "item_2", name: "shell", input: { command: "ls" }, status: "running" }],
  );
  assert.deepEqual(
    item("item.completed", { id: "item_2", type: "command_execution", command: "ls", aggregated_output: "a\n", exit_code: 1, status: "failed" }),
    [{ type: "tool", id: "item_2", name: "shell", input: { command: "ls" }, status: "failed", output: "a\n" }],
  );
  assert.deepEqual(
    item("item.completed", { id: "item_3", type: "file_change", changes: [{ path: "a.txt", kind: "add" }], status: "completed" }),
    [{ type: "tool", id: "item_3", name: "apply_patch", input: { changes: [{ path: "a.txt", kind: "add" }] }, status: "done" }],
  );
  assert.deepEqual(
    item("item.completed", { id: "item_4", type: "mcp_tool_call", server: "github", tool: "search", arguments: { q: "x" }, status: "failed", error: { message: "401" } }),
    [{ type: "tool", id: "item_4", name: "mcp__github__search", input: { q: "x" }, status: "failed", output: "401" }],
  );
  // Items without their own status are running until they complete.
  assert.equal(item("item.started", { id: "item_5", type: "web_search", query: "iced" })[0].status, "running");
  assert.equal(item("item.completed", { id: "item_5", type: "web_search", query: "iced" })[0].status, "done");
  assert.deepEqual(item("item.completed", { id: "item_6", type: "error", message: "Sandbox denied" }), [
    { type: "notice", text: "Sandbox denied" },
  ]);
});

test("Codex: turns end as done or failed", () => {
  assert.deepEqual(codexEvents({ type: "turn.completed", usage: {} }), [{ type: "turn_end", status: "done" }]);
  assert.deepEqual(codexEvents({ type: "turn.failed", error: { message: "quota" } }), [
    { type: "turn_end", status: "failed", error: "quota" },
  ]);
  // An error is only a note: a retry keeps the turn going, and a fatal one is
  // followed by `turn.failed`.
  assert.deepEqual(codexEvents({ type: "error", message: "Reconnecting… 1/5" }), [
    { type: "notice", text: "Reconnecting… 1/5" },
  ]);
  assert.deepEqual(codexEvents({ type: "thread.started", thread_id: "t" }), [{ type: "session", id: "t" }]);
});

test("Pi: message updates stream as deltas and tool executions as rows", () => {
  const normalizer = new PiNormalizer();
  const update = (assistantMessageEvent) => ({ type: "message_update", assistantMessageEvent });
  const events = all(normalizer, [
    { type: "message_start", message: { role: "user" } },
    { type: "message_start", message: { role: "assistant" } },
    update({ type: "thinking_start", contentIndex: 0 }),
    update({ type: "thinking_delta", contentIndex: 0, delta: "Hmm" }),
    update({ type: "text_delta", contentIndex: 1, delta: "Hello" }),
    update({ type: "toolcall_delta", contentIndex: 2, delta: "{" }),
    { type: "tool_execution_start", toolCallId: "c1", toolName: "bash", args: { command: "ls" } },
    { type: "tool_execution_end", toolCallId: "c1", toolName: "bash", isError: false, result: { content: [{ type: "text", text: "a.txt" }] } },
    { type: "tool_execution_end", toolCallId: "c2", toolName: "read", isError: true, result: { content: [{ type: "text", text: "ENOENT" }] } },
  ]);
  assert.deepEqual(events, [
    { type: "thinking", id: "m2:0", delta: "" },
    { type: "thinking", id: "m2:0", delta: "Hmm" },
    { type: "text", id: "m2:1", delta: "Hello" },
    { type: "tool", id: "c1", name: "bash", input: { command: "ls" }, status: "running" },
    { type: "tool", id: "c1", name: "bash", status: "done", output: "a.txt" },
    { type: "tool", id: "c2", name: "read", status: "failed", output: "ENOENT" },
  ]);
});

test("Pi: only the error of the last message counts, since Pi retries by itself", () => {
  const normalizer = new PiNormalizer();
  normalizer.events({ type: "message_start", message: { role: "assistant" } });
  normalizer.events({ type: "message_end", message: { role: "assistant", stopReason: "error", errorMessage: "overloaded" } });
  assert.equal(normalizer.lastError, "overloaded");
  normalizer.events({ type: "message_start", message: { role: "assistant" } });
  normalizer.events({ type: "message_end", message: { role: "assistant", stopReason: "stop" } });
  assert.equal(normalizer.lastError, null);
});

test("model lists share one shape across the three harnesses", () => {
  assert.deepEqual(
    claudeModels([
      { value: "default", displayName: "Default", description: "Opus", supportedEffortLevels: ["low", "high"] },
      { value: "haiku-old", displayName: "Haiku 4.5", description: "Fast" },
    ]),
    {
      default_model: "default",
      models: [
        { id: "default", name: "Default", description: "Opus", thinking: ["low", "high"], default_thinking: null },
        { id: "haiku-old", name: "Haiku 4.5", description: "Fast", thinking: [], default_thinking: null },
      ],
    },
  );

  const codex = codexModels({
    models: [
      { slug: "b", display_name: "B", visibility: "list", priority: 2, supported_reasoning_levels: [{ effort: "low" }], default_reasoning_level: "low" },
      { slug: "hidden", visibility: "hide", priority: 0 },
      { slug: "a", display_name: "A", description: "Go-to", visibility: "list", priority: 1, supported_reasoning_levels: [{ effort: "medium" }, { effort: "high" }], default_reasoning_level: "medium" },
    ],
  });
  assert.equal(codex.default_model, "a");
  assert.deepEqual(codex.models.map((model) => model.id), ["a", "b"]);
  assert.deepEqual(codex.models[0], { id: "a", name: "A", description: "Go-to", thinking: ["medium", "high"], default_thinking: "medium" });

  const levels = (model) => (model.reasoning ? ["low", "high"] : ["off"]);
  const pi = piModels(
    [
      { provider: "anthropic", id: "sonnet", name: "Sonnet", reasoning: true },
      { provider: "local", id: "tiny", name: "Tiny", reasoning: false },
    ],
    levels,
    { provider: "local", model: "tiny", thinking: "high" },
  );
  assert.equal(pi.default_model, "local/tiny");
  assert.deepEqual(pi.models[0], { id: "anthropic/sonnet", name: "Sonnet", description: "anthropic", thinking: ["low", "high"], default_thinking: "high" });
  // The settings' level is offered as a default only where the model supports it.
  assert.equal(pi.models[1].default_thinking, null);
  // Settings that name a model the user can't use fall back to the first one.
  assert.equal(piModels([{ provider: "p", id: "m" }], levels, { provider: "x", model: "y" }).default_model, "p/m");
});

test("Claude: a saved session reads back as prompts, replies and finished tool calls", () => {
  const items = claudeHistory([
    { type: "user", uuid: "u1", parent_tool_use_id: null, message: { role: "user", content: "Fix the test" } },
    {
      type: "assistant",
      uuid: "a1",
      parent_tool_use_id: null,
      message: {
        role: "assistant",
        content: [
          { type: "thinking", thinking: "Look first." },
          { type: "tool_use", id: "t1", name: "Bash", input: { command: "ls" } },
        ],
      },
    },
    { type: "assistant", uuid: "s1", parent_tool_use_id: "t1", message: { content: [{ type: "text", text: "sub" }] } },
    {
      type: "user",
      uuid: "u2",
      parent_tool_use_id: null,
      message: { content: [{ type: "tool_result", tool_use_id: "t1", content: "a.rs", is_error: false }] },
    },
    { type: "assistant", uuid: "a2", parent_tool_use_id: null, message: { content: [{ type: "text", text: "Done." }] } },
    {
      type: "user",
      uuid: "u3",
      parent_tool_use_id: null,
      message: { content: [{ type: "text", text: "[Request interrupted by user]" }] },
    },
  ]);
  assert.deepEqual(items, [
    { type: "user", text: "Fix the test" },
    { type: "thinking", id: "a1:0", text: "Look first." },
    { type: "tool", id: "t1", name: "Bash", input: { command: "ls" }, status: "running" },
    { type: "tool", id: "t1", status: "done", output: "a.rs" },
    { type: "text", id: "a2:0", text: "Done." },
    { type: "notice", text: "Stopped." },
  ]);
});

test("Pi: a saved session reads back as prompts, replies, tool calls and errors", () => {
  const items = piHistory([
    { role: "user", content: [{ type: "text", text: "Hi" }] },
    {
      role: "assistant",
      content: [
        { type: "thinking", thinking: "hidden", redacted: true },
        { type: "text", text: "Looking." },
        { type: "toolCall", id: "c1", name: "read", arguments: { path: "a" } },
      ],
    },
    { role: "toolResult", toolCallId: "c1", toolName: "read", content: [{ type: "text", text: "body" }], isError: true },
    { role: "assistant", content: [], stopReason: "error", errorMessage: "No credit" },
  ]);
  assert.deepEqual(items, [
    { type: "user", text: "Hi" },
    { type: "text", id: "m1:1", text: "Looking." },
    { type: "tool", id: "c1", name: "read", input: { path: "a" }, status: "running" },
    { type: "tool", id: "c1", name: "read", status: "failed", output: "body" },
    { type: "notice", text: "No credit" },
  ]);
});
