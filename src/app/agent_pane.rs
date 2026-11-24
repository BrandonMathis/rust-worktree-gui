//! Managed worktrees: the coding-agent conversation shown where the terminal
//! would be, with a prompt to pick the harness, model and thinking level.
//!
//! Each prompt runs one turn of the harness (`agent::run_turn`) as a streamed
//! `Task`. Prompts sent while a turn runs wait in a queue and go out, in
//! order, when it finishes. The harness's session id is saved with the
//! bookmark, so later turns (even after a restart) continue the conversation.

use super::view::placeholder;
use super::widgets::single_line;
use super::{App, Message, Project, ProjectId, background};
use crate::agent::{self, Entry, Harness, Level, Model, StepState};
use crate::git::Worktree;
use crate::icons;
use crate::style::{self, Button, Panel};
use crate::theme::Palette;
use iced::keyboard::{Key, key::Named};
// `text_editor` is both a module (its types) and a function (the widget);
// one `use` brings in both.
use iced::widget::text_editor;
use iced::widget::text_editor::Binding;
use iced::widget::{
    Space, button, column, container, operation, pick_list, row, scrollable, text, tooltip,
};
use iced::{Center, Element, Fill, Task};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The prompt editor's widget id. Only one is on screen at a time.
const COMPOSER: &str = "agent-composer";

/// A harness's models: listed in the background once per launch.
pub(super) enum ModelList {
    Loading,
    Ready(Vec<Model>),
    Failed(String),
}

/// The live state of one managed worktree's conversation. Its saved settings
/// (harness, model, thinking, session id) live in `Project::managed`.
#[derive(Default)]
pub(super) struct AgentPane {
    pub(super) transcript: agent::Transcript,
    pub(super) composer: text_editor::Content,
    /// Prompts waiting for the running turn to finish.
    pub(super) queue: VecDeque<String>,
    pub(super) run: Option<Run>,
    /// Activity groups (transcript indices) the user expanded.
    expanded: BTreeSet<usize>,
}

/// The turn in flight. Dropping it aborts the turn's task, which kills the
/// harness process (`kill_on_drop`).
pub(super) struct Run {
    pub(super) id: u64,
    _abort: iced::task::Handle,
}

impl AgentPane {
    pub(super) fn is_running(&self) -> bool {
        self.run.is_some()
    }

    /// Screenshot builds only: a conversation mid-turn, with a follow-up
    /// queued, built from the same events a harness would send.
    #[cfg(feature = "capture")]
    pub(super) fn sample_conversation() -> Self {
        use agent::{Event, Kind, StepState::*};
        let mut pane = Self::default();
        let transcript = &mut pane.transcript;
        let tool = |id: &str, kind, summary: &str, state| Event::Tool {
            id: id.into(),
            kind,
            summary: summary.into(),
            state,
        };
        transcript.push(Entry::User(
            "Check the dark theme's text colors against WCAG AA and fix any that fail.".into(),
        ));
        for event in [
            Event::Thinking("Find where the tokens are defined, then compute contrast.".into()),
            tool("1", Kind::Search, "Searched for “foreground”", Done),
            tool("2", Kind::Read, "Read src/tokens/dark.ts", Done),
            tool("3", Kind::Read, "Read src/tokens/light.ts", Done),
            tool("4", Kind::Command, "Ran `npm run contrast -- dark`", Failed),
            Event::Text(
                "Two dark-theme tokens fail AA for body text: `comment` (3.1:1) and \
                 `muted` (3.9:1) on the background. I'll lighten both and keep their hues."
                    .into(),
            ),
            tool("5", Kind::Edit, "Edited src/tokens/dark.ts", Done),
            tool(
                "6",
                Kind::Command,
                "Ran `npm run contrast -- dark`",
                Running,
            ),
        ] {
            transcript.apply(event);
        }
        pane.expanded.insert(1);
        pane.queue
            .push_back("Then add a test so the contrast check runs in CI.".into());
        let (_, handle) = Task::<Message>::none().abortable();
        pane.run = Some(Run {
            id: 0,
            _abort: handle,
        });
        pane
    }

    /// Once a conversation has started, its harness can't change: the
    /// session id belongs to that harness.
    fn started(&self, settings: &agent::Settings) -> bool {
        settings.session.is_some() || !self.transcript.entries.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum AgentMessage {
    Edit(PathBuf, text_editor::Action),
    Harness(PathBuf, Harness),
    Model(PathBuf, Model),
    Thinking(PathBuf, Level),
    Submit(PathBuf),
    Stop(PathBuf),
    RemoveQueued(PathBuf, usize),
    ToggleActivity(PathBuf, usize),
    /// An update from turn `u64` of a worktree.
    Turn(PathBuf, u64, agent::Update),
    ModelsListed(Harness, Result<Vec<Model>, String>),
}

impl Project {
    pub(super) fn is_managed(&self, path: &Path) -> bool {
        self.managed.contains_key(path)
    }
}

impl App {
    /// The selected worktree, if it is managed and available.
    pub(super) fn selected_managed(&self) -> Option<(&Project, &Worktree)> {
        let (project, selection) = self.selected()?;
        let tree = project.worktree(selection.worktree.as_deref()?)?;
        (project.is_managed(&tree.path) && tree.unavailable_reason().is_none())
            .then_some((project, tree))
    }

    fn managed_settings(&self, path: &Path) -> Option<(ProjectId, &agent::Settings)> {
        self.projects
            .iter()
            .find_map(|p| Some((p.id, p.managed.get(path)?)))
    }

    /// Marks `path` as managed with `settings`, or as a terminal worktree
    /// (`None`), and saves the bookmarks.
    pub(super) fn set_managed(
        &mut self,
        id: ProjectId,
        path: &Path,
        settings: Option<agent::Settings>,
    ) -> Result<(), String> {
        let project = self
            .projects
            .iter()
            .find(|p| p.id == id)
            .ok_or("The project is no longer bookmarked.")?;
        let mut updated = project.bookmark();
        match settings {
            Some(settings) => updated.managed.insert(path.to_path_buf(), settings),
            None => updated.managed.remove(path),
        };
        self.save(self.projects.iter().map(|p| {
            if p.id == id {
                updated.clone()
            } else {
                p.bookmark()
            }
        }))?;
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
            project.managed = updated.managed;
        }
        Ok(())
    }

    /// Changes a managed worktree's saved settings. The change is kept even
    /// if saving fails, so the next turn uses it; the error is shown.
    fn change_settings(&mut self, path: &Path, change: impl FnOnce(&mut agent::Settings)) {
        let Some((id, settings)) = self.managed_settings(path) else {
            return;
        };
        let mut settings = settings.clone();
        change(&mut settings);
        if let Err(error) = self.set_managed(id, path, Some(settings.clone())) {
            self.notice = Some(error);
            if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                project.managed.insert(path.to_path_buf(), settings);
            }
        }
    }

    pub(super) fn agents_running(&self) -> bool {
        self.agents.values().any(AgentPane::is_running)
    }

    /// Opens the selected managed worktree: lists its harness's models if
    /// needed and focuses the prompt.
    pub(super) fn open_agent(&mut self) -> Task<Message> {
        let Some((project, tree)) = self.selected_managed() else {
            return Task::none();
        };
        let harness = project
            .managed
            .get(&tree.path)
            .map_or(Harness::Claude, |s| s.harness);
        let path = tree.path.clone();
        self.agents.entry(path).or_default();
        Task::batch([self.list_models(harness), self.focus_terminal()])
    }

    pub(super) fn focus_composer(&self) -> Task<Message> {
        operation::focus(COMPOSER)
    }

    /// Lists `harness`'s models, unless they're listed or being listed. A
    /// failed listing is tried again.
    fn list_models(&mut self, harness: Harness) -> Task<Message> {
        if matches!(
            self.agent_models.get(&harness),
            Some(ModelList::Loading | ModelList::Ready(_))
        ) {
            return Task::none();
        }
        self.agent_models.insert(harness, ModelList::Loading);
        background(
            move || agent::list_models(harness),
            move |result| Message::Agent(AgentMessage::ModelsListed(harness, result)),
        )
    }

    /// Puts a path dropped on the window into the prompt.
    pub(super) fn paste_into_composer(&mut self, dropped: &Path) -> bool {
        let Some(path) = self.selected_managed().map(|(_, tree)| tree.path.clone()) else {
            return false;
        };
        let pane = self.agents.entry(path).or_default();
        let text = format!("{} ", dropped.display());
        pane.composer
            .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                Arc::new(text),
            )));
        true
    }

    pub(super) fn agent_message(&mut self, message: AgentMessage) -> Task<Message> {
        match message {
            AgentMessage::Edit(path, action) => {
                self.agents
                    .entry(path)
                    .or_default()
                    .composer
                    .perform(action);
                Task::none()
            }
            AgentMessage::Harness(path, harness) => {
                let started = self.managed_settings(&path).is_none_or(|(_, settings)| {
                    self.agents
                        .get(&path)
                        .is_some_and(|pane| pane.started(settings))
                });
                if started {
                    return Task::none();
                }
                self.change_settings(&path, |settings| {
                    *settings = agent::Settings {
                        harness,
                        ..Default::default()
                    };
                });
                Task::batch([self.list_models(harness), self.focus_composer()])
            }
            AgentMessage::Model(path, model) => {
                self.change_settings(&path, |settings| {
                    if settings
                        .thinking
                        .as_ref()
                        .is_some_and(|level| !model.levels.contains(level))
                    {
                        settings.thinking = None;
                    }
                    settings.model = model.id;
                });
                self.focus_composer()
            }
            AgentMessage::Thinking(path, level) => {
                self.change_settings(&path, |settings| settings.thinking = level.0);
                self.focus_composer()
            }
            AgentMessage::Submit(path) => self.submit(path),
            AgentMessage::Stop(path) => {
                self.stop(&path);
                self.focus_composer()
            }
            AgentMessage::RemoveQueued(path, index) => {
                if let Some(pane) = self.agents.get_mut(&path) {
                    pane.queue.remove(index);
                }
                Task::none()
            }
            AgentMessage::ToggleActivity(path, index) => {
                if let Some(pane) = self.agents.get_mut(&path)
                    && !pane.expanded.remove(&index)
                {
                    pane.expanded.insert(index);
                }
                Task::none()
            }
            AgentMessage::Turn(path, id, update) => self.turn_update(path, id, update),
            AgentMessage::ModelsListed(harness, result) => {
                self.agent_models.insert(
                    harness,
                    match result {
                        Ok(models) => ModelList::Ready(models),
                        Err(error) => ModelList::Failed(error),
                    },
                );
                Task::none()
            }
        }
    }

    /// Sends the prompt, or queues it while a turn runs.
    fn submit(&mut self, path: PathBuf) -> Task<Message> {
        let available = self
            .projects
            .iter()
            .filter(|p| p.is_managed(&path))
            .filter_map(|p| p.worktree(&path))
            .any(|tree| tree.unavailable_reason().is_none());
        let Some(pane) = self.agents.get_mut(&path).filter(|_| available) else {
            return Task::none();
        };
        let prompt = pane.composer.text().trim().to_owned();
        if prompt.is_empty() {
            return Task::none();
        }
        pane.composer = text_editor::Content::new();
        if pane.is_running() {
            pane.queue.push_back(prompt);
            return Task::none();
        }
        self.start_turn(path, prompt)
    }

    fn start_turn(&mut self, path: PathBuf, prompt: String) -> Task<Message> {
        let Some(settings) = self.managed_settings(&path).map(|(_, s)| s.clone()) else {
            return Task::none();
        };
        self.next_turn_id += 1;
        let id = self.next_turn_id;
        let stream = agent::run_turn(&settings, &prompt, path.clone());
        let pane = self.agents.entry(path.clone()).or_default();
        pane.transcript.push(Entry::User(prompt));
        // `Task::run` turns each streamed update into a message; `abortable`
        // returns a handle that stops the stream (and so the harness).
        let (task, handle) = Task::run(stream, move |update| {
            Message::Agent(AgentMessage::Turn(path.clone(), id, update))
        })
        .abortable();
        pane.run = Some(Run {
            id,
            _abort: handle.abort_on_drop(),
        });
        task
    }

    fn turn_update(&mut self, path: PathBuf, id: u64, update: agent::Update) -> Task<Message> {
        let current = self
            .agents
            .get(&path)
            .and_then(|pane| pane.run.as_ref())
            .is_some_and(|run| run.id == id);
        if !current {
            return Task::none();
        }
        match update {
            agent::Update::Event(agent::Event::Session(session)) => {
                let known = self
                    .managed_settings(&path)
                    .is_some_and(|(_, s)| s.session.as_ref() == Some(&session));
                if !known {
                    self.change_settings(&path, |settings| settings.session = Some(session));
                }
                Task::none()
            }
            agent::Update::Event(event) => {
                if let Some(pane) = self.agents.get_mut(&path) {
                    pane.transcript.apply(event);
                }
                Task::none()
            }
            agent::Update::Finished(result) => {
                let Some(pane) = self.agents.get_mut(&path) else {
                    return Task::none();
                };
                pane.run = None;
                pane.transcript.settle();
                if let Err(error) = result {
                    pane.transcript.push(Entry::Error(error));
                }
                match pane.queue.pop_front() {
                    Some(next) => self.start_turn(path, next),
                    None => Task::none(),
                }
            }
        }
    }

    /// Stops the running turn. Queued prompts go back into the editor, so
    /// nothing typed is lost and they can be edited before sending.
    fn stop(&mut self, path: &Path) {
        let Some(pane) = self.agents.get_mut(path) else {
            return;
        };
        if pane.run.take().is_none() {
            return;
        }
        pane.transcript.settle();
        pane.transcript.push(Entry::Notice("Stopped.".into()));
        let mut restored: Vec<String> = pane.queue.drain(..).collect();
        if !restored.is_empty() {
            let draft = pane.composer.text();
            if !draft.trim().is_empty() {
                restored.insert(0, draft.trim().to_owned());
            }
            pane.composer = text_editor::Content::with_text(&restored.join("\n\n"));
        }
    }

    /// A managed worktree's header status and its pane: the conversation and
    /// the prompt, or a placeholder when the worktree is unavailable.
    pub(super) fn agent_workspace<'a>(
        &'a self,
        tree: &'a Worktree,
        settings: &'a agent::Settings,
    ) -> (Element<'a, Message>, Element<'a, Message>) {
        let p = self.palette();
        if let Some(reason) = tree.unavailable_reason() {
            return (
                row![
                    icons::warning(13, p.red),
                    text("Unavailable").size(11).color(p.red)
                ]
                .spacing(3)
                .align_y(Center)
                .into(),
                placeholder(reason, None, p),
            );
        }
        let pane = self.agents.get(&tree.path);
        let running = pane.is_some_and(AgentPane::is_running);
        let mut status = row![
            icons::harness(settings.harness, 13, p),
            text(if running {
                format!("{} is working…", settings.harness)
            } else {
                settings.harness.to_string()
            })
            .size(11)
            .color(if running { p.purple } else { p.muted() }),
        ]
        .spacing(4)
        .align_y(Center);
        if running {
            status = status.push(
                button(text("Stop").size(11))
                    .padding([2, 6])
                    .style(style::button(p, Button::Secondary))
                    .on_press(Message::Agent(AgentMessage::Stop(tree.path.clone()))),
            );
        }
        let Some(pane) = pane else {
            return (status.into(), placeholder("Opening…", None, p));
        };
        let composer = self.composer(tree, settings, pane, p);
        let body: Element<'a, Message> = if pane.transcript.entries.is_empty() {
            // A fresh conversation: the prompt sits in the middle of the pane.
            container(
                column![
                    text("What should the agent do in this worktree?").size(17),
                    text(tree.path.display().to_string())
                        .size(12)
                        .color(p.muted()),
                    composer,
                ]
                .spacing(8)
                .max_width(680),
            )
            .padding(16)
            .center(Fill)
            .into()
        } else {
            let mut queued = column![].spacing(3);
            for (index, prompt) in pane.queue.iter().enumerate() {
                queued = queued.push(
                    container(
                        row![
                            text("Queued").size(11).color(p.purple),
                            single_line(prompt.replace('\n', " "), 12),
                            button(text("Remove").size(11))
                                .padding([1, 5])
                                .style(style::button(p, Button::Nav))
                                .on_press(Message::Agent(AgentMessage::RemoveQueued(
                                    tree.path.clone(),
                                    index,
                                ))),
                        ]
                        .spacing(6)
                        .align_y(Center),
                    )
                    .padding([3, 7])
                    .style(style::panel(p, Panel::Glass)),
                );
            }
            column![
                scrollable(
                    container(self.transcript_view(tree, settings, pane, p))
                        .padding([10, 14])
                        .max_width(820)
                        .center_x(Fill),
                )
                // Stay pinned to the newest message while it is in view.
                .anchor_bottom()
                .height(Fill),
                container(column![queued, composer].spacing(6).max_width(820))
                    .padding([6, 14])
                    .center_x(Fill),
            ]
            .into()
        };
        (status.into(), body)
    }

    fn transcript_view<'a>(
        &'a self,
        tree: &'a Worktree,
        settings: &'a agent::Settings,
        pane: &'a AgentPane,
        p: Palette,
    ) -> Element<'a, Message> {
        let mut list = column![].spacing(10).width(Fill);
        let last = pane.transcript.entries.len().saturating_sub(1);
        for (index, entry) in pane.transcript.entries.iter().enumerate() {
            let item: Element<'a, Message> = match entry {
                Entry::User(prompt) => container(
                    container(text(prompt).size(13))
                        .padding([6, 10])
                        .max_width(620)
                        .style(style::panel(p, Panel::Glass)),
                )
                .align_right(Fill)
                .into(),
                Entry::Assistant(reply) => row![
                    container(icons::harness(settings.harness, 14, p)).padding([2, 0]),
                    text(reply).size(13).width(Fill),
                ]
                .spacing(8)
                .into(),
                Entry::Activity(steps) => {
                    let live = pane.is_running() && index == last;
                    activity(
                        steps,
                        pane.expanded.contains(&index),
                        live,
                        Message::Agent(AgentMessage::ToggleActivity(tree.path.clone(), index)),
                        p,
                    )
                }
                Entry::Error(error) => {
                    row![icons::warning(13, p.red), text(error).size(12).color(p.red)]
                        .spacing(6)
                        .align_y(Center)
                        .into()
                }
                Entry::Notice(notice) => text(notice).size(12).color(p.muted()).into(),
            };
            list = list.push(item);
        }
        if pane.is_running() && !matches!(pane.transcript.entries.last(), Some(Entry::Activity(_)))
        {
            list = list.push(
                row![
                    icons::connecting(12, p.purple),
                    text("Working…").size(12).color(p.muted())
                ]
                .spacing(6)
                .align_y(Center),
            );
        }
        list.into()
    }

    /// The prompt editor with the harness, model and thinking pickers.
    fn composer<'a>(
        &'a self,
        tree: &'a Worktree,
        settings: &'a agent::Settings,
        pane: &'a AgentPane,
        p: Palette,
    ) -> Element<'a, Message> {
        let path = tree.path.clone();
        let submit = Message::Agent(AgentMessage::Submit(path.clone()));
        let editor_path = path.clone();
        let editor = text_editor(&pane.composer)
            .id(COMPOSER)
            .placeholder(format!(
                "Ask {} to make a change…  (Enter sends, Shift+Enter adds a line)",
                settings.harness
            ))
            .on_action(move |action| {
                Message::Agent(AgentMessage::Edit(editor_path.clone(), action))
            })
            // Enter sends; Shift+Enter falls through to the default (a new line).
            .key_binding(move |press| match press.key {
                Key::Named(Named::Enter) if !press.modifiers.shift() => {
                    Some(Binding::Custom(submit.clone()))
                }
                _ => Binding::from_key_press(press),
            })
            .size(13)
            .padding(8)
            .min_height(44)
            .max_height(200);

        let harness_choice: Element<'a, Message> = if pane.started(settings) {
            // The conversation belongs to its harness now.
            tooltip(
                row![
                    icons::harness(settings.harness, 13, p),
                    text(settings.harness.name()).size(12)
                ]
                .spacing(4)
                .align_y(Center),
                container(text("The harness is fixed once the conversation starts.").size(11))
                    .padding([3, 5])
                    .style(style::panel(p, Panel::Console)),
                tooltip::Position::Top,
            )
            .into()
        } else {
            row(Harness::ALL.map(|harness| {
                button(
                    row![
                        icons::harness(harness, 13, p),
                        text(harness.name()).size(12)
                    ]
                    .spacing(4)
                    .align_y(Center),
                )
                .padding([3, 7])
                .style(style::button(
                    p,
                    if harness == settings.harness {
                        Button::Selected
                    } else {
                        Button::Nav
                    },
                ))
                .on_press(Message::Agent(AgentMessage::Harness(path.clone(), harness)))
                .into()
            }))
            .spacing(2)
            .into()
        };

        let current = Model::new(settings.harness, settings.model.as_deref());
        let (models, model_note) = match self.agent_models.get(&settings.harness) {
            Some(ModelList::Ready(models)) => (models.clone(), None),
            Some(ModelList::Failed(error)) => (vec![current.clone()], Some(error.as_str())),
            Some(ModelList::Loading) | None => (vec![current.clone()], None),
        };
        let selected = models
            .iter()
            .find(|model| model.id == settings.model)
            .cloned()
            .unwrap_or(current);
        let levels: Vec<Level> = std::iter::once(Level(None))
            .chain(selected.levels.iter().cloned().map(Some).map(Level))
            .collect();
        let model_path = path.clone();
        let level_path = path.clone();
        let model_pick = pick_list(models, Some(selected), move |model| {
            Message::Agent(AgentMessage::Model(model_path.clone(), model))
        })
        .text_size(12)
        .padding([3, 7])
        .style(style::action_picker(p))
        .menu_style(style::action_menu(p));
        let level_pick = pick_list(
            levels,
            Some(Level(settings.thinking.clone())),
            move |level| Message::Agent(AgentMessage::Thinking(level_path.clone(), level)),
        )
        .text_size(12)
        .padding([3, 7])
        .style(style::action_picker(p))
        .menu_style(style::action_menu(p));

        let mut controls = row![
            harness_choice,
            model_pick,
            level_pick,
            Space::new().width(Fill)
        ]
        .spacing(6)
        .align_y(Center);
        if pane.is_running() {
            controls = controls.push(
                button(text("Stop").size(12))
                    .style(style::button(p, Button::Secondary))
                    .on_press(Message::Agent(AgentMessage::Stop(path.clone()))),
            );
        }
        let has_text = !pane.composer.text().trim().is_empty();
        controls = controls.push(
            button(text(if pane.is_running() { "Queue" } else { "Send" }).size(12))
                .style(style::button(p, Button::Primary))
                .on_press_maybe(
                    has_text.then_some(Message::Agent(AgentMessage::Submit(path.clone()))),
                ),
        );
        let mut body = column![editor, controls].spacing(6);
        if let Some(note) = model_note {
            body = body.push(text(note).size(11).color(p.red));
        }
        container(body)
            .padding(8)
            .width(Fill)
            .style(style::panel(p, Panel::Glass))
            .into()
    }
}

/// One collapsed group of thinking and tool calls: a summary line ("Read 2
/// files, ran a command") that expands to one line per step.
fn activity<'a>(
    steps: &'a [agent::Step],
    expanded: bool,
    live: bool,
    toggle: Message,
    p: Palette,
) -> Element<'a, Message> {
    let mut header = row![
        if expanded {
            icons::chevron_down(12, p.muted())
        } else {
            icons::chevron_right(12, p.muted())
        },
        text(agent::summarize(steps)).size(12).color(p.muted()),
    ]
    .spacing(5)
    .align_y(Center);
    // While the turn runs, show what the newest step is doing.
    if live && let Some(step) = steps.last() {
        header = header.push(icons::connecting(11, p.purple)).push(
            container(
                text(step.summary.as_str())
                    .size(12)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .clip(true),
        );
    }
    let mut group = column![
        button(header)
            .padding([2, 4])
            .style(style::button(p, Button::Nav))
            .on_press(toggle)
    ]
    .spacing(2);
    if expanded {
        let mut list = column![].spacing(3);
        for step in steps {
            let icon = match step.state {
                StepState::Running => icons::connecting(11, p.purple),
                StepState::Done => icons::check(11, p.green),
                StepState::Failed => icons::warning(11, p.red),
            };
            list = list.push(
                row![icon, text(step.summary.as_str()).size(12)]
                    .spacing(6)
                    .align_y(Center),
            );
            if let Some(detail) = &step.detail {
                list = list.push(
                    container(text(detail.as_str()).size(12).color(p.muted()))
                        .padding(iced::padding::left(17)),
                );
            }
        }
        group = group.push(container(list).padding(iced::padding::left(22)));
    }
    group.into()
}
