//! Managed worktrees: the coding agents ("harnesses") a worktree can be handed
//! to, the messages exchanged with the agent bridge, and the transcript the
//! workspace shows.
//!
//! The bridge (`agent-bridge/bridge.mjs`, run by `agent_bridge.rs`) drives each
//! harness through its official SDK and reports what happens as one JSON object
//! per line. [`Event`] is that wire format and [`Command`] is what the app sends
//! back. [`Transcript`] folds events into the list the UI draws, in which
//! thinking and tool calls are one-line summaries ([`tool_summary`]).

use iced::widget::markdown;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::path::Path;

/// A coding agent the app can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
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
}

/// A model a harness offers, with the thinking levels it accepts.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Thinking (reasoning effort) levels, lowest first. Empty when the model
    /// has no such setting.
    #[serde(default)]
    pub thinking: Vec<String>,
    /// The level the harness uses when none is chosen, if it says.
    pub default_thinking: Option<String>,
}

// `pick_list` shows its options with `Display`.
impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// The models a harness offers to the signed-in user.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Models {
    pub models: Vec<Model>,
    pub default_model: Option<String>,
}

impl Models {
    pub fn get(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|model| model.id == id)
    }

    /// The harness's default model, or the first one it lists.
    pub fn preferred(&self) -> Option<&Model> {
        self.default_model
            .as_deref()
            .and_then(|id| self.get(id))
            .or_else(|| self.models.first())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Done,
    Interrupted,
    Failed,
}

/// One line from the bridge. `#[serde(tag = "type")]` reads the variant from
/// the object's `"type"` field, like a discriminated union in TypeScript.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// The answer to [`Command::Models`].
    Models(Models),
    /// Assistant prose. `delta` appends to the item `id`; `text` replaces it.
    Text {
        id: String,
        delta: Option<String>,
        text: Option<String>,
    },
    /// A reasoning summary, with the same append-or-replace rule.
    Thinking {
        id: String,
        delta: Option<String>,
        text: Option<String>,
    },
    /// A tool call started or changed. Later events for the same `id` may
    /// leave out `name` and `input`.
    Tool {
        id: String,
        name: Option<String>,
        input: Option<Value>,
        status: ToolStatus,
        output: Option<String>,
    },
    /// Something the user should know, such as a declined permission.
    Notice { text: String },
    /// The harness finished its turn and can take the next prompt.
    TurnEnd {
        status: TurnStatus,
        error: Option<String>,
    },
    /// The bridge could not do what a command asked.
    Error { message: String },
}

impl Event {
    /// Parses one line of bridge output. Anything that is not an event (a
    /// stray log line from a dependency) is `None`.
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok()
    }
}

/// One line to the bridge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Models {
        harness: Harness,
    },
    /// Starts the conversation, in the bridge's working directory.
    Start {
        harness: Harness,
        model: Option<String>,
        thinking: Option<String>,
        /// The model has thinking levels, so it can be asked to summarize its
        /// reasoning (Claude returns no thinking text otherwise).
        thinking_summaries: bool,
    },
    Prompt {
        text: String,
    },
    Interrupt,
}

/// One entry in the transcript.
pub enum Item {
    User(String),
    Text {
        id: String,
        source: String,
        /// `source` parsed for the Markdown widget, kept so a view doesn't
        /// re-parse the whole reply on every frame.
        content: markdown::Content,
    },
    Thinking {
        id: String,
        text: String,
        open: bool,
    },
    Tool {
        id: String,
        name: String,
        input: Value,
        status: ToolStatus,
        output: String,
        open: bool,
    },
    Notice(String),
}

/// A conversation as the workspace shows it.
#[derive(Default)]
pub struct Transcript {
    items: Vec<Item>,
    /// Index of the first item of the current turn. Harnesses reuse item ids
    /// from turn to turn (Codex numbers them from zero each time), so an event
    /// only ever updates an item of the turn in progress.
    turn_start: usize,
}

impl Transcript {
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Adds the user's prompt and starts a new turn.
    pub fn push_user(&mut self, text: String) {
        self.items.push(Item::User(text));
        self.turn_start = self.items.len();
    }

    pub fn push_notice(&mut self, text: String) {
        self.items.push(Item::Notice(text));
    }

    /// Folds a streamed event into the transcript. Events that aren't
    /// transcript content (`Models`, `TurnEnd`, `Error`) are ignored: the app
    /// handles those.
    pub fn apply(&mut self, event: Event) {
        let turn = &mut self.items[self.turn_start..];
        match event {
            Event::Text { id, delta, text } => {
                let existing = turn.iter_mut().rev().find_map(|item| match item {
                    Item::Text {
                        id: item_id,
                        source,
                        content,
                    } if *item_id == id => Some((source, content)),
                    _ => None,
                });
                match existing {
                    Some((source, content)) => {
                        if let Some(text) = text {
                            *content = markdown::Content::parse(&text);
                            *source = text;
                        } else if let Some(delta) = delta {
                            content.push_str(&delta);
                            source.push_str(&delta);
                        }
                    }
                    None => {
                        let source = text.or(delta).unwrap_or_default();
                        self.items.push(Item::Text {
                            id,
                            content: markdown::Content::parse(&source),
                            source,
                        });
                    }
                }
            }
            Event::Thinking { id, delta, text } => {
                let existing = turn.iter_mut().rev().find_map(|item| match item {
                    Item::Thinking {
                        id: item_id, text, ..
                    } if *item_id == id => Some(text),
                    _ => None,
                });
                match existing {
                    Some(current) => {
                        if let Some(text) = text {
                            *current = text;
                        } else if let Some(delta) = delta {
                            current.push_str(&delta);
                        }
                    }
                    None => self.items.push(Item::Thinking {
                        id,
                        text: text.or(delta).unwrap_or_default(),
                        open: false,
                    }),
                }
            }
            Event::Tool {
                id,
                name,
                input,
                status,
                output,
            } => {
                let existing = turn.iter_mut().rev().find(|item| {
                    matches!(
                        item,
                        Item::Tool { id: item_id, .. } if *item_id == id
                    )
                });
                match existing {
                    Some(Item::Tool {
                        name: current_name,
                        input: current_input,
                        status: current_status,
                        output: current_output,
                        ..
                    }) => {
                        if let Some(name) = name {
                            *current_name = name;
                        }
                        if let Some(input) = input {
                            *current_input = input;
                        }
                        if let Some(output) = output {
                            *current_output = output;
                        }
                        *current_status = status;
                    }
                    _ => self.items.push(Item::Tool {
                        id,
                        name: name.unwrap_or_else(|| "tool".into()),
                        input: input.unwrap_or(Value::Null),
                        status,
                        output: output.unwrap_or_default(),
                        open: false,
                    }),
                }
            }
            Event::Notice { text } => self.push_notice(text),
            Event::Models(_) | Event::TurnEnd { .. } | Event::Error { .. } => {}
        }
    }

    /// Closes the current turn: a tool call still marked running will never
    /// report back, so it is shown as stopped rather than spinning forever.
    pub fn end_turn(&mut self, status: TurnStatus, error: Option<String>) {
        for item in &mut self.items[self.turn_start..] {
            if let Item::Tool { status: tool, .. } = item
                && *tool == ToolStatus::Running
            {
                *tool = if status == TurnStatus::Done {
                    ToolStatus::Done
                } else {
                    ToolStatus::Failed
                };
            }
        }
        match status {
            TurnStatus::Done => {}
            TurnStatus::Interrupted => self.push_notice("Stopped.".into()),
            TurnStatus::Failed => {
                self.push_notice(error.unwrap_or_else(|| "The turn failed.".into()));
            }
        }
    }

    /// Expands or collapses a thinking or tool row.
    pub fn toggle(&mut self, index: usize) {
        if let Some(Item::Thinking { open, .. } | Item::Tool { open, .. }) =
            self.items.get_mut(index)
        {
            *open = !*open;
        }
    }
}

/// The longest summary shown on one collapsed row.
const SUMMARY_CHARS: usize = 96;

/// Cuts `text` to its first line and at most `limit` characters.
fn one_line(text: &str, limit: usize) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty());
    let line = line.unwrap_or_default().trim();
    if line.chars().count() > limit {
        let cut: String = line.chars().take(limit.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_owned()
    }
}

/// What a collapsed thinking row says: the first line of the reasoning
/// summary, without Markdown emphasis. `None` while there is no text yet (or
/// the harness doesn't share it).
pub fn thinking_summary(text: &str) -> Option<String> {
    let line = one_line(text, SUMMARY_CHARS).replace(['*', '`', '#'], "");
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// A string field of a tool's input, under the first of `keys` that is set.
fn field<'a>(input: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| input.get(key)?.as_str())
        .filter(|value| !value.trim().is_empty())
}

/// A file path as the row shows it: relative to the worktree when inside it.
fn short_path(path: &str, worktree: &Path) -> String {
    let relative = Path::new(path)
        .strip_prefix(worktree)
        .ok()
        .and_then(Path::to_str)
        .filter(|relative| !relative.is_empty());
    relative.unwrap_or(path).to_owned()
}

/// Removes the login-shell wrapper Codex puts around every command
/// (`/bin/zsh -lc '…'`), leaving the command the agent actually ran.
fn unwrap_shell(command: &str) -> &str {
    let Some((shell, rest)) = command.split_once(' ') else {
        return command;
    };
    let shell = shell.rsplit('/').next().unwrap_or(shell);
    if !matches!(shell, "sh" | "bash" | "zsh") {
        return command;
    }
    let Some(script) = rest
        .strip_prefix("-lc ")
        .or_else(|| rest.strip_prefix("-c "))
    else {
        return command;
    };
    let script = script.trim();
    for quote in ['\'', '"'] {
        if script.len() >= 2 && script.starts_with(quote) && script.ends_with(quote) {
            return &script[1..script.len() - 1];
        }
    }
    script
}

/// The one-line description of a tool call shown in the transcript, such as
/// `Read src/main.rs` or `Run cargo test`, instead of the call's full input.
/// The three harnesses name their tools differently (`Read`, `read`,
/// `command_execution`), so names are matched case-insensitively.
pub fn tool_summary(name: &str, input: &Value, worktree: &Path) -> String {
    let path = |keys: &[&str]| field(input, keys).map(|path| short_path(path, worktree));
    let file = || path(&["file_path", "path", "notebook_path"]);
    let with = |verb: &str, object: Option<String>| match object {
        Some(object) => format!("{verb} {}", one_line(&object, SUMMARY_CHARS)),
        None => verb.to_owned(),
    };
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "read" => with("Read", file()),
        "write" => with("Write", file()),
        "edit" | "multiedit" | "notebookedit" => with("Edit", file()),
        "bash" | "shell" => {
            // Claude describes its own commands; prefer that to the command line.
            match field(input, &["description"]) {
                Some(description) => one_line(description, SUMMARY_CHARS),
                None => with(
                    "Run",
                    field(input, &["command"]).map(|command| unwrap_shell(command).to_owned()),
                ),
            }
        }
        "glob" | "find" => with("Find", field(input, &["pattern"]).map(str::to_owned)),
        "grep" => {
            let pattern = field(input, &["pattern"]).map(|pattern| format!("“{pattern}”"));
            match (pattern, path(&["path"])) {
                (Some(pattern), Some(place)) => {
                    one_line(&format!("Search for {pattern} in {place}"), SUMMARY_CHARS)
                }
                (pattern, _) => with("Search for", pattern),
            }
        }
        "ls" => with("List", Some(path(&["path"]).unwrap_or_else(|| ".".into()))),
        "webfetch" | "web_fetch" => with("Fetch", field(input, &["url"]).map(str::to_owned)),
        "websearch" | "web_search" => with(
            "Search the web for",
            field(input, &["query"]).map(|query| format!("“{query}”")),
        ),
        "task" | "agent" => with(
            "Agent:",
            field(input, &["description", "prompt"]).map(str::to_owned),
        ),
        "todowrite" | "todo_list" => {
            let count = ["todos", "items"]
                .iter()
                .find_map(|key| input.get(key)?.as_array())
                .map_or(0, Vec::len);
            match count {
                0 => "Update the to-do list".into(),
                1 => "Update the to-do list (1 item)".into(),
                count => format!("Update the to-do list ({count} items)"),
            }
        }
        "apply_patch" => {
            let changes = input.get("changes").and_then(Value::as_array);
            match changes.map(Vec::as_slice) {
                Some([change]) => {
                    let verb = match change.get("kind").and_then(Value::as_str) {
                        Some("add") => "Add",
                        Some("delete") => "Delete",
                        _ => "Edit",
                    };
                    with(
                        verb,
                        field(change, &["path"]).map(|path| short_path(path, worktree)),
                    )
                }
                Some(changes) if !changes.is_empty() => format!("Edit {} files", changes.len()),
                _ => "Edit files".into(),
            }
        }
        "skill" => with(
            "Use skill",
            field(input, &["skill", "name"]).map(str::to_owned),
        ),
        "toolsearch" | "tool_search" => "Look up tools".into(),
        _ => match name
            .strip_prefix("mcp__")
            .and_then(|rest| rest.split_once("__"))
        {
            // MCP tools are named `mcp__<server>__<tool>`.
            Some((server, tool)) => format!("{server}: {}", tool.replace('_', " ")),
            None => name.to_owned(),
        },
    }
}

/// The detail shown under an expanded tool row: its input, then its output.
pub fn tool_detail(input: &Value, output: &str) -> String {
    let mut detail = match input {
        Value::Null => String::new(),
        Value::Object(fields) if fields.is_empty() => String::new(),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| match value {
                Value::String(text) => format!("{key}: {text}"),
                other => format!("{key}: {other}"),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    };
    let output = output.trim_end();
    if !output.is_empty() {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str(output);
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(line: &str) -> Event {
        Event::parse(line).unwrap()
    }

    #[test]
    fn bridge_lines_parse_into_events_and_log_noise_is_skipped() {
        assert_eq!(
            event(r#"{"type":"text","id":"m:0","delta":"Hi"}"#),
            Event::Text {
                id: "m:0".into(),
                delta: Some("Hi".into()),
                text: None,
            }
        );
        assert_eq!(
            event(r#"{"type":"tool","id":"t1","status":"done","output":"ok"}"#),
            Event::Tool {
                id: "t1".into(),
                name: None,
                input: None,
                status: ToolStatus::Done,
                output: Some("ok".into()),
            }
        );
        assert_eq!(
            event(r#"{"type":"turn_end","status":"failed","error":"No credit"}"#),
            Event::TurnEnd {
                status: TurnStatus::Failed,
                error: Some("No credit".into()),
            }
        );
        let models = event(
            r#"{"type":"models","default_model":"b","models":[
                {"id":"a","name":"A","thinking":["low","high"],"default_thinking":"low"},
                {"id":"b","name":"B"}]}"#,
        );
        let Event::Models(models) = models else {
            panic!("expected a model list");
        };
        assert_eq!(models.preferred().unwrap().name, "B");
        assert_eq!(models.get("a").unwrap().thinking, ["low", "high"]);
        assert!(models.get("b").unwrap().thinking.is_empty());

        for noise in ["", "npm warn something", "{\"type\":\"unknown\"}", "[1,2]"] {
            assert!(Event::parse(noise).is_none(), "{noise:?}");
        }
    }

    #[test]
    fn commands_serialize_as_tagged_json_lines() {
        assert_eq!(
            serde_json::to_string(&Command::Start {
                harness: Harness::Codex,
                model: Some("gpt".into()),
                thinking: None,
                thinking_summaries: true,
            })
            .unwrap(),
            r#"{"type":"start","harness":"codex","model":"gpt","thinking":null,"thinking_summaries":true}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::Prompt {
                text: "two\nlines".into()
            })
            .unwrap(),
            r#"{"type":"prompt","text":"two\nlines"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::Interrupt).unwrap(),
            r#"{"type":"interrupt"}"#
        );
    }

    #[test]
    fn streamed_text_appends_and_a_full_text_replaces() {
        let mut transcript = Transcript::default();
        transcript.push_user("hello".into());
        transcript.apply(event(r#"{"type":"text","id":"a","delta":"One "}"#));
        transcript.apply(event(r#"{"type":"text","id":"a","delta":"two"}"#));
        transcript.apply(event(r#"{"type":"text","id":"b","text":"draft"}"#));
        transcript.apply(event(r#"{"type":"text","id":"b","text":"final"}"#));
        let texts: Vec<_> = transcript
            .items()
            .iter()
            .filter_map(|item| match item {
                Item::Text { source, .. } => Some(source.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["One two", "final"]);
    }

    #[test]
    fn tool_calls_update_in_place_and_keep_their_name_and_input() {
        let mut transcript = Transcript::default();
        transcript.push_user("go".into());
        transcript.apply(event(
            r#"{"type":"tool","id":"t","name":"Read","status":"running"}"#,
        ));
        transcript.apply(event(
            r#"{"type":"tool","id":"t","name":"Read","input":{"file_path":"/w/a.rs"},"status":"running"}"#,
        ));
        transcript.apply(event(
            r#"{"type":"tool","id":"t","status":"done","output":"contents"}"#,
        ));
        assert_eq!(transcript.items().len(), 2);
        let Item::Tool {
            name,
            input,
            status,
            output,
            ..
        } = &transcript.items()[1]
        else {
            panic!("expected a tool row");
        };
        assert_eq!(name, "Read");
        assert_eq!(input, &json!({"file_path": "/w/a.rs"}));
        assert_eq!(*status, ToolStatus::Done);
        assert_eq!(output, "contents");
    }

    #[test]
    fn item_ids_reused_by_a_later_turn_start_new_rows() {
        // Codex numbers its items from zero in every turn.
        let mut transcript = Transcript::default();
        transcript.push_user("first".into());
        transcript.apply(event(r#"{"type":"text","id":"item_0","text":"one"}"#));
        transcript.end_turn(TurnStatus::Done, None);
        transcript.push_user("second".into());
        transcript.apply(event(r#"{"type":"text","id":"item_0","text":"two"}"#));
        let texts: Vec<_> = transcript
            .items()
            .iter()
            .filter_map(|item| match item {
                Item::Text { source, .. } => Some(source.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["one", "two"]);
    }

    #[test]
    fn ending_a_turn_settles_running_tools_and_reports_why_it_stopped() {
        let mut transcript = Transcript::default();
        transcript.push_user("go".into());
        transcript.apply(event(
            r#"{"type":"tool","id":"t","name":"Bash","status":"running"}"#,
        ));
        transcript.end_turn(TurnStatus::Failed, Some("Rate limited".into()));
        assert!(matches!(
            transcript.items()[1],
            Item::Tool {
                status: ToolStatus::Failed,
                ..
            }
        ));
        assert!(matches!(&transcript.items()[2], Item::Notice(text) if text == "Rate limited"));

        transcript.push_user("again".into());
        transcript.end_turn(TurnStatus::Interrupted, None);
        assert!(
            matches!(transcript.items().last(), Some(Item::Notice(text)) if text == "Stopped.")
        );
    }

    #[test]
    fn thinking_and_tool_rows_start_collapsed_and_toggle() {
        let mut transcript = Transcript::default();
        transcript.push_user("go".into());
        transcript.apply(event(r#"{"type":"thinking","id":"k","delta":"Hmm"}"#));
        transcript.apply(event(
            r#"{"type":"tool","id":"t","name":"Read","status":"done"}"#,
        ));
        let open = |transcript: &Transcript| -> Vec<bool> {
            transcript
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Thinking { open, .. } | Item::Tool { open, .. } => Some(*open),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(open(&transcript), [false, false]);
        transcript.toggle(2);
        assert_eq!(open(&transcript), [false, true]);
        transcript.toggle(0); // The user's prompt has nothing to expand.
        transcript.toggle(1);
        transcript.toggle(2);
        assert_eq!(open(&transcript), [true, false]);
    }

    #[test]
    fn tool_calls_are_summarized_in_one_line_for_every_harness() {
        let worktree = Path::new("/repo/tree");
        let summary = |name: &str, input: Value| tool_summary(name, &input, worktree);
        // Claude
        assert_eq!(
            summary("Read", json!({"file_path": "/repo/tree/src/main.rs"})),
            "Read src/main.rs"
        );
        assert_eq!(
            summary(
                "Bash",
                json!({"command": "cargo test --locked", "description": "Run the tests"})
            ),
            "Run the tests"
        );
        assert_eq!(
            summary(
                "Grep",
                json!({"pattern": "fn main", "path": "/repo/tree/src"})
            ),
            "Search for “fn main” in src"
        );
        assert_eq!(
            summary("Task", json!({"description": "Audit the parser"})),
            "Agent: Audit the parser"
        );
        assert_eq!(
            summary("TodoWrite", json!({"todos": [{}, {}]})),
            "Update the to-do list (2 items)"
        );
        assert_eq!(
            summary("mcp__github__create_issue", json!({})),
            "github: create issue"
        );
        // Codex
        assert_eq!(
            summary("shell", json!({"command": "/bin/zsh -lc 'ls -la && pwd'"})),
            "Run ls -la && pwd"
        );
        assert_eq!(
            summary(
                "apply_patch",
                json!({"changes": [{"path": "/repo/tree/a.txt", "kind": "add"}]})
            ),
            "Add a.txt"
        );
        assert_eq!(
            summary("apply_patch", json!({"changes": [{}, {}, {}]})),
            "Edit 3 files"
        );
        assert_eq!(
            summary("web_search", json!({"query": "iced markdown"})),
            "Search the web for “iced markdown”"
        );
        // Pi
        assert_eq!(
            summary("write", json!({"path": "hello.txt"})),
            "Write hello.txt"
        );
        assert_eq!(
            summary("bash", json!({"command": "make\ncheck"})),
            "Run make"
        );
        assert_eq!(summary("ls", json!({})), "List .");
        // Unknown tools fall back to their name; a path outside the worktree stays whole.
        assert_eq!(summary("Frobnicate", json!({"x": 1})), "Frobnicate");
        assert_eq!(
            summary("Read", json!({"file_path": "/etc/hosts"})),
            "Read /etc/hosts"
        );
    }

    #[test]
    fn long_summaries_are_cut_to_one_short_line() {
        let command = "x".repeat(400);
        let summary = tool_summary("bash", &json!({ "command": command }), Path::new("/w"));
        assert!(summary.chars().count() <= "Run ".len() + SUMMARY_CHARS);
        assert!(summary.ends_with('…'));
        assert_eq!(
            thinking_summary("\n**Planning the change**\n\nFirst I will…").as_deref(),
            Some("Planning the change")
        );
        assert_eq!(thinking_summary("  \n"), None);
    }

    #[test]
    fn expanded_tool_detail_lists_the_input_then_the_output() {
        assert_eq!(
            tool_detail(&json!({"command": "ls", "timeout": 10}), "a\nb\n"),
            "command: ls\ntimeout: 10\n\na\nb"
        );
        assert_eq!(tool_detail(&Value::Null, ""), "");
        assert_eq!(tool_detail(&json!({}), "done"), "done");
    }
}
