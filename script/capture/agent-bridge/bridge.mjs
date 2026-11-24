// A stand-in for agent-bridge/bridge.mjs, used only by the screenshot script.
//
// It speaks the same JSON-lines protocol but calls no SDK and no network: it
// lists a few made-up models and answers every prompt with the same short,
// scripted turn. That keeps the screenshots of managed worktrees identical on
// every run, on machines that have no harness installed or signed in.

import { createInterface } from "node:readline";

const MODELS = {
  claude: {
    default_model: "sonnet",
    models: [
      { id: "opus", name: "Opus", description: "For complex work", thinking: ["low", "medium", "high", "xhigh", "max"], default_thinking: null },
      { id: "sonnet", name: "Sonnet", description: "For everyday tasks", thinking: ["low", "medium", "high", "xhigh", "max"], default_thinking: null },
      { id: "haiku", name: "Haiku", description: "Fastest", thinking: [], default_thinking: null },
    ],
  },
  codex: {
    default_model: "gpt-sol",
    models: [
      { id: "gpt-sol", name: "GPT Sol", description: "Go-to model", thinking: ["low", "medium", "high"], default_thinking: "medium" },
      { id: "gpt-luna", name: "GPT Luna", description: "Fast", thinking: ["low", "medium"], default_thinking: "low" },
    ],
  },
  pi: {
    default_model: "anthropic/sonnet",
    models: [
      { id: "anthropic/sonnet", name: "Claude Sonnet", description: "anthropic", thinking: ["off", "low", "medium", "high"], default_thinking: "medium" },
    ],
  },
};

const cwd = process.cwd();

// One scripted turn: thinking, a few tool calls, then a Markdown reply.
const TURN = [
  { type: "thinking", id: "k1", delta: "**Finding where requests are logged**\n\nThe routes module handles every request, so the audit entry belongs there." },
  { type: "tool", id: "t1", name: "Grep", input: { pattern: "fn handle", path: `${cwd}/src` }, status: "done", output: "src/routes.rs:14:pub fn handle(request: &Request) -> Response {" },
  { type: "tool", id: "t2", name: "Read", input: { file_path: `${cwd}/src/routes.rs` }, status: "done", output: "pub fn handle(request: &Request) -> Response {\n    // TODO: auth\n    route(request)\n}" },
  { type: "thinking", id: "k2", delta: "" },
  { type: "tool", id: "t3", name: "Edit", input: { file_path: `${cwd}/src/audit.rs` }, status: "done", output: "The file was updated." },
  { type: "tool", id: "t4", name: "Bash", input: { command: "cargo test audit", description: "Run the audit log tests" }, status: "done", output: "running 3 tests\ntest result: ok. 3 passed; 0 failed" },
  { type: "tool", id: "t5", name: "Bash", input: { command: "cargo clippy -- -D warnings" }, status: "failed", output: "error: unused variable: `actor`" },
  {
    type: "text",
    id: "m1",
    text: "I added an audit log for every request.\n\n- `src/audit.rs` records the **actor**, route and status of each call.\n- `routes::handle` writes an entry after the response is built.\n\nThe three new tests pass. `cargo clippy` still reports one unused variable in `audit.rs`, which I can remove next.",
  },
  { type: "turn_end", status: "done" },
];

function emit(event) {
  process.stdout.write(`${JSON.stringify(event)}\n`);
}

createInterface({ input: process.stdin }).on("line", (line) => {
  const command = JSON.parse(line);
  if (command.type === "models") emit({ type: "models", ...MODELS[command.harness] });
  if (command.type === "prompt") TURN.forEach(emit);
});
