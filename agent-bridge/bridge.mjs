// Worktree Browser's agent bridge: one process per managed worktree.
//
// The app (Rust) starts `node bridge.mjs`, writes one JSON command per line to
// stdin, and reads one JSON event per line from stdout. This file drives the
// three official SDKs; `normalize.mjs` turns their events into the app's.
//
// Commands:
//   { type: "models", harness }                          -> { type: "models", models, default_model }
//   { type: "start", harness, model, thinking, thinking_summaries, resume? }
//                                                        starts a conversation in the working directory,
//                                                        or continues the harness's saved one `resume`
//                                                        -> { type: "session", id } once the harness names it
//   { type: "history", harness, resume }                -> { type: "history", items }: the saved conversation's
//                                                        prompts, replies and tool calls, to show it again
//   { type: "prompt", text }                             -> text / thinking / tool events, then turn_end
//   { type: "interrupt" }                                -> the running turn ends as "interrupted"
//
// The app sends one prompt at a time: it queues follow-ups itself and sends
// the next one after `turn_end`. Closing stdin ends the session.

import { execFile, spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { createInterface } from "node:readline";
import { promisify } from "node:util";
import {
  ClaudeNormalizer,
  claudeHistory,
  PiNormalizer,
  claudeModels,
  codexEvents,
  codexModels,
  piHistory,
  piModels,
} from "./normalize.mjs";

// stdout carries only protocol lines, so anything a dependency logs goes to stderr.
console.log = console.error;
console.info = console.error;
console.debug = console.error;

// The app starts the bridge inside the worktree, so the path never has to
// survive a trip through JSON. Arguments are extra folders the agent may write
// to: the repository's Git directory, which lives outside a linked worktree.
const WORKTREE = process.cwd();
const WRITABLE_DIRS = process.argv.slice(2);

function emit(event) {
  process.stdout.write(`${JSON.stringify(event)}\n`);
}

function describe(error) {
  return error instanceof Error ? error.message : String(error);
}

/** A queue the Claude SDK reads user messages from (its "streaming input" mode). */
class MessageQueue {
  #items = [];
  #waiting = null;
  #closed = false;

  push(item) {
    if (this.#waiting) {
      this.#waiting({ value: item, done: false });
      this.#waiting = null;
    } else {
      this.#items.push(item);
    }
  }

  close() {
    this.#closed = true;
    this.#waiting?.({ value: undefined, done: true });
  }

  [Symbol.asyncIterator]() {
    return {
      next: () => {
        if (this.#items.length > 0) return Promise.resolve({ value: this.#items.shift(), done: false });
        if (this.#closed) return Promise.resolve({ value: undefined, done: true });
        return new Promise((resolve) => (this.#waiting = resolve));
      },
    };
  }
}

// --- Claude: @anthropic-ai/claude-agent-sdk --------------------------------

const claude = {
  async models() {
    const { query } = await import("@anthropic-ai/claude-agent-sdk");
    const input = new MessageQueue();
    const session = query({ prompt: input, options: {} });
    try {
      return claudeModels(await session.supportedModels());
    } finally {
      input.close();
      session.close();
    }
  },

  async history(resume) {
    const { getSessionMessages } = await import("@anthropic-ai/claude-agent-sdk");
    return claudeHistory(await getSessionMessages(resume, { dir: WORKTREE }));
  },

  async start({ model, thinking, thinking_summaries, resume }) {
    const { query } = await import("@anthropic-ai/claude-agent-sdk");
    const input = new MessageQueue();
    const normalizer = new ClaudeNormalizer();
    const options = {
      cwd: WORKTREE,
      // Claude Code's own system prompt, tools and the user's settings, as in the CLI.
      systemPrompt: { type: "preset", preset: "claude_code" },
      includePartialMessages: true,
      // The permission mode is left to the user's Claude settings. This UI has
      // no approval prompt, so a call that would ask is declined, and said so.
      canUseTool: async (toolName) => {
        emit({
          type: "notice",
          text: `Claude asked for permission to use ${toolName}. Managed worktrees can't show permission prompts, so it was declined. Allow the tool in Claude's settings, or use a permission mode that doesn't ask.`,
        });
        return {
          behavior: "deny",
          message: "This session can't ask the user for permission. Continue without this tool call.",
        };
      },
    };
    if (model) options.model = model;
    if (thinking) options.effort = thinking;
    // Claude reloads the conversation from its own session file.
    if (resume) options.resume = resume;
    // Current models think adaptively but return no thinking text unless asked
    // for a summary. Older ones reject the setting, so the app says which is which.
    if (thinking_summaries) options.thinking = { type: "adaptive", display: "summarized" };
    const session = query({ prompt: input, options });
    (async () => {
      try {
        for await (const message of session) {
          for (const event of normalizer.events(message)) emit(event);
        }
      } catch (error) {
        emit({ type: "turn_end", status: "failed", error: describe(error) });
      }
    })();
    return {
      prompt(text) {
        input.push({
          type: "user",
          message: { role: "user", content: text },
          parent_tool_use_id: null,
        });
      },
      async interrupt() {
        normalizer.interrupting = true;
        await session.interrupt();
      },
      dispose() {
        input.close();
        session.close();
      },
    };
  },
};

// --- Codex: @openai/codex-sdk ----------------------------------------------

/** The pids of every process this one started, directly or through a child. */
function descendants() {
  const { stdout } = spawnSync("/bin/ps", ["-axo", "pid=,ppid="], { encoding: "utf8" });
  const children = new Map();
  for (const line of (stdout ?? "").split("\n")) {
    const [pid, parent] = line.trim().split(/\s+/).map(Number);
    if (!pid) continue;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(pid);
  }
  const found = [];
  const pending = [process.pid];
  while (pending.length > 0) {
    for (const child of children.get(pending.pop()) ?? []) {
      found.push(child);
      pending.push(child);
    }
  }
  return found;
}

/**
 * Stops a Codex turn. The SDK's only way is to kill the Codex CLI, which
 * leaves the shell command it was running (in its own process group)
 * behind, so end everything the turn started as well.
 */
function stopCodexTurn(abort) {
  if (!abort || abort.signal.aborted) return;
  const running = descendants();
  abort.abort();
  for (const pid of running) {
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // Already gone.
    }
  }
}

const codex = {
  // The SDK has no model list, so ask the Codex binary it bundles for its catalog.
  async models() {
    const launcher = createRequire(import.meta.url).resolve("@openai/codex/bin/codex.js");
    const { stdout } = await promisify(execFile)(process.execPath, [launcher, "debug", "models"], {
      maxBuffer: 64 * 1024 * 1024,
    });
    return codexModels(JSON.parse(stdout));
  },

  // The Codex SDK resumes a thread but can't read its earlier items back.
  async history() {
    return [
      {
        type: "notice",
        text: "Codex can't show the earlier messages of a resumed thread, but the agent remembers them.",
      },
    ];
  },

  async start({ model, thinking, resume }) {
    const { Codex } = await import("@openai/codex-sdk");
    const options = {
      workingDirectory: WORKTREE,
      // Edits stay inside the worktree (plus its Git directory, so commits
      // work), enforced by Codex's sandbox. Nothing asks for approval.
      sandboxMode: "workspace-write",
      approvalPolicy: "never",
      additionalDirectories: WRITABLE_DIRS,
    };
    if (model) options.model = model;
    if (thinking) options.modelReasoningEffort = thinking;
    // A new thread reports its id with its first turn (`thread.started`).
    const thread = resume ? new Codex().resumeThread(resume, options) : new Codex().startThread(options);
    let abort = null;
    return {
      async prompt(text) {
        abort = new AbortController();
        // A prompt ends in exactly one `turn_end`: the app sends the next
        // queued follow-up on each one.
        let ended = false;
        let lastError = null;
        try {
          const { events } = await thread.runStreamed(text, { signal: abort.signal });
          for await (const event of events) {
            for (const normalized of codexEvents(event)) {
              if (normalized.type === "turn_end") {
                if (ended) continue;
                ended = true;
              }
              if (event.type === "error") lastError = event.message;
              emit(normalized);
            }
          }
          if (!ended) {
            emit(lastError ? { type: "turn_end", status: "failed", error: lastError } : { type: "turn_end", status: "done" });
          }
        } catch (error) {
          if (ended) return;
          emit(
            abort.signal.aborted
              ? { type: "turn_end", status: "interrupted" }
              : { type: "turn_end", status: "failed", error: describe(error) },
          );
        }
      },
      async interrupt() {
        stopCodexTurn(abort);
      },
      dispose() {
        stopCodexTurn(abort);
      },
    };
  },
};

// --- Pi: @earendil-works/pi-coding-agent -----------------------------------

const pi = {
  async models() {
    const { ModelRuntime, SettingsManager } = await import("@earendil-works/pi-coding-agent");
    const { getSupportedThinkingLevels } = await import("@earendil-works/pi-ai");
    const runtime = await ModelRuntime.create();
    const settings = SettingsManager.create(process.cwd());
    return piModels(await runtime.getAvailable(), getSupportedThinkingLevels, {
      provider: settings.getDefaultProvider(),
      model: settings.getDefaultModel(),
      thinking: settings.getDefaultThinkingLevel(),
    });
  },

  async history(resume) {
    return piHistory((await piSession(resume)).buildSessionContext().messages);
  },

  async start({ model, thinking, resume }) {
    const { ModelRuntime, createAgentSession } = await import("@earendil-works/pi-coding-agent");
    const modelRuntime = await ModelRuntime.create();
    const options = { cwd: WORKTREE, modelRuntime };
    if (resume) options.sessionManager = await piSession(resume);
    if (model) {
      const [provider, ...id] = model.split("/");
      const found = modelRuntime.getModel(provider, id.join("/"));
      if (!found) throw new Error(`Pi has no model named ${model}.`);
      options.model = found;
    }
    if (thinking) options.thinkingLevel = thinking;
    const { session } = await createAgentSession(options);
    emit({ type: "session", id: session.sessionId });
    const normalizer = new PiNormalizer();
    session.subscribe((event) => {
      for (const normalized of normalizer.events(event)) emit(normalized);
    });
    let interrupting = false;
    return {
      async prompt(text) {
        interrupting = false;
        try {
          await session.prompt(text);
          if (interrupting) emit({ type: "turn_end", status: "interrupted" });
          else if (normalizer.lastError) emit({ type: "turn_end", status: "failed", error: normalizer.lastError });
          else emit({ type: "turn_end", status: "done" });
        } catch (error) {
          emit(
            interrupting
              ? { type: "turn_end", status: "interrupted" }
              : { type: "turn_end", status: "failed", error: describe(error) },
          );
        }
      },
      async interrupt() {
        interrupting = true;
        await session.abort();
      },
      dispose() {
        session.dispose();
      },
    };
  },
};

/** Opens Pi's saved conversation `id`. Pi keeps one file per conversation; find this worktree's by its id. */
async function piSession(id) {
  const { SessionManager } = await import("@earendil-works/pi-coding-agent");
  const saved = (await SessionManager.list(WORKTREE)).find((info) => info.id === id);
  if (!saved) throw new Error("Pi has no saved conversation to resume. Start a new chat.");
  return SessionManager.open(saved.path);
}

const HARNESSES = { claude, codex, pi };

// --- Command loop ----------------------------------------------------------

let session = null;

async function handle(command) {
  switch (command.type) {
    case "models": {
      const harness = HARNESSES[command.harness];
      if (!harness) throw new Error(`Unknown harness: ${command.harness}`);
      emit({ type: "models", ...(await harness.models()) });
      return;
    }
    case "history": {
      const harness = HARNESSES[command.harness];
      if (!harness) throw new Error(`Unknown harness: ${command.harness}`);
      emit({ type: "history", items: await harness.history(command.resume) });
      return;
    }
    case "start": {
      const harness = HARNESSES[command.harness];
      if (!harness) throw new Error(`Unknown harness: ${command.harness}`);
      session?.dispose();
      session = await harness.start(command);
      return;
    }
    case "prompt":
      if (!session) throw new Error("No session has been started.");
      // Not awaited: the turn's events stream while later commands (interrupt) are read.
      Promise.resolve(session.prompt(command.text)).catch((error) => {
        emit({ type: "turn_end", status: "failed", error: describe(error) });
      });
      return;
    case "interrupt":
      await session?.interrupt();
      return;
    default:
      throw new Error(`Unknown command: ${command.type}`);
  }
}

// Commands run in order: `start` must finish before the `prompt` behind it.
let queue = Promise.resolve();
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on("line", (line) => {
  if (!line.trim()) return;
  queue = queue.then(async () => {
    try {
      await handle(JSON.parse(line));
    } catch (error) {
      emit({ type: "error", message: describe(error) });
    }
  });
});
lines.on("close", () => {
  // The app closed stdin (the session was dropped, or the app quit).
  queue.finally(() => {
    try {
      session?.dispose();
    } finally {
      process.exit(0);
    }
  });
});
