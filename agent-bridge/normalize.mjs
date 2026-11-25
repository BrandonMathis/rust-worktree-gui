// Turns each harness SDK's events into the bridge's own small event set, so
// the app renders one transcript format whichever harness runs.
//
// Everything here is a pure function or a small state holder with no SDK
// import, so `node --test` covers it without installing the SDKs.
//
// Events the app receives (see `src/agent.rs`):
//   { type: "text", id, delta? | text? }      assistant prose; `delta` appends, `text` replaces
//   { type: "thinking", id, delta? | text? }  reasoning summary, same rule
//   { type: "tool", id, name, input?, status, output? }  status: running | done | failed
//   { type: "notice", text }                  something the user should know (a denied tool, an error)
//   { type: "turn_end", status, error? }      status: done | interrupted | failed

/** Longest string the app is sent for one tool input field or tool output. */
export const CLIP = 2000;

/** Shortens long strings (file contents, command output) anywhere in `value`. */
export function clip(value, limit = CLIP) {
  if (typeof value === "string") {
    return value.length > limit ? `${value.slice(0, limit)}…` : value;
  }
  if (Array.isArray(value)) return value.map((item) => clip(item, limit));
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, clip(item, limit)]));
  }
  return value;
}

/** The text of a tool result, which the SDKs give as a string or as content blocks. */
export function resultText(content) {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((block) => (typeof block === "string" ? block : (block?.text ?? "")))
      .filter(Boolean)
      .join("\n");
  }
  if (content && typeof content === "object") {
    if (Array.isArray(content.content)) return resultText(content.content);
    if (typeof content.text === "string") return content.text;
  }
  return "";
}

// --- Claude Agent SDK ------------------------------------------------------

/**
 * Converts `SDKMessage`s from `query()`. Text and thinking arrive as stream
 * events (`includePartialMessages`), tool calls as complete assistant
 * messages, and tool results as user messages.
 */
export class ClaudeNormalizer {
  #message = "message";
  #streamed = new Set();
  /** Set by the bridge while an interrupt is in flight, to label the turn's end. */
  interrupting = false;

  events(message) {
    // Subagent activity is summarized by its parent `Agent` tool call.
    if (message.parent_tool_use_id) return [];
    switch (message.type) {
      case "stream_event":
        return this.#stream(message.event);
      case "assistant":
        return this.#assistant(message.message);
      case "user":
        return this.#toolResults(message.message);
      case "result":
        return [this.#result(message)];
      default:
        return [];
    }
  }

  #stream(event) {
    switch (event.type) {
      case "message_start":
        this.#message = event.message.id;
        this.#streamed.add(this.#message);
        return [];
      case "content_block_start": {
        const id = `${this.#message}:${event.index}`;
        const block = event.content_block;
        if (block.type === "thinking") return [{ type: "thinking", id, delta: "" }];
        if (block.type === "tool_use") {
          return [{ type: "tool", id: block.id, name: block.name, status: "running" }];
        }
        return [];
      }
      case "content_block_delta": {
        const id = `${this.#message}:${event.index}`;
        if (event.delta.type === "text_delta") {
          return [{ type: "text", id, delta: event.delta.text }];
        }
        if (event.delta.type === "thinking_delta") {
          return [{ type: "thinking", id, delta: event.delta.thinking }];
        }
        return [];
      }
      default:
        return [];
    }
  }

  #assistant(message) {
    const streamed = this.#streamed.has(message.id);
    const events = [];
    for (const [index, block] of (message.content ?? []).entries()) {
      if (block.type === "tool_use") {
        events.push({
          type: "tool",
          id: block.id,
          name: block.name,
          input: clip(block.input ?? {}),
          status: "running",
        });
      } else if (!streamed && block.type === "text" && block.text) {
        // A message that never streamed, such as an API error the CLI reports as text.
        events.push({ type: "text", id: `${message.id}:${index}`, text: block.text });
      }
    }
    return events;
  }

  #toolResults(message) {
    if (!Array.isArray(message?.content)) return [];
    return message.content
      .filter((block) => block.type === "tool_result")
      .map((block) => ({
        type: "tool",
        id: block.tool_use_id,
        status: block.is_error ? "failed" : "done",
        output: clip(resultText(block.content)),
      }));
  }

  #result(message) {
    const interrupted = this.interrupting;
    this.interrupting = false;
    if (interrupted) return { type: "turn_end", status: "interrupted" };
    if (message.subtype === "success" && !message.is_error) {
      return { type: "turn_end", status: "done" };
    }
    const error =
      (Array.isArray(message.errors) && message.errors.join("\n")) ||
      (typeof message.result === "string" && message.result) ||
      String(message.subtype ?? "The turn failed.").replaceAll("_", " ");
    return { type: "turn_end", status: "failed", error };
  }
}

// --- Codex SDK -------------------------------------------------------------

const CODEX_STATUS = { in_progress: "running", completed: "done", failed: "failed" };

/** Converts one `ThreadEvent` from `thread.runStreamed()`. */
export function codexEvents(event) {
  switch (event.type) {
    case "item.started":
    case "item.updated":
    case "item.completed":
      return codexItem(event.item, event.type === "item.completed");
    case "turn.completed":
      return [{ type: "turn_end", status: "done" }];
    case "turn.failed":
      return [{ type: "turn_end", status: "failed", error: event.error?.message ?? "The turn failed." }];
    // A top-level error doesn't end the turn by itself: Codex also reports
    // retries this way ("Reconnecting… 1/5"), and a fatal one is followed by
    // `turn.failed`. The bridge ends the turn when the stream does.
    case "error":
      return [{ type: "notice", text: event.message }];
    default:
      return [];
  }
}

function codexItem(item, completed) {
  const status = CODEX_STATUS[item.status] ?? (completed ? "done" : "running");
  const tool = (name, input, output) => {
    const event = { type: "tool", id: item.id, name, input: clip(input), status };
    if (output) event.output = clip(output);
    return [event];
  };
  switch (item.type) {
    case "agent_message":
      return [{ type: "text", id: item.id, text: item.text }];
    case "reasoning":
      return [{ type: "thinking", id: item.id, text: item.text }];
    case "command_execution":
      return tool("shell", { command: item.command }, item.aggregated_output);
    case "file_change":
      return tool("apply_patch", { changes: item.changes });
    case "mcp_tool_call":
      return tool(`mcp__${item.server}__${item.tool}`, item.arguments ?? {}, item.error?.message);
    case "web_search":
      return tool("web_search", { query: item.query });
    case "todo_list":
      return tool("todo_list", { items: item.items });
    case "error":
      return [{ type: "notice", text: item.message }];
    default:
      return [];
  }
}

// --- Pi SDK ----------------------------------------------------------------

/** Converts `AgentSessionEvent`s from `session.subscribe()`. */
export class PiNormalizer {
  #message = 0;
  /**
   * The error that ended the latest assistant message, if any. Pi retries
   * some errors itself, so the bridge reads this only once the run is over.
   */
  lastError = null;

  events(event) {
    switch (event.type) {
      case "message_start":
        this.#message += 1;
        this.lastError = null;
        return [];
      case "message_update": {
        const update = event.assistantMessageEvent;
        const id = `m${this.#message}:${update.contentIndex}`;
        if (update.type === "text_delta") return [{ type: "text", id, delta: update.delta }];
        if (update.type === "thinking_start") return [{ type: "thinking", id, delta: "" }];
        if (update.type === "thinking_delta") return [{ type: "thinking", id, delta: update.delta }];
        return [];
      }
      case "message_end": {
        const message = event.message;
        if (message?.role === "assistant" && message.stopReason === "error") {
          this.lastError = clip(message.errorMessage || "The model request failed.", 600);
        }
        return [];
      }
      case "tool_execution_start":
        return [
          {
            type: "tool",
            id: event.toolCallId,
            name: event.toolName,
            input: clip(event.args ?? {}),
            status: "running",
          },
        ];
      case "tool_execution_end":
        return [
          {
            type: "tool",
            id: event.toolCallId,
            name: event.toolName,
            status: event.isError ? "failed" : "done",
            output: clip(resultText(event.result)),
          },
        ];
      default:
        return [];
    }
  }
}

// --- Model lists -----------------------------------------------------------

/** `ModelInfo[]` from the Claude Agent SDK's `supportedModels()`. */
export function claudeModels(models) {
  return {
    default_model: models[0]?.value ?? null,
    models: models.map((model) => ({
      id: model.value,
      name: model.displayName,
      description: model.description ?? "",
      thinking: model.supportedEffortLevels ?? [],
      default_thinking: null,
    })),
  };
}

/** The catalog printed by `codex debug models`. */
export function codexModels(catalog) {
  const models = (catalog.models ?? [])
    .filter((model) => model.visibility === "list")
    .sort((a, b) => (a.priority ?? 0) - (b.priority ?? 0))
    .map((model) => ({
      id: model.slug,
      name: model.display_name ?? model.slug,
      description: model.description ?? "",
      thinking: (model.supported_reasoning_levels ?? []).map((level) => level.effort),
      default_thinking: model.default_reasoning_level ?? null,
    }));
  return { default_model: models[0]?.id ?? null, models };
}

/**
 * Pi's available models. `levels(model)` is pi-ai's `getSupportedThinkingLevels`;
 * `defaults` come from the user's Pi settings.
 */
export function piModels(available, levels, defaults = {}) {
  const models = available.map((model) => {
    const thinking = levels(model);
    return {
      id: `${model.provider}/${model.id}`,
      name: model.name ?? model.id,
      description: model.provider,
      thinking,
      default_thinking: thinking.includes(defaults.thinking) ? defaults.thinking : null,
    };
  });
  const preferred = `${defaults.provider}/${defaults.model}`;
  return {
    default_model: models.some((model) => model.id === preferred) ? preferred : (models[0]?.id ?? null),
    models,
  };
}
