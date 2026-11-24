//! Managed worktrees: a small coding-agent harness that drives the Claude
//! Code, Codex or Pi command-line tools instead of a terminal.
//!
//! Each turn runs the harness once in its headless JSON mode, from the
//! worktree's directory, and the harness's own session id is passed back on
//! the next turn so the conversation continues. This module is the part with
//! no UI: which harnesses and models exist, the command line for a turn,
//! parsing each harness's JSON lines into one [`Event`] type, and the
//! [`Transcript`] those events build. `app::agent_pane` draws it.

use crate::runner::{self, Tool};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A coding-agent command-line tool that can run a managed worktree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
    Pi,
}

impl Harness {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::Pi];

    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Pi => "Pi",
        }
    }

    pub fn tool(self) -> Tool {
        match self {
            Self::Claude => Tool::Claude,
            Self::Codex => Tool::Codex,
            Self::Pi => Tool::Pi,
        }
    }

    /// Thinking levels the harness accepts when a model doesn't list its own.
    fn levels(self) -> &'static [&'static str] {
        match self {
            // `claude --effort`
            Self::Claude => &["low", "medium", "high", "xhigh", "max"],
            // Codex's `model_reasoning_effort`
            Self::Codex => &["minimal", "low", "medium", "high", "xhigh"],
            // `pi --thinking`
            Self::Pi => &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        }
    }
}

impl fmt::Display for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A model to pick for a turn. `id: None` is the harness's own default (its
/// config file decides), so nothing is passed on the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub id: Option<String>,
    /// Thinking levels this model accepts.
    pub levels: Vec<String>,
}

impl Model {
    pub fn new(harness: Harness, id: Option<&str>) -> Self {
        Self {
            id: id.map(str::to_owned),
            levels: harness.levels().iter().map(|&l| l.to_owned()).collect(),
        }
    }
}

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id.as_deref().unwrap_or("Default model"))
    }
}

/// A thinking level for the pick list. `None` is the harness default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level(pub Option<String>);

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(level) => write!(f, "Thinking: {level}"),
            None => f.write_str("Default thinking"),
        }
    }
}

/// What a managed worktree runs with, saved with its bookmark.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// The harness's session id, once its first turn has reported one.
    /// Follow-up turns resume it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            harness: Harness::Claude,
            model: None,
            thinking: None,
            session: None,
        }
    }
}

/// Lists the models `harness` offers, default first. Blocking: run it from
/// `background(...)`.
pub fn list_models(harness: Harness) -> Result<Vec<Model>, String> {
    let mut models = vec![Model::new(harness, None)];
    match harness {
        // Claude Code has no command that lists models; these aliases always
        // select the latest model of each family.
        Harness::Claude => {
            models.extend(
                ["fable", "opus", "sonnet", "haiku"].map(|alias| Model::new(harness, Some(alias))),
            );
        }
        Harness::Codex => {
            models.extend(parse_codex_models(&query(harness, &["debug", "models"])?)?)
        }
        Harness::Pi => models.extend(parse_pi_models(&query(harness, &["--list-models"])?)),
    }
    Ok(models)
}

fn query(harness: Harness, args: &[&str]) -> Result<String, String> {
    let program = runner::find(harness.tool())?;
    let output = runner::run(
        runner::command(&program).args(args),
        Duration::from_secs(20),
    )
    .map_err(|error| format!("Could not list {} models: {error}", harness.name()))?;
    if !output.status.success() {
        return Err(format!(
            "Could not list {} models: {}",
            harness.name(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `codex debug models` prints its catalog as JSON. Hidden models are skipped.
fn parse_codex_models(json: &str) -> Result<Vec<Model>, String> {
    let catalog: Value =
        serde_json::from_str(json).map_err(|error| format!("Unreadable Codex models: {error}"))?;
    let models = catalog["models"].as_array().cloned().unwrap_or_default();
    Ok(models
        .iter()
        .filter(|model| model["visibility"].as_str() != Some("hide"))
        .filter_map(|model| {
            let mut found = Model::new(Harness::Codex, Some(model["slug"].as_str()?));
            let levels: Vec<String> = model["supported_reasoning_levels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|level| level["effort"].as_str().map(str::to_owned))
                .collect();
            if !levels.is_empty() {
                found.levels = levels;
            }
            Some(found)
        })
        .collect())
}

/// `pi --list-models` prints a table: `provider  model  context  max-out
/// thinking  images`. Models are picked as `provider/model`.
fn parse_pi_models(table: &str) -> Vec<Model> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            let [provider, model, _, _, thinking, ..] = columns[..] else {
                return None;
            };
            let mut found = Model::new(Harness::Pi, Some(&format!("{provider}/{model}")));
            if thinking != "yes" {
                found.levels.clear();
            }
            Some(found)
        })
        .collect()
}

/// How to run one turn: arguments after the program, and where the prompt goes.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub args: Vec<OsString>,
    /// Written to the harness's stdin, which is then closed. `None` when the
    /// prompt is already in `args`.
    pub stdin: Option<String>,
}

/// The command line for one turn of `settings` with `prompt`.
///
/// Permissions: managed turns run unattended, so nobody can answer a
/// permission prompt. Claude uses its `auto` mode (a classifier approves safe
/// actions and refuses risky ones), Codex its `workspace-write` sandbox (edits
/// and commands inside the worktree only), and Pi has no permission system.
pub fn invocation(settings: &Settings, prompt: &str) -> Invocation {
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |values: &[&str]| args.extend(values.iter().map(OsString::from));
    match settings.harness {
        Harness::Claude => {
            push(&[
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "auto",
            ]);
            if let Some(model) = &settings.model {
                push(&["--model", model]);
            }
            if let Some(level) = &settings.thinking {
                push(&["--effort", level]);
            }
            if let Some(session) = &settings.session {
                push(&["--resume", session]);
            }
        }
        Harness::Codex => {
            push(&["exec"]);
            if settings.session.is_some() {
                push(&["resume"]);
            }
            push(&["--json", "-c", r#"sandbox_mode="workspace-write""#]);
            if let Some(model) = &settings.model {
                push(&["-m", model]);
            }
            if let Some(level) = &settings.thinking {
                push(&["-c", &format!("model_reasoning_effort={level:?}")]);
            }
            if let Some(session) = &settings.session {
                push(&[session]);
            }
            // `-` reads the prompt from stdin.
            push(&["-"]);
        }
        Harness::Pi => {
            push(&["-p", "--mode", "json"]);
            if let Some(model) = &settings.model {
                push(&["--model", model]);
            }
            if let Some(level) = &settings.thinking {
                push(&["--thinking", level]);
            }
            if let Some(session) = &settings.session {
                push(&["--session", session]);
            }
            // After `--`, a prompt starting with `-` is not read as an option.
            push(&["--", prompt]);
            return Invocation { args, stdin: None };
        }
    }
    Invocation {
        args,
        stdin: Some(prompt.to_owned()),
    }
}

/// What a tool call did, for the collapsed summary ("Read 2 files, ran a
/// command").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Thinking,
    Read,
    Edit,
    Search,
    Command,
    Web,
    Agent,
    Plan,
    Other,
}

impl Kind {
    /// The phrase for `count` steps of this kind, e.g. "ran 2 commands".
    fn phrase(self, count: usize) -> String {
        let plural = |one: &str, many: &str| {
            if count == 1 {
                one.replace('#', "1")
            } else {
                many.replace('#', &count.to_string())
            }
        };
        match self {
            Self::Thinking => "thought".into(),
            Self::Read => plural("read a file", "read # files"),
            Self::Edit => plural("edited a file", "made # edits"),
            Self::Search => plural("searched once", "searched # times"),
            Self::Command => plural("ran a command", "ran # commands"),
            Self::Web => plural("used the web", "used the web # times"),
            Self::Agent => plural("ran an agent", "ran # agents"),
            Self::Plan => "updated the plan".into(),
            Self::Other => plural("used a tool", "used # tools"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Running,
    Done,
    Failed,
}

/// One harness event, the same for every harness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The harness's session id, to resume on the next turn.
    Session(String),
    Text(String),
    Thinking(String),
    /// A tool call started or finished. A finish for an id never seen
    /// starting adds the step then.
    Tool {
        id: String,
        kind: Kind,
        summary: String,
        state: StepState,
    },
    /// The harness reported that the turn failed.
    Failed(String),
}

/// Parses one line of a harness's JSON output. Lines that aren't JSON (a
/// warning on stdout) or events the transcript doesn't show give nothing.
pub fn parse_line(harness: Harness, cwd: &Path, line: &str) -> Vec<Event> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match harness {
        Harness::Claude => parse_claude(&value, cwd),
        Harness::Codex => parse_codex(&value, cwd),
        Harness::Pi => parse_pi(&value, cwd),
    }
}

fn text_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// Claude Code's `--output-format stream-json`: whole assistant messages
/// (text, thinking and `tool_use` blocks), `tool_result` blocks in user
/// messages, and a final `result`.
fn parse_claude(value: &Value, cwd: &Path) -> Vec<Event> {
    // Messages from a subagent carry its parent's tool call id. The parent's
    // "Ran an agent" step stands for all of them.
    if value["parent_tool_use_id"].is_string() {
        return Vec::new();
    }
    let blocks = || value["message"]["content"].as_array().into_iter().flatten();
    match value["type"].as_str() {
        Some("system") if value["subtype"] == "init" => value["session_id"]
            .as_str()
            .map(|id| Event::Session(id.into()))
            .into_iter()
            .collect(),
        Some("assistant") => blocks()
            .filter_map(|block| match block["type"].as_str()? {
                "text" => Some(Event::Text(text_of(&block["text"]))),
                "thinking" => Some(Event::Thinking(text_of(&block["thinking"]))),
                "redacted_thinking" => Some(Event::Thinking(String::new())),
                "tool_use" => {
                    let (kind, summary) =
                        describe_tool(block["name"].as_str()?, &block["input"], cwd);
                    Some(Event::Tool {
                        id: block["id"].as_str()?.into(),
                        kind,
                        summary,
                        state: StepState::Running,
                    })
                }
                _ => None,
            })
            .collect(),
        Some("user") => blocks()
            .filter(|block| block["type"] == "tool_result")
            .filter_map(|block| {
                Some(Event::Tool {
                    id: block["tool_use_id"].as_str()?.into(),
                    kind: Kind::Other,
                    summary: String::new(),
                    state: if block["is_error"] == true {
                        StepState::Failed
                    } else {
                        StepState::Done
                    },
                })
            })
            .collect(),
        Some("result") if value["is_error"] == true => {
            let reason = value["result"]
                .as_str()
                .or_else(|| value["subtype"].as_str())
                .unwrap_or("Claude reported an error.");
            vec![Event::Failed(reason.into())]
        }
        _ => Vec::new(),
    }
}

/// `codex exec --json`: `thread.started`, then `item.started` and
/// `item.completed` for messages, reasoning and tool calls.
fn parse_codex(value: &Value, cwd: &Path) -> Vec<Event> {
    codex_event(value, cwd).into_iter().collect()
}

fn codex_event(value: &Value, cwd: &Path) -> Option<Event> {
    let item = &value["item"];
    match value["type"].as_str() {
        Some("thread.started") => value["thread_id"]
            .as_str()
            .map(|id| Event::Session(id.into())),
        Some("turn.failed") => Some(Event::Failed(
            value["error"]["message"]
                .as_str()
                .unwrap_or("Codex reported an error.")
                .into(),
        )),
        Some("error") => Some(Event::Failed(text_of(&value["message"]))),
        Some(phase @ ("item.started" | "item.completed")) => {
            let done = phase == "item.completed";
            match item["type"].as_str() {
                Some("agent_message") if done => Some(Event::Text(text_of(&item["text"]))),
                Some("reasoning") if done => Some(Event::Thinking(text_of(&item["text"]))),
                Some("error") if done => Some(Event::Failed(text_of(&item["message"]))),
                Some(kind) => {
                    let (kind, summary) = describe_codex_item(kind, item, cwd)?;
                    let failed = item["status"] == "failed"
                        || item["exit_code"].as_i64().is_some_and(|code| code != 0);
                    Some(Event::Tool {
                        id: item["id"].as_str()?.into(),
                        kind,
                        summary,
                        state: match (done, failed) {
                            (false, _) => StepState::Running,
                            (true, false) => StepState::Done,
                            (true, true) => StepState::Failed,
                        },
                    })
                }
                None => None,
            }
        }
        _ => None,
    }
}

fn describe_codex_item(kind: &str, item: &Value, cwd: &Path) -> Option<(Kind, String)> {
    Some(match kind {
        "command_execution" => {
            let command = unwrap_shell(item["command"].as_str().unwrap_or_default());
            (Kind::Command, format!("Ran {}", code(command)))
        }
        "file_change" => {
            let paths: Vec<String> = item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|change| change["path"].as_str())
                .map(|path| relative(path, cwd))
                .collect();
            (Kind::Edit, format!("Edited {}", paths.join(", ")))
        }
        "mcp_tool_call" => (
            Kind::Other,
            format!(
                "Used {}.{}",
                item["server"].as_str().unwrap_or("mcp"),
                item["tool"].as_str().unwrap_or("tool")
            ),
        ),
        "web_search" => (
            Kind::Web,
            format!("Searched the web for {}", quote(&item["query"])),
        ),
        "todo_list" => (Kind::Plan, "Updated the plan".into()),
        _ => return None,
    })
}

/// Codex runs commands as `/bin/zsh -lc '<command>'` (unquoted when the
/// command is a single word); show just the command.
fn unwrap_shell(command: &str) -> &str {
    match command.split_once(" -lc ") {
        Some((shell, rest)) if shell.starts_with('/') && !shell.contains(' ') => rest
            .strip_prefix('\'')
            .and_then(|rest| rest.strip_suffix('\''))
            .unwrap_or(rest),
        _ => command,
    }
}

/// Pi's `--mode json`: `session`, whole messages at `message_end`, and
/// `tool_execution_start`/`_end` for tool calls.
fn parse_pi(value: &Value, cwd: &Path) -> Vec<Event> {
    match value["type"].as_str() {
        Some("session") => value["id"]
            .as_str()
            .map(|id| Event::Session(id.into()))
            .into_iter()
            .collect(),
        Some("message_end") if value["message"]["role"] == "assistant" => {
            let message = &value["message"];
            let mut events: Vec<Event> = message["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|block| match block["type"].as_str()? {
                    "text" => Some(Event::Text(text_of(&block["text"]))),
                    "thinking" => Some(Event::Thinking(text_of(&block["thinking"]))),
                    // Tool calls are reported by `tool_execution_*` instead.
                    _ => None,
                })
                .collect();
            if message["stopReason"] == "error" {
                events.push(Event::Failed(
                    message["errorMessage"]
                        .as_str()
                        .unwrap_or("Pi reported an error.")
                        .into(),
                ));
            }
            events
        }
        Some(phase @ ("tool_execution_start" | "tool_execution_end")) => {
            let Some(name) = value["toolName"].as_str() else {
                return Vec::new();
            };
            // `codemode` is Pi's script runner. The tools a script calls are
            // reported on their own (with a `parentToolCallId`), so the
            // wrapper itself would only be counted twice.
            if name == "codemode" {
                return Vec::new();
            }
            let Some(id) = value["toolCallId"].as_str() else {
                return Vec::new();
            };
            let (kind, summary) = describe_tool(name, &value["args"], cwd);
            vec![Event::Tool {
                id: id.into(),
                kind,
                summary,
                state: match (phase, value["isError"] == true) {
                    ("tool_execution_start", _) => StepState::Running,
                    (_, false) => StepState::Done,
                    (_, true) => StepState::Failed,
                },
            }]
        }
        _ => Vec::new(),
    }
}

/// A one-line description of a Claude or Pi tool call from its name and input.
fn describe_tool(name: &str, input: &Value, cwd: &Path) -> (Kind, String) {
    let path = || {
        relative(
            input["file_path"]
                .as_str()
                .or_else(|| input["path"].as_str())
                .unwrap_or("a file"),
            cwd,
        )
    };
    match name {
        "Bash" | "bash" => {
            let command = input["command"].as_str().unwrap_or_default();
            match input["description"]
                .as_str()
                .filter(|d| !d.trim().is_empty())
            {
                Some(description) => (Kind::Command, description.trim().to_owned()),
                None => (Kind::Command, format!("Ran {}", code(command))),
            }
        }
        "Read" | "read" | "NotebookRead" => (Kind::Read, format!("Read {}", path())),
        "Edit" | "MultiEdit" | "edit" | "NotebookEdit" => {
            (Kind::Edit, format!("Edited {}", path()))
        }
        "Write" | "write" => (Kind::Edit, format!("Wrote {}", path())),
        "Grep" | "grep" => (
            Kind::Search,
            format!("Searched for {}", quote(&input["pattern"])),
        ),
        "Glob" | "find" => (
            Kind::Search,
            format!("Found files matching {}", quote(&input["pattern"])),
        ),
        "LS" | "ls" => (Kind::Search, format!("Listed {}", path())),
        "WebFetch" => (Kind::Web, format!("Fetched {}", text_of(&input["url"]))),
        "WebSearch" => (
            Kind::Web,
            format!("Searched the web for {}", quote(&input["query"])),
        ),
        "Task" | "Agent" => (
            Kind::Agent,
            format!(
                "Agent: {}",
                input["description"].as_str().unwrap_or("subtask")
            ),
        ),
        "TodoWrite" => (Kind::Plan, "Updated the plan".into()),
        other => (Kind::Other, format!("Used {other}")),
    }
}

/// `path` relative to the worktree when it is inside it.
fn relative(path: &str, cwd: &Path) -> String {
    let path = Path::new(path);
    // macOS reports `/tmp/...` as `/private/tmp/...`; try both spellings.
    let private = Path::new("/private").join(cwd.strip_prefix("/").unwrap_or(cwd));
    [cwd, private.as_path()]
        .iter()
        .find_map(|root| path.strip_prefix(root).ok())
        .filter(|rest| !rest.as_os_str().is_empty())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The first line of a command, shortened, in backticks.
fn code(command: &str) -> String {
    let line = command.lines().next().unwrap_or_default().trim();
    format!("`{}`", shorten(line, 80))
}

fn quote(value: &Value) -> String {
    format!("“{}”", shorten(value.as_str().unwrap_or_default(), 60))
}

fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let cut: String = text.chars().take(limit - 1).collect();
    format!("{cut}…")
}

/// One thinking block or tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub id: Option<String>,
    pub kind: Kind,
    pub summary: String,
    /// The thinking text, shown when the group is expanded.
    pub detail: Option<String>,
    pub state: StepState,
}

/// A row in the conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    User(String),
    Assistant(String),
    /// Consecutive thinking and tool calls, drawn collapsed to one summary line.
    Activity(Vec<Step>),
    Error(String),
    /// A note from the app itself, such as "Stopped."
    Notice(String),
}

/// The summary line of a collapsed activity group, e.g. "Thought, read 2
/// files, ran a command". Kinds are listed in the order they first appear.
pub fn summarize(steps: &[Step]) -> String {
    let mut counts: Vec<(Kind, usize)> = Vec::new();
    for step in steps {
        match counts.iter_mut().find(|(kind, _)| *kind == step.kind) {
            Some((_, count)) => *count += 1,
            None => counts.push((step.kind, 1)),
        }
    }
    let text = counts
        .into_iter()
        .map(|(kind, count)| kind.phrase(count))
        .collect::<Vec<_>>()
        .join(", ");
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The conversation of one managed worktree, built from [`Event`]s.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transcript {
    pub entries: Vec<Entry>,
}

impl Transcript {
    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// Adds an event. [`Event::Session`] is the caller's to handle.
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Session(_) => {}
            Event::Text(text) if text.trim().is_empty() => {}
            Event::Text(text) => match self.entries.last_mut() {
                // A turn's text blocks read as one reply.
                Some(Entry::Assistant(reply)) => {
                    reply.push_str("\n\n");
                    reply.push_str(text.trim());
                }
                _ => self.push(Entry::Assistant(text.trim().to_owned())),
            },
            Event::Thinking(text) => self.push_step(Step {
                id: None,
                kind: Kind::Thinking,
                summary: "Thinking".into(),
                detail: Some(text.trim().to_owned()).filter(|text| !text.is_empty()),
                state: StepState::Done,
            }),
            Event::Tool {
                id,
                kind,
                summary,
                state,
            } => {
                let existing = self
                    .entries
                    .iter_mut()
                    .rev()
                    .filter_map(|entry| match entry {
                        Entry::Activity(steps) => Some(steps),
                        _ => None,
                    })
                    .flatten()
                    .find(|step| step.id.as_deref() == Some(&id));
                match existing {
                    Some(step) => step.state = state,
                    None => self.push_step(Step {
                        id: Some(id),
                        kind,
                        summary: if summary.is_empty() {
                            "Used a tool".into()
                        } else {
                            summary
                        },
                        detail: None,
                        state,
                    }),
                }
            }
            Event::Failed(reason) => self.push(Entry::Error(reason)),
        }
    }

    fn push_step(&mut self, step: Step) {
        match self.entries.last_mut() {
            Some(Entry::Activity(steps)) => steps.push(step),
            _ => self.push(Entry::Activity(vec![step])),
        }
    }

    /// Marks calls still running as finished, when the turn ends without
    /// reporting them (stopped, or the harness exited).
    pub fn settle(&mut self) {
        for entry in &mut self.entries {
            if let Entry::Activity(steps) = entry {
                for step in steps.iter_mut().filter(|s| s.state == StepState::Running) {
                    step.state = StepState::Failed;
                }
            }
        }
    }
}

/// A message from a running turn.
#[derive(Clone, Debug)]
pub enum Update {
    Event(Event),
    /// The harness exited. `Err` carries why the turn failed: it could not
    /// start, or it exited unsuccessfully (with the end of its stderr).
    Finished(Result<(), String>),
}

/// Runs one turn of `settings` in `cwd`, streaming its events. Dropping the
/// stream (an aborted `Task`) kills the harness.
pub fn run_turn(
    settings: &Settings,
    prompt: &str,
    cwd: PathBuf,
) -> impl iced::futures::Stream<Item = Update> + use<> {
    let harness = settings.harness;
    let invocation = invocation(settings, prompt);
    iced::stream::channel(64, async move |mut output| {
        use iced::futures::SinkExt;
        let result = match runner::find(harness.tool()) {
            Ok(program) => {
                let send = |update| {
                    let mut output = output.clone();
                    async move {
                        // The receiver is gone only when the run was stopped.
                        let _ = output.send(update).await;
                    }
                };
                stream_process(harness, &program, invocation, &cwd, send).await
            }
            Err(error) => Err(error),
        };
        let _ = output.send(Update::Finished(result)).await;
    })
}

/// Starts the harness and sends an [`Update::Event`] for each event it prints.
async fn stream_process<F, Fut>(
    harness: Harness,
    program: &Path,
    invocation: Invocation,
    cwd: &Path,
    send: F,
) -> Result<(), String>
where
    F: Fn(Update) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let mut command = runner::command(program);
    command.args(&invocation.args).current_dir(cwd);
    if harness == Harness::Claude {
        // Claude Code refuses to start inside another Claude Code session,
        // which it detects from these (set when the app is launched from one).
        command
            .env_remove("CLAUDECODE")
            .env_remove("CLAUDE_CODE_ENTRYPOINT");
    }
    // `From` turns the std command (with its cleaned environment) into Tokio's.
    let mut command = tokio::process::Command::from(command);
    command
        .stdin(if invocation.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", harness.name()))?;
    if let (Some(mut stdin), Some(prompt)) = (child.stdin.take(), invocation.stdin) {
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|error| format!("Could not send the prompt to {}: {error}", harness.name()))?;
        // Dropping stdin closes it, which tells the harness the prompt ended.
    }
    // Read stderr concurrently, so a chatty harness can't fill the pipe and stall.
    let stderr = child.stderr.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes).await;
            bytes
        })
    });
    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            for event in parse_line(harness, cwd, &line) {
                send(Update::Event(event)).await;
            }
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|error| format!("{} failed while running: {error}", harness.name()))?;
    if status.success() {
        return Ok(());
    }
    let stderr = match stderr {
        Some(task) => task.await.unwrap_or_default(),
        None => Vec::new(),
    };
    let stderr = String::from_utf8_lossy(&stderr);
    let tail: Vec<&str> = stderr.trim().lines().rev().take(3).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Err(if tail.is_empty() {
        format!("{} exited with {status}.", harness.name())
    } else {
        format!(
            "{} exited with {status}: {}",
            harness.name(),
            tail.join(" ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> PathBuf {
        PathBuf::from("/work/my tree")
    }

    fn parse(harness: Harness, lines: &[&str]) -> Transcript {
        let mut transcript = Transcript::default();
        for line in lines {
            for event in parse_line(harness, &cwd(), line) {
                transcript.apply(event);
            }
        }
        transcript
    }

    fn steps(transcript: &Transcript) -> Vec<(Kind, String, StepState)> {
        transcript
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Activity(steps) => Some(steps),
                _ => None,
            })
            .flatten()
            .map(|step| (step.kind, step.summary.clone(), step.state))
            .collect()
    }

    #[test]
    fn claude_tool_calls_become_one_line_steps_matched_to_their_results() {
        let transcript = parse(
            Harness::Claude,
            &[
                r#"{"type":"system","subtype":"init","session_id":"s-1"}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Look first."}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la","description":"List files"}}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/work/my tree/src/main.rs"}}]}}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"..."}]}}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true}]}}"#,
                r#"{"type":"assistant","parent_tool_use_id":"t9","message":{"content":[{"type":"text","text":"subagent chatter"}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}"#,
                r#"{"type":"result","subtype":"success","is_error":false,"result":"Done."}"#,
            ],
        );
        assert_eq!(
            steps(&transcript),
            [
                (Kind::Thinking, "Thinking".into(), StepState::Done),
                (Kind::Command, "List files".into(), StepState::Failed),
                (Kind::Read, "Read src/main.rs".into(), StepState::Done),
            ]
        );
        assert_eq!(transcript.entries.len(), 2);
        assert_eq!(transcript.entries[1], Entry::Assistant("Done.".into()));
        assert_eq!(
            parse_line(
                Harness::Claude,
                &cwd(),
                r#"{"type":"system","subtype":"init","session_id":"s-1"}"#
            ),
            [Event::Session("s-1".into())]
        );
        let failed = parse(
            Harness::Claude,
            &[r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#],
        );
        assert_eq!(failed.entries, [Entry::Error("error_max_turns".into())]);
    }

    #[test]
    fn codex_items_report_commands_edits_and_replies() {
        let transcript = parse(
            Harness::Codex,
            &[
                r#"{"type":"thread.started","thread_id":"th-1"}"#,
                r#"{"type":"turn.started"}"#,
                r#"{"type":"item.completed","item":{"id":"i0","type":"reasoning","text":"Plan it."}}"#,
                r#"{"type":"item.started","item":{"id":"i1","type":"command_execution","command":"/bin/zsh -lc 'cargo test'","exit_code":null,"status":"in_progress"}}"#,
                r#"{"type":"item.completed","item":{"id":"i1","type":"command_execution","command":"/bin/zsh -lc 'cargo test'","exit_code":101,"status":"failed"}}"#,
                r#"{"type":"item.completed","item":{"id":"i2","type":"file_change","changes":[{"path":"/work/my tree/a.rs","kind":"update"}],"status":"completed"}}"#,
                r#"{"type":"item.completed","item":{"id":"i3","type":"agent_message","text":"Fixed."}}"#,
                r#"{"type":"turn.completed","usage":{}}"#,
            ],
        );
        assert_eq!(
            steps(&transcript),
            [
                (Kind::Thinking, "Thinking".into(), StepState::Done),
                (Kind::Command, "Ran `cargo test`".into(), StepState::Failed),
                (Kind::Edit, "Edited a.rs".into(), StepState::Done),
            ]
        );
        assert_eq!(
            transcript.entries.last(),
            Some(&Entry::Assistant("Fixed.".into()))
        );
        assert_eq!(unwrap_shell("/bin/zsh -lc ls"), "ls");
        assert_eq!(unwrap_shell("cargo test -lc x"), "cargo test -lc x");
        assert_eq!(
            parse_line(
                Harness::Codex,
                &cwd(),
                r#"{"type":"thread.started","thread_id":"th-1"}"#
            ),
            [Event::Session("th-1".into())]
        );
        let failed = parse(
            Harness::Codex,
            &[r#"{"type":"turn.failed","error":{"message":"quota"}}"#],
        );
        assert_eq!(failed.entries, [Entry::Error("quota".into())]);
    }

    #[test]
    fn pi_reports_script_calls_without_their_wrapper_and_surfaces_errors() {
        let transcript = parse(
            Harness::Pi,
            &[
                r#"{"type":"session","version":3,"id":"pi-1","cwd":"/work/my tree"}"#,
                r#"{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#,
                r#"{"type":"tool_execution_start","toolCallId":"c","toolName":"codemode","args":{"code":"..."}}"#,
                r#"{"type":"tool_execution_start","toolCallId":"c/1","toolName":"bash","args":{"command":"ls"},"parentToolCallId":"c"}"#,
                r#"{"type":"tool_execution_end","toolCallId":"c/1","toolName":"bash","isError":false}"#,
                r#"{"type":"tool_execution_start","toolCallId":"c/2","toolName":"read","args":{"path":"a.txt"}}"#,
                r#"{"type":"tool_execution_end","toolCallId":"c","toolName":"codemode","isError":false}"#,
                r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"ok"},{"type":"text","text":"hello"}],"stopReason":"stop"}}"#,
                r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"OAuth refresh failed"}}"#,
            ],
        );
        assert_eq!(
            steps(&transcript),
            [
                (Kind::Command, "Ran `ls`".into(), StepState::Done),
                (Kind::Read, "Read a.txt".into(), StepState::Running),
                (Kind::Thinking, "Thinking".into(), StepState::Done),
            ]
        );
        assert_eq!(
            transcript.entries[1..],
            [
                Entry::Assistant("hello".into()),
                Entry::Error("OAuth refresh failed".into())
            ]
        );
        assert_eq!(
            parse_line(Harness::Pi, &cwd(), r#"{"type":"session","id":"pi-1"}"#),
            [Event::Session("pi-1".into())]
        );
        assert!(parse_line(Harness::Pi, &cwd(), "not json").is_empty());
    }

    #[test]
    fn collapsed_activity_reads_as_a_short_summary() {
        let step = |kind| Step {
            id: None,
            kind,
            summary: String::new(),
            detail: None,
            state: StepState::Done,
        };
        assert_eq!(
            summarize(&[
                step(Kind::Thinking),
                step(Kind::Read),
                step(Kind::Command),
                step(Kind::Read),
                step(Kind::Thinking),
            ]),
            "Thought, read 2 files, ran a command"
        );
        assert_eq!(
            summarize(&[step(Kind::Edit), step(Kind::Edit)]),
            "Made 2 edits"
        );
        assert_eq!(summarize(&[]), "");
    }

    #[test]
    fn turns_pass_model_thinking_and_session_to_each_harness() {
        let args = |settings: &Settings| -> Vec<String> {
            invocation(settings, "-fix it")
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect()
        };
        let mut claude = Settings {
            harness: Harness::Claude,
            model: Some("opus".into()),
            thinking: Some("high".into()),
            session: None,
        };
        let first = invocation(&claude, "-fix it");
        assert_eq!(first.stdin.as_deref(), Some("-fix it"));
        assert!(args(&claude).windows(2).any(|w| w == ["--model", "opus"]));
        assert!(args(&claude).windows(2).any(|w| w == ["--effort", "high"]));
        assert!(!args(&claude).contains(&"--resume".to_owned()));
        claude.session = Some("s-1".into());
        assert!(args(&claude).windows(2).any(|w| w == ["--resume", "s-1"]));

        let codex = Settings {
            harness: Harness::Codex,
            model: None,
            thinking: Some("low".into()),
            session: Some("th-1".into()),
        };
        let codex_args = args(&codex);
        assert_eq!(codex_args[..2], ["exec", "resume"]);
        assert!(codex_args.contains(&r#"model_reasoning_effort="low""#.to_owned()));
        assert!(!codex_args.contains(&"-m".to_owned()), "default model");
        assert_eq!(codex_args[codex_args.len() - 2..], ["th-1", "-"]);

        let pi = Settings {
            harness: Harness::Pi,
            model: Some("openai-codex/gpt-6.2-sol".into()),
            thinking: None,
            session: Some("pi-1".into()),
        };
        let run = invocation(&pi, "-fix it");
        assert_eq!(run.stdin, None);
        let pi_args = args(&pi);
        assert!(pi_args.windows(2).any(|w| w == ["--session", "pi-1"]));
        assert_eq!(pi_args[pi_args.len() - 2..], ["--", "-fix it"]);
    }

    #[test]
    fn models_are_read_from_codex_and_pi_catalogs() {
        let codex = parse_codex_models(
            r#"{"models":[
                {"slug":"gpt-a","visibility":"list","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]},
                {"slug":"hidden","visibility":"hide"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].id.as_deref(), Some("gpt-a"));
        assert_eq!(codex[0].levels, ["low", "high"]);
        assert!(parse_codex_models("nope").is_err());

        let pi = parse_pi_models(
            "provider  model  context  max-out  thinking  images\n\
             anthropic  claude-x  1M  128K  yes  yes\n\
             local  tiny  8K  4K  no  no\n",
        );
        assert_eq!(pi.len(), 2);
        assert_eq!(pi[0].id.as_deref(), Some("anthropic/claude-x"));
        assert!(!pi[0].levels.is_empty());
        assert!(
            pi[1].levels.is_empty(),
            "no thinking levels for {:?}",
            pi[1]
        );
    }

    #[test]
    fn a_turn_streams_events_and_reports_a_failed_exit() {
        use iced::futures::StreamExt;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let fake = crate::runner::tests::script(
                dir.path(),
                "fake claude",
                r#"cat > "$PWD/prompt.txt"
printf '%s\n' '{"type":"system","subtype":"init","session_id":"s-9"}'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}'
echo "boom" >&2
exit 3"#,
            );
            let invocation = invocation(&Settings::default(), "the prompt");
            let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
            let result = stream_process(Harness::Claude, &fake, invocation, dir.path(), |update| {
                let sender = sender.clone();
                async move {
                    let _ = sender.unbounded_send(update);
                }
            })
            .await;
            drop(sender);
            let events: Vec<_> = receiver
                .filter_map(|update| async move {
                    match update {
                        Update::Event(event) => Some(event),
                        Update::Finished(_) => None,
                    }
                })
                .collect()
                .await;
            assert_eq!(
                events,
                [Event::Session("s-9".into()), Event::Text("hi".into())]
            );
            let error = result.unwrap_err();
            assert!(error.contains("boom"), "{error}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("prompt.txt")).unwrap(),
                "the prompt"
            );
        });
    }
}
