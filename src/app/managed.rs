//! Managed worktrees: instead of a terminal, the workspace shows a small
//! coding-agent harness. The user picks a harness (Claude, Codex or Pi), one
//! of its models and a thinking level, and sends a prompt. Thinking and tool
//! calls appear as collapsed one-line rows.
//!
//! Each managed worktree has a [`Session`]. Its agent runs in a bridge process
//! (`agent_bridge.rs`) that keeps going while the user looks at other
//! worktrees, and stops when the app quits or the user starts a new chat.
//! Follow-up prompts sent while the agent works are queued here and go in one
//! at a time, each after the turn before it ends.

use super::widgets::single_line;
use super::{App, Message, Project, ProjectId, background};
use crate::agent::{
    self, Command, Event, Harness, Item, Model, Models, ToolStatus, Transcript, TurnStatus,
};
use crate::agent_bridge::{self, Bridge, Prepared, Update};
use crate::conversations::{self, SavedChat, SavedWorktree};
use crate::git::Worktree;
use crate::style::{self, Button, Panel};
use crate::theme::Palette;
use crate::{actions, icons};
use iced::futures::channel::mpsc;
use iced::keyboard::{Key, key::Named};
use iced::widget::{
    Space, Svg, button, column, container, markdown, operation, pick_list, row, scrollable, text,
    text_editor,
};
use iced::{Center, Element, Fill, Task};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

const PROMPT_INPUT: &str = "agent-prompt-input";

/// Everything the app knows about managed worktrees.
#[derive(Default)]
pub(super) struct Managed {
    pub(super) bridge: BridgeState,
    /// Each harness's models, loaded the first time it is picked and kept for
    /// as long as the app runs.
    pub(super) models: BTreeMap<Harness, ModelList>,
    /// One conversation per managed worktree, by worktree path.
    pub(super) sessions: BTreeMap<PathBuf, Session>,
    /// Earlier conversations per worktree, oldest first: the ones **New Chat**
    /// set aside. Their agents are stopped; resuming one starts its harness
    /// again on the conversation the harness saved. Kept while the app runs.
    pub(super) history: BTreeMap<PathBuf, Vec<Session>>,
    /// Where the conversations to resume after a restart are saved. `None`
    /// in tests, and when Application Support can't be found.
    pub(super) store: Option<conversations::Store>,
    next_session_id: u64,
    next_models_request: u64,
}

/// Whether the agent bridge and its SDKs are installed.
#[derive(Debug, Default)]
pub(super) enum BridgeState {
    #[default]
    Unknown,
    Checking,
    NeedsInstall,
    Installing,
    Ready(Bridge),
    Failed(String),
}

/// `Loading` carries its request's id, so an answer to an older request is ignored.
#[derive(Debug)]
pub(super) enum ModelList {
    Loading(u64),
    Ready(Models),
    Failed(String),
}

pub(super) struct Session {
    /// Identifies this conversation's bridge. A new chat gets a new id, so
    /// events still arriving from the old bridge are ignored.
    id: u64,
    pub(super) harness: Harness,
    /// The chosen model's id. `None` means the harness's default.
    pub(super) model: Option<String>,
    /// The chosen thinking level. `None` means the model's default.
    pub(super) thinking: Option<String>,
    editor: text_editor::Content,
    pub(super) transcript: Transcript,
    /// Follow-ups waiting for the running turn to end, oldest first.
    pub(super) queue: VecDeque<String>,
    pub(super) run: Run,
    /// A turn ended while the user was looking elsewhere.
    pub(super) unseen: bool,
    /// The harness's own id for this conversation, once it reports one. The
    /// next start resumes it, so the agent remembers what was said.
    pub(super) resume: Option<String>,
    /// The saved first prompt of a conversation restored after a restart,
    /// which names it until its messages are read back.
    saved_title: Option<String>,
    pub(super) replay: Replay,
}

/// Whether a conversation restored after a restart has its earlier messages
/// back in the transcript. They are read from the harness when it is shown.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Replay {
    /// Nothing to read back: a conversation that began in this run.
    Done,
    Pending,
    Loading,
}

/// Where a session's agent process is.
pub(super) enum Run {
    /// Nothing sent yet: the composer still offers harness, model and thinking.
    New,
    Live {
        /// Dropping this sender (with the session) ends the conversation:
        /// the bridge stops its agent and exits.
        commands: mpsc::UnboundedSender<Command>,
        /// A turn is in progress.
        working: bool,
    },
    /// The bridge stopped. The transcript stays; a new prompt starts over.
    Ended,
}

impl Session {
    fn new(id: u64) -> Self {
        Self {
            id,
            harness: Harness::Claude,
            model: None,
            thinking: None,
            editor: text_editor::Content::new(),
            transcript: Transcript::default(),
            queue: VecDeque::new(),
            run: Run::New,
            unseen: false,
            resume: None,
            saved_title: None,
            replay: Replay::Done,
        }
    }

    /// A conversation saved before the app last quit. Its agent isn't
    /// running; the next prompt resumes the harness's conversation.
    fn restored(id: u64, chat: SavedChat) -> Self {
        Self {
            harness: chat.harness,
            model: chat.model,
            thinking: chat.thinking,
            run: Run::Ended,
            resume: Some(chat.resume),
            saved_title: Some(chat.title),
            replay: Replay::Pending,
            ..Self::new(id)
        }
    }

    /// What is saved to resume this conversation after a restart. `None`
    /// until the harness has named it.
    fn saved(&self) -> Option<SavedChat> {
        Some(SavedChat {
            harness: self.harness,
            resume: self.resume.clone()?,
            model: self.model.clone(),
            thinking: self.thinking.clone(),
            title: self.title().to_owned(),
        })
    }

    /// Ends this conversation's agent so the session can be set aside, and
    /// keeps everything the user wrote: unsent follow-ups go back into the
    /// prompt field, ready for when the conversation is resumed.
    fn set_aside(mut self) -> Self {
        if self.working() {
            self.transcript.end_turn(TurnStatus::Interrupted, None);
        }
        // Dropping the sender closes the bridge's input, which stops its agent.
        if matches!(self.run, Run::Live { .. }) {
            self.run = Run::Ended;
        }
        self.unseen = false;
        self.return_queue();
        self
    }

    /// Moves queued follow-ups (then anything typed) into the prompt field.
    fn return_queue(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let mut draft: Vec<String> = self.queue.drain(..).collect();
        let typed = self.editor.text();
        if !typed.trim().is_empty() {
            draft.push(typed.trim_end().to_owned());
        }
        self.editor = text_editor::Content::with_text(&draft.join("\n\n"));
    }

    /// The conversation's first prompt, which names it in the list of earlier chats.
    pub(super) fn title(&self) -> &str {
        self.transcript
            .items()
            .iter()
            .find_map(|item| match item {
                Item::User(prompt) => Some(prompt.as_str()),
                _ => None,
            })
            .or(self.saved_title.as_deref())
            .unwrap_or("Untitled chat")
    }

    pub(super) fn working(&self) -> bool {
        matches!(self.run, Run::Live { working: true, .. })
    }

    /// Whether a conversation has begun (the harness can no longer be changed).
    fn started(&self) -> bool {
        !matches!(self.run, Run::New) || !self.transcript.is_empty()
    }

    /// The command that starts this session's conversation. The model is the
    /// one picked, else the harness's default (what the picker showed).
    pub(super) fn start_command(&self, models: Option<&Models>) -> Command {
        let chosen = models.and_then(|models| match &self.model {
            Some(id) => models.get(id),
            None => models.preferred(),
        });
        Command::Start {
            harness: self.harness,
            model: chosen
                .map(|model| model.id.clone())
                .or_else(|| self.model.clone()),
            thinking: self.thinking.clone(),
            thinking_summaries: chosen.is_some_and(|model| !model.thinking.is_empty()),
            resume: self.resume.clone(),
        }
    }

    #[cfg(test)]
    pub(super) fn draft(&self) -> String {
        self.editor.text()
    }

    /// Tests only: marks the session as running a bridge, and returns the
    /// commands the app sends to it.
    #[cfg(test)]
    pub(super) fn attach(&mut self, working: bool) -> mpsc::UnboundedReceiver<Command> {
        let (commands, receiver) = mpsc::unbounded();
        self.run = Run::Live { commands, working };
        receiver
    }

    #[cfg(test)]
    pub(super) fn id(&self) -> u64 {
        self.id
    }
}

/// An entry in the thinking drop-down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThinkingChoice {
    /// Let the harness decide. Carries the level it will use, when known.
    Default(Option<String>),
    Level(String),
}

impl std::fmt::Display for ThinkingChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default(Some(level)) => write!(f, "Default ({level})"),
            Self::Default(None) => f.write_str("Default"),
            Self::Level(level) => f.write_str(level),
        }
    }
}

/// Messages for the managed-worktree pane. The composer ones act on the
/// selected worktree's session.
#[derive(Debug, Clone)]
pub enum ManagedMessage {
    /// The bridge check finished.
    Prepared(Result<Prepared, String>),
    Install,
    Installed(Result<Bridge, String>),
    /// A harness's model list arrived, tagged with its request id.
    ModelsListed(Harness, u64, Result<Models, String>),
    RetryModels,
    Harness(Harness),
    Model(Model),
    Thinking(ThinkingChoice),
    Edit(text_editor::Action),
    Submit,
    /// Interrupt the running turn.
    Stop,
    /// Remove a queued follow-up.
    Unqueue(usize),
    NewChat,
    /// Go back to one of the selected worktree's earlier chats, by its index
    /// in [`Managed::history`].
    Resume(usize),
    /// Expand or collapse a thinking or tool row.
    Toggle(usize),
    Link(String),
    LinkOpened(Result<(), String>),
    /// A restored conversation's earlier messages: worktree, session id, events.
    HistoryLoaded(PathBuf, u64, Result<Vec<Event>, String>),
    /// Something from a session's bridge: worktree, session id, update.
    Bridge(PathBuf, u64, Update),
}

/// The badge on the right of a managed worktree's sidebar row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentBadge {
    /// Managed, with nothing going on.
    Idle,
    Working,
    /// A turn finished that the user hasn't looked at.
    Finished,
}

impl AgentBadge {
    pub(super) fn view<'a>(self, size: u32, p: Palette) -> Svg<'a> {
        match self {
            Self::Idle => icons::agent(size, p.muted()),
            Self::Working => icons::agent(size, p.purple),
            Self::Finished => icons::session_activity(size, p.cyan),
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Idle => "Managed agent",
            Self::Working => "Agent is working",
            Self::Finished => "Agent finished its turn",
        }
    }
}

impl Project {
    pub(super) fn is_managed(&self, path: &Path) -> bool {
        self.managed.contains(path)
    }
}

impl App {
    /// The selected worktree, if it is managed and its folder exists.
    pub(super) fn selected_managed(&self) -> Option<(&Project, &Worktree)> {
        let (project, selection) = self.selected()?;
        let tree = project.worktree(selection.worktree.as_deref()?)?;
        (project.is_managed(&tree.path) && tree.unavailable_reason().is_none())
            .then_some((project, tree))
    }

    fn selected_session(&mut self) -> Option<&mut Session> {
        let path = self.selected_managed()?.1.path.clone();
        self.managed.sessions.get_mut(&path)
    }

    pub(super) fn agent_badge(&self, project: &Project, path: &Path) -> Option<AgentBadge> {
        if !project.is_managed(path) {
            return None;
        }
        Some(match self.managed.sessions.get(path) {
            Some(session) if session.working() => AgentBadge::Working,
            Some(session) if session.unseen => AgentBadge::Finished,
            _ => AgentBadge::Idle,
        })
    }

    /// Whether any agent is in the middle of a turn (quitting would stop it).
    pub(super) fn agents_working(&self) -> bool {
        self.managed.sessions.values().any(Session::working)
    }

    /// Whether an agent in one of `id`'s worktrees is in the middle of a turn
    /// (removing the project would stop it).
    pub(super) fn project_agents_working(&self, id: ProjectId) -> bool {
        self.project_agent_paths(id).iter().any(|path| {
            self.managed
                .sessions
                .get(path)
                .is_some_and(Session::working)
        })
    }

    /// Ends the conversations in `id`'s worktrees, as for [`Self::stop_agents`].
    /// Called when the project is removed: once its rows are gone, nothing
    /// could show or stop those agents.
    pub(super) fn stop_project_agents(&mut self, id: ProjectId) {
        for path in self.project_agent_paths(id) {
            self.managed.sessions.remove(&path);
            self.managed.history.remove(&path);
        }
        self.save_chats();
    }

    /// Brings back the conversations saved before the app last quit, for the
    /// worktrees that are still managed. Each one's messages are read back
    /// from its harness when the worktree is shown.
    pub(super) fn restore_chats(&mut self, saved: BTreeMap<PathBuf, SavedWorktree>) {
        let managed: Vec<PathBuf> = self
            .projects
            .iter()
            .flat_map(|project| project.managed.iter().cloned())
            .collect();
        for (path, saved) in saved {
            if !managed.contains(&path) {
                continue;
            }
            let mut restore = |chat| {
                self.managed.next_session_id += 1;
                Session::restored(self.managed.next_session_id, chat)
            };
            let earlier: Vec<Session> = saved.earlier.into_iter().map(&mut restore).collect();
            if let Some(chat) = saved.current {
                let current = restore(chat);
                self.managed.sessions.insert(path.clone(), current);
            }
            if !earlier.is_empty() {
                self.managed.history.insert(path, earlier);
            }
        }
    }

    /// Saves which conversations each worktree can resume, so they come back
    /// after a restart. Called whenever that changes.
    fn save_chats(&mut self) {
        let Some(store) = &self.managed.store else {
            return;
        };
        let mut worktrees: BTreeMap<PathBuf, SavedWorktree> = BTreeMap::new();
        for (path, session) in &self.managed.sessions {
            worktrees.entry(path.clone()).or_default().current = session.saved();
        }
        for (path, history) in &self.managed.history {
            worktrees.entry(path.clone()).or_default().earlier =
                history.iter().filter_map(Session::saved).collect();
        }
        if let Err(error) = store.save(&worktrees) {
            self.notice = Some(error);
        }
    }

    /// Reads back the selected conversation's earlier messages from its
    /// harness, if it was restored after a restart and they aren't loaded.
    fn load_replay(&mut self) -> Task<Message> {
        let BridgeState::Ready(bridge) = &self.managed.bridge else {
            return Task::none();
        };
        let bridge = bridge.clone();
        let Some(path) = self.selected_managed().map(|(_, tree)| tree.path.clone()) else {
            return Task::none();
        };
        let Some(session) = self
            .managed
            .sessions
            .get_mut(&path)
            .filter(|session| session.replay == Replay::Pending)
        else {
            return Task::none();
        };
        let Some(resume) = session.resume.clone() else {
            session.replay = Replay::Done;
            return Task::none();
        };
        session.replay = Replay::Loading;
        let id = session.id;
        Task::perform(
            agent_bridge::load_history(bridge, path.clone(), session.harness, resume),
            move |result| Message::Managed(ManagedMessage::HistoryLoaded(path.clone(), id, result)),
        )
    }

    /// Puts a restored conversation's earlier messages back above anything
    /// said since. The session may have been set aside under Earlier chats
    /// while they loaded.
    fn history_loaded(&mut self, path: &Path, id: u64, result: Result<Vec<Event>, String>) {
        let current = self.managed.sessions.get_mut(path);
        let session = match current.filter(|session| session.id == id) {
            Some(session) => session,
            None => {
                let earlier = self.managed.history.get_mut(path);
                match earlier.and_then(|history| history.iter_mut().find(|s| s.id == id)) {
                    Some(session) => session,
                    None => return,
                }
            }
        };
        if session.replay != Replay::Loading {
            return;
        }
        session.replay = Replay::Done;
        let mut earlier = Transcript::default();
        match result {
            Ok(events) => events.into_iter().for_each(|event| earlier.apply(event)),
            Err(error) => earlier.push_notice(error),
        }
        session.transcript.prepend(earlier);
    }

    /// The worktrees of project `id` that can have an agent session: the ones
    /// Git lists, and the ones marked managed (in case Git no longer lists them).
    fn project_agent_paths(&self, id: ProjectId) -> Vec<PathBuf> {
        let Some(project) = self.projects.iter().find(|project| project.id == id) else {
            return Vec::new();
        };
        let listed = project.inventory.iter().flatten().map(|tree| &tree.path);
        listed.chain(&project.managed).cloned().collect()
    }

    /// Ends every conversation before the app quits, and returns whether any
    /// bridge was running. Dropping the sessions closes their bridges' input,
    /// which is how a bridge is told to stop its agent and exit.
    pub(super) fn stop_agents(&mut self) -> bool {
        let running = self
            .managed
            .sessions
            .values()
            .any(|session| matches!(session.run, Run::Live { .. }));
        self.managed.sessions.clear();
        running
    }

    /// Shows the selected managed worktree: makes sure it has a session, the
    /// bridge has been checked, and the harness's models are loading.
    pub(super) fn open_managed(&mut self) -> Task<Message> {
        let Some(path) = self.selected_managed().map(|(_, tree)| tree.path.clone()) else {
            return Task::none();
        };
        let next_id = &mut self.managed.next_session_id;
        let session = self.managed.sessions.entry(path).or_insert_with(|| {
            *next_id += 1;
            Session::new(*next_id)
        });
        session.unseen = false;
        let harness = session.harness;
        let load = match self.managed.bridge {
            BridgeState::Unknown | BridgeState::Failed(_) => self.check_bridge(),
            BridgeState::Ready(_) => Task::batch([self.load_models(harness), self.load_replay()]),
            _ => Task::none(),
        };
        Task::batch([load, operation::focus(PROMPT_INPUT)])
    }

    fn check_bridge(&mut self) -> Task<Message> {
        self.managed.bridge = BridgeState::Checking;
        background(agent_bridge::prepare, |result| {
            Message::Managed(ManagedMessage::Prepared(result))
        })
    }

    /// Asks `harness` for its models, unless they are loaded or loading.
    fn load_models(&mut self, harness: Harness) -> Task<Message> {
        let BridgeState::Ready(bridge) = &self.managed.bridge else {
            return Task::none();
        };
        if matches!(
            self.managed.models.get(&harness),
            Some(ModelList::Ready(_) | ModelList::Loading(_))
        ) {
            return Task::none();
        }
        self.managed.next_models_request += 1;
        let request = self.managed.next_models_request;
        self.managed
            .models
            .insert(harness, ModelList::Loading(request));
        Task::perform(
            agent_bridge::list_models(bridge.clone(), harness),
            move |result| Message::Managed(ManagedMessage::ModelsListed(harness, request, result)),
        )
    }

    /// Loads the models of the harness the selected session uses.
    /// Also reads back its earlier messages, if it was restored.
    fn load_selected_models(&mut self) -> Task<Message> {
        match self.selected_session().map(|session| session.harness) {
            Some(harness) => Task::batch([self.load_models(harness), self.load_replay()]),
            None => Task::none(),
        }
    }

    pub(super) fn managed_message(&mut self, message: ManagedMessage) -> Task<Message> {
        match message {
            ManagedMessage::Prepared(result) => {
                if !matches!(self.managed.bridge, BridgeState::Checking) {
                    return Task::none();
                }
                self.managed.bridge = match result {
                    Ok(Prepared::Ready(bridge)) => BridgeState::Ready(bridge),
                    Ok(Prepared::NeedsInstall) => BridgeState::NeedsInstall,
                    Err(error) => BridgeState::Failed(error),
                };
                self.load_selected_models()
            }
            ManagedMessage::Install => match self.managed.bridge {
                BridgeState::NeedsInstall => {
                    self.managed.bridge = BridgeState::Installing;
                    background(agent_bridge::install, |result| {
                        Message::Managed(ManagedMessage::Installed(result))
                    })
                }
                // "Try again" after a failed check or install starts from the check.
                BridgeState::Failed(_) => self.check_bridge(),
                _ => Task::none(),
            },
            ManagedMessage::Installed(result) => {
                if !matches!(self.managed.bridge, BridgeState::Installing) {
                    return Task::none();
                }
                self.managed.bridge = match result {
                    Ok(bridge) => BridgeState::Ready(bridge),
                    Err(error) => BridgeState::Failed(error),
                };
                self.load_selected_models()
            }
            ManagedMessage::ModelsListed(harness, request, result) => {
                if matches!(
                    self.managed.models.get(&harness),
                    Some(ModelList::Loading(current)) if *current == request
                ) {
                    let list = match result {
                        Ok(models) => ModelList::Ready(models),
                        Err(error) => ModelList::Failed(error),
                    };
                    self.managed.models.insert(harness, list);
                }
                // Screenshot builds only: send the scripted prompt once the
                // composer is ready, as if the user had typed it.
                #[cfg(feature = "capture")]
                if let Some(prompt) = crate::capture::prompt()
                    && let Some(session) = self.selected_session()
                    && !session.started()
                {
                    session.editor = text_editor::Content::with_text(&prompt);
                    let sent = self.submit_prompt();
                    if matches!(
                        crate::capture::open(),
                        Some(crate::capture::Open::EarlierChats)
                    ) {
                        let new_chat = self.managed_message(ManagedMessage::NewChat);
                        return Task::batch([sent, new_chat]);
                    }
                    return sent;
                }
                Task::none()
            }
            ManagedMessage::RetryModels => {
                if let Some(harness) = self.selected_session().map(|session| session.harness)
                    && matches!(
                        self.managed.models.get(&harness),
                        Some(ModelList::Failed(_))
                    )
                {
                    self.managed.models.remove(&harness);
                }
                self.load_selected_models()
            }
            ManagedMessage::Harness(harness) => {
                // The harness is fixed once a conversation has started.
                if let Some(session) = self.selected_session()
                    && !session.started()
                    && session.harness != harness
                {
                    session.harness = harness;
                    session.model = None;
                    session.thinking = None;
                }
                self.load_selected_models()
            }
            ManagedMessage::Model(model) => {
                if let Some(session) = self.selected_session()
                    && !session.started()
                {
                    // Thinking levels differ per model, so fall back to its default.
                    session.model = Some(model.id);
                    session.thinking = None;
                }
                Task::none()
            }
            ManagedMessage::Thinking(choice) => {
                if let Some(session) = self.selected_session()
                    && !session.started()
                {
                    session.thinking = match choice {
                        ThinkingChoice::Default(_) => None,
                        ThinkingChoice::Level(level) => Some(level),
                    };
                }
                Task::none()
            }
            ManagedMessage::Edit(action) => {
                if let Some(session) = self.selected_session() {
                    session.editor.perform(action);
                }
                Task::none()
            }
            ManagedMessage::Submit => self.submit_prompt(),
            ManagedMessage::Stop => {
                if let Some(Session {
                    run:
                        Run::Live {
                            commands,
                            working: true,
                            ..
                        },
                    ..
                }) = self.selected_session()
                {
                    // An error means the bridge is gone; its `Exited` update follows.
                    let _ = commands.unbounded_send(Command::Interrupt);
                }
                Task::none()
            }
            ManagedMessage::Unqueue(index) => {
                if let Some(session) = self.selected_session() {
                    session.queue.remove(index);
                }
                Task::none()
            }
            ManagedMessage::NewChat => {
                self.managed.next_session_id += 1;
                let id = self.managed.next_session_id;
                let Some(path) = self.selected_managed().map(|(_, tree)| tree.path.clone()) else {
                    return Task::none();
                };
                let Some(session) = self.managed.sessions.get_mut(&path) else {
                    return Task::none();
                };
                // The same harness and model stay picked.
                let fresh = Session {
                    harness: session.harness,
                    model: session.model.clone(),
                    thinking: session.thinking.clone(),
                    ..Session::new(id)
                };
                // `std::mem::replace` swaps the new session in and hands back
                // the old one, which goes into the worktree's history.
                let old = std::mem::replace(session, fresh);
                self.set_aside(path, old);
                self.save_chats();
                operation::focus(PROMPT_INPUT)
            }
            ManagedMessage::Resume(index) => self.resume_chat(index),
            ManagedMessage::HistoryLoaded(path, id, result) => {
                self.history_loaded(&path, id, result);
                Task::none()
            }
            ManagedMessage::Toggle(index) => {
                if let Some(session) = self.selected_session() {
                    session.transcript.toggle(index);
                }
                Task::none()
            }
            ManagedMessage::Link(url) => background(
                move || actions::open_https(&url),
                |result| Message::Managed(ManagedMessage::LinkOpened(result)),
            ),
            ManagedMessage::LinkOpened(result) => {
                if let Err(error) = result {
                    self.notice = Some(error);
                }
                Task::none()
            }
            ManagedMessage::Bridge(path, id, update) => {
                self.bridge_update(&path, id, update);
                Task::none()
            }
        }
    }

    /// Keeps a conversation that is being replaced in its worktree's history,
    /// with its agent stopped. One that never started has nothing to keep.
    fn set_aside(&mut self, path: PathBuf, session: Session) {
        if session.started() {
            let history = self.managed.history.entry(path).or_default();
            history.push(session.set_aside());
        }
    }

    /// Makes earlier chat `index` the selected worktree's conversation again,
    /// and sets the current one aside. The agent starts with the next prompt,
    /// resuming the harness's saved conversation.
    fn resume_chat(&mut self, index: usize) -> Task<Message> {
        let Some(path) = self.selected_managed().map(|(_, tree)| tree.path.clone()) else {
            return Task::none();
        };
        // `Vec::remove` panics on a bad index, so check it first (a click
        // can't name a chat that isn't listed, but the lint rules are strict).
        let Some(mut earlier) = self
            .managed
            .history
            .get_mut(&path)
            .filter(|history| index < history.len())
            .map(|history| history.remove(index))
        else {
            return Task::none();
        };
        // A new id, so late events from its old bridge stay ignored.
        self.managed.next_session_id += 1;
        earlier.id = self.managed.next_session_id;
        let harness = earlier.harness;
        if let Some(current) = self.managed.sessions.remove(&path) {
            if current.started() {
                self.set_aside(path.clone(), current);
            } else {
                // Text typed into a fresh chat's form isn't lost by resuming.
                let typed = current.editor.text();
                if !typed.trim().is_empty() {
                    let kept = earlier.editor.text();
                    let draft = [kept.trim_end(), typed.trim_end()]
                        .into_iter()
                        .filter(|part| !part.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    earlier.editor = text_editor::Content::with_text(&draft);
                }
            }
        }
        self.managed.sessions.insert(path, earlier);
        self.save_chats();
        Task::batch([
            self.load_models(harness),
            self.load_replay(),
            operation::focus(PROMPT_INPUT),
        ])
    }

    /// Sends the composer's text: straight to an idle agent, into the queue
    /// behind a working one, or as the first prompt of a new bridge.
    fn submit_prompt(&mut self) -> Task<Message> {
        let Some((common_dir, worktree)) = self
            .selected_managed()
            .map(|(project, tree)| (project.checkout.common_dir.clone(), tree.path.clone()))
        else {
            return Task::none();
        };
        let bridge = match &self.managed.bridge {
            BridgeState::Ready(bridge) => bridge.clone(),
            _ => return Task::none(),
        };
        let Some(session) = self.managed.sessions.get_mut(&worktree) else {
            return Task::none();
        };
        let prompt = session.editor.text().trim().to_owned();
        if prompt.is_empty() {
            return Task::none();
        }
        session.editor = text_editor::Content::new();
        match &mut session.run {
            Run::Live { working: true, .. } => {
                session.queue.push_back(prompt);
                Task::none()
            }
            Run::Live {
                commands, working, ..
            } => {
                session.transcript.push_user(prompt.clone());
                *working = true;
                let _ = commands.unbounded_send(Command::Prompt { text: prompt });
                Task::none()
            }
            Run::New | Run::Ended => {
                // With the harness's id, the restarted agent picks the
                // conversation up again; without one it starts afresh.
                if matches!(session.run, Run::Ended) && session.resume.is_none() {
                    session.transcript.push_notice(
                        "The agent was restarted, so it does not remember the conversation above."
                            .into(),
                    );
                }
                let models = match self.managed.models.get(&session.harness) {
                    Some(ModelList::Ready(models)) => Some(models),
                    _ => None,
                };
                let start = session.start_command(models);
                // Keep the resolved model, so the header names what is running.
                if let Command::Start { model, .. } = &start {
                    session.model.clone_from(model);
                }
                let (commands, receiver) = mpsc::unbounded();
                let _ = commands.unbounded_send(start);
                let _ = commands.unbounded_send(Command::Prompt {
                    text: prompt.clone(),
                });
                session.transcript.push_user(prompt);
                let id = session.id;
                let path = worktree.clone();
                // The agent may write to the repository's Git directory too:
                // a linked worktree keeps its index and refs there.
                let stream = agent_bridge::session(bridge, worktree, vec![common_dir], receiver);
                session.run = Run::Live {
                    commands,
                    working: true,
                };
                // `Task::run` feeds each item of the stream back into `update`.
                Task::run(stream, move |update| {
                    Message::Managed(ManagedMessage::Bridge(path.clone(), id, update))
                })
            }
        }
    }

    fn bridge_update(&mut self, path: &Path, id: u64, update: Update) {
        let viewing = self
            .selected_managed()
            .is_some_and(|(_, tree)| tree.path == path);
        let Some(session) = self
            .managed
            .sessions
            .get_mut(path)
            .filter(|session| session.id == id)
        else {
            // From a conversation that was replaced by a new chat. Only its
            // id still matters: it may arrive after the chat was set aside.
            if let Update::Event(Event::Session { id: resume }) = update
                && let Some(earlier) = self
                    .managed
                    .history
                    .get_mut(path)
                    .and_then(|history| history.iter_mut().find(|earlier| earlier.id == id))
            {
                earlier.resume = Some(resume);
                self.save_chats();
            }
            return;
        };
        // A bridge-level error ends the running turn like a failed one. With
        // no turn running there is nothing to end, so it is only noted.
        let update = match update {
            Update::Event(Event::Error { message }) if session.working() => {
                Update::Event(Event::TurnEnd {
                    status: TurnStatus::Failed,
                    error: Some(message),
                })
            }
            Update::Event(Event::Error { message }) => {
                Update::Event(Event::Notice { text: message })
            }
            other => other,
        };
        match update {
            // A turn ends once. A second end for the same turn (a harness
            // that reports an error and then the failed turn) must not pop
            // another queued follow-up while the first one is running.
            Update::Event(Event::TurnEnd { .. }) if !session.working() => {}
            Update::Event(Event::TurnEnd { status, error }) => {
                session.transcript.end_turn(status, error);
                session.unseen |= !viewing;
                let Run::Live {
                    commands, working, ..
                } = &mut session.run
                else {
                    return;
                };
                // The turn is over: the oldest queued follow-up goes in next.
                match session.queue.pop_front() {
                    Some(next) => {
                        session.transcript.push_user(next.clone());
                        let _ = commands.unbounded_send(Command::Prompt { text: next });
                    }
                    None => *working = false,
                }
            }
            Update::Event(Event::Session { id: resume }) => {
                if session.resume.as_ref() != Some(&resume) {
                    session.resume = Some(resume);
                    self.save_chats();
                }
            }
            Update::Event(event) => session.transcript.apply(event),
            Update::Exited(reason) => {
                if session.working() {
                    session
                        .transcript
                        .end_turn(TurnStatus::Failed, Some(reason));
                } else {
                    session.transcript.push_notice(reason);
                }
                session.unseen |= !viewing;
                session.run = Run::Ended;
                // Hand unsent follow-ups back rather than dropping them.
                session.return_queue();
            }
        }
    }

    /// The header control and the pane for a managed worktree.
    pub(super) fn agent_pane<'a>(
        &'a self,
        tree: &'a Worktree,
    ) -> (Element<'a, Message>, Element<'a, Message>) {
        let p = self.palette();
        let Some(session) = self.managed.sessions.get(&tree.path) else {
            // `open_managed` creates the session as the worktree is selected.
            return (Space::new().into(), Space::new().into());
        };
        let body = if session.started() {
            self.conversation(session, tree)
        } else {
            container(self.first_prompt(session, &tree.path))
                .center(Fill)
                .padding(12)
                .into()
        };
        (self.agent_status(session, p), body)
    }

    /// The models to offer for `harness`, once loaded.
    fn models(&self, harness: Harness) -> Option<&Models> {
        match self.managed.models.get(&harness) {
            Some(ModelList::Ready(models)) => Some(models),
            _ => None,
        }
    }

    /// The model the session will use: the one picked, else the harness default.
    fn chosen_model(&self, session: &Session) -> Option<&Model> {
        let models = self.models(session.harness)?;
        match &session.model {
            Some(id) => models.get(id),
            None => models.preferred(),
        }
    }

    /// "Claude · Opus 5.5 · high" plus Stop / New Chat, for the worktree header.
    fn agent_status<'a>(&'a self, session: &'a Session, p: Palette) -> Element<'a, Message> {
        if !session.started() {
            return row![
                icons::agent(13, p.muted()),
                text("Managed").size(11).color(p.muted())
            ]
            .spacing(3)
            .align_y(Center)
            .into();
        }
        let mut label = session.harness.name().to_owned();
        let model = self
            .chosen_model(session)
            .map(|model| model.name.as_str())
            .or(session.model.as_deref());
        for part in [model, session.thinking.as_deref()].into_iter().flatten() {
            label.push_str(" · ");
            label.push_str(part);
        }
        let small = |label: &'static str, message| {
            button(text(label).size(11))
                .padding([2, 5])
                .style(style::button(p, Button::Secondary))
                .on_press(Message::Managed(message))
        };
        let mut status = row![
            harness_icon(session.harness, 13, p.purple),
            text(label)
                .size(11)
                .color(p.muted())
                .wrapping(text::Wrapping::None),
        ]
        .spacing(4)
        .align_y(Center);
        if session.working() {
            status = status.push(small("Stop", ManagedMessage::Stop));
        }
        status
            .push(small("New Chat", ManagedMessage::NewChat))
            .into()
    }

    /// The centered form a new managed worktree opens with.
    fn first_prompt<'a>(&'a self, session: &'a Session, path: &Path) -> Element<'a, Message> {
        let p = self.palette();
        let note = |message: &'a str| text(message).size(13).color(p.muted());
        let action = |label: &'static str| {
            button(text(label).size(13))
                .style(style::button(p, Button::Primary))
                .on_press(Message::Managed(ManagedMessage::Install))
        };
        let mut form = column![text("What should the agent work on?").size(18)]
            .spacing(10)
            .max_width(640);
        form = match &self.managed.bridge {
            BridgeState::Unknown | BridgeState::Checking => {
                form.push(note("Checking the agent SDKs…"))
            }
            BridgeState::NeedsInstall => form
                .push(note(
                    "Managed worktrees drive Claude, Codex and Pi through their SDKs. Installing them downloads about 1 GB into Application Support, once.",
                ))
                .push(action("Install Agent SDKs")),
            BridgeState::Installing => form.push(note(
                "Installing the agent SDKs. This can take a few minutes…",
            )),
            BridgeState::Failed(error) => form
                .push(text(error).size(13).color(p.red))
                .push(action("Try Again")),
            BridgeState::Ready(_) => {
                let harnesses = Harness::ALL.map(|harness| {
                    let chosen = harness == session.harness;
                    button(
                        row![
                            harness_icon(harness, 14, if chosen { p.purple } else { p.muted() }),
                            text(harness.name()).size(13)
                        ]
                        .spacing(6)
                        .align_y(Center),
                    )
                    .padding([5, 10])
                    .style(style::button(
                        p,
                        if chosen {
                            Button::Selected
                        } else {
                            Button::Secondary
                        },
                    ))
                    .on_press(Message::Managed(ManagedMessage::Harness(harness)))
                    .into()
                });
                form.push(row(harnesses).spacing(6))
                    .push(self.model_pickers(session, p))
                    .push(self.composer(session, "Describe a task…", 96.0))
                    .push(self.earlier_chats(path, p))
            }
        };
        form.into()
    }

    /// The worktree's earlier chats, newest first, each a button that resumes it.
    fn earlier_chats<'a>(&'a self, path: &Path, p: Palette) -> Element<'a, Message> {
        let Some(history) = self
            .managed
            .history
            .get(path)
            .filter(|history| !history.is_empty())
        else {
            return Space::new().into();
        };
        let mut list = column![text("Earlier chats").size(12).color(p.muted())].spacing(4);
        for (index, earlier) in history.iter().enumerate().rev() {
            let model = self
                .models(earlier.harness)
                .and_then(|models| models.get(earlier.model.as_deref()?))
                .map(|model| model.name.as_str())
                .or(earlier.model.as_deref())
                .unwrap_or(earlier.harness.name());
            list = list.push(
                button(
                    row![
                        harness_icon(earlier.harness, 13, p.purple),
                        single_line(earlier.title().replace('\n', " "), 12),
                        text(model)
                            .size(11)
                            .color(p.muted())
                            .wrapping(text::Wrapping::None),
                    ]
                    .spacing(6)
                    .align_y(Center),
                )
                .width(Fill)
                .padding([4, 8])
                .style(style::button(p, Button::Secondary))
                .on_press(Message::Managed(ManagedMessage::Resume(index))),
            );
        }
        list.into()
    }

    /// The model and thinking drop-downs for the session's harness.
    fn model_pickers<'a>(&'a self, session: &'a Session, p: Palette) -> Element<'a, Message> {
        let models = match self.managed.models.get(&session.harness) {
            Some(ModelList::Ready(models)) => models,
            Some(ModelList::Failed(error)) => {
                return row![
                    text(error).size(12).color(p.red).width(Fill),
                    button(text("Retry").size(12))
                        .style(style::button(p, Button::Secondary))
                        .on_press(Message::Managed(ManagedMessage::RetryModels)),
                ]
                .spacing(8)
                .align_y(Center)
                .into();
            }
            _ => {
                return text(format!("Loading {} models…", session.harness.name()))
                    .size(12)
                    .color(p.muted())
                    .into();
            }
        };
        let chosen = self.chosen_model(session);
        let levels = chosen
            .map(|model| model.thinking.as_slice())
            .unwrap_or_default();
        let default =
            ThinkingChoice::Default(chosen.and_then(|model| model.default_thinking.clone()));
        let thinking: Vec<_> = std::iter::once(default.clone())
            .chain(levels.iter().cloned().map(ThinkingChoice::Level))
            .collect();
        let selected = session
            .thinking
            .clone()
            .map_or(default, ThinkingChoice::Level);
        let label = |label: &'static str| text(label).size(12).color(p.muted());
        row![
            label("Model"),
            pick_list(models.models.as_slice(), chosen.cloned(), |model| {
                Message::Managed(ManagedMessage::Model(model))
            })
            .placeholder("No models available")
            .text_size(12)
            .padding([4, 7])
            .style(style::action_picker(p))
            .menu_style(style::action_menu(p)),
            Space::new().width(6),
            label("Thinking"),
            pick_list(thinking, Some(selected), |choice| {
                Message::Managed(ManagedMessage::Thinking(choice))
            })
            .text_size(12)
            .padding([4, 7])
            .style(style::action_picker(p))
            .menu_style(style::action_menu(p)),
        ]
        .spacing(6)
        .align_y(Center)
        .into()
    }

    /// The prompt editor and its Send button. Return sends; Shift+Return
    /// starts a new line.
    fn composer<'a>(
        &'a self,
        session: &'a Session,
        placeholder: &'a str,
        min_height: f32,
    ) -> Element<'a, Message> {
        let p = self.palette();
        let empty = session.editor.text().trim().is_empty();
        let editor = text_editor(&session.editor)
            .id(PROMPT_INPUT)
            .placeholder(placeholder)
            .size(13)
            .padding(8)
            .min_height(min_height)
            .max_height(220.0)
            .on_action(|action| Message::Managed(ManagedMessage::Edit(action)))
            .key_binding(|press| {
                let sends = matches!(press.key, Key::Named(Named::Enter))
                    && !press.modifiers.shift()
                    && matches!(press.status, text_editor::Status::Focused { .. });
                if sends {
                    Some(text_editor::Binding::Custom(Message::Managed(
                        ManagedMessage::Submit,
                    )))
                } else {
                    text_editor::Binding::from_key_press(press)
                }
            })
            .style(style::editor(p));
        let send = button(text(if session.working() { "Queue" } else { "Send" }).size(13))
            .padding([6, 12])
            .style(style::button(p, Button::Primary))
            .on_press_maybe((!empty).then_some(Message::Managed(ManagedMessage::Submit)));
        column![
            editor,
            row![
                text("Return to send  ·  Shift+Return for a new line")
                    .size(11)
                    .color(p.muted())
                    .width(Fill),
                send,
            ]
            .spacing(8)
            .align_y(Center),
        ]
        .spacing(6)
        .into()
    }

    /// The transcript, the queued follow-ups and the follow-up composer.
    fn conversation<'a>(
        &'a self,
        session: &'a Session,
        tree: &'a Worktree,
    ) -> Element<'a, Message> {
        let p = self.palette();
        let working = session.working();
        let items = session.transcript.items();
        let mut list = column![].spacing(8).width(Fill);
        // A conversation restored after a restart, before its messages are back.
        if session.replay != Replay::Done {
            list = list.push(
                row![
                    icons::connecting(12, p.purple),
                    text(format!("Loading “{}”…", session.title()))
                        .size(12)
                        .color(p.muted())
                ]
                .spacing(5)
                .align_y(Center),
            );
        }
        for (index, item) in items.iter().enumerate() {
            // Only the newest row can still be in progress.
            let active = working && index + 1 == items.len();
            list = list.push(self.item(index, item, active, &tree.path, p));
        }
        if working {
            list = list.push(
                row![
                    icons::connecting(12, p.purple),
                    text("Working…").size(12).color(p.muted())
                ]
                .spacing(5)
                .align_y(Center),
            );
        }
        let mut bottom = column![].spacing(4);
        for (index, queued) in session.queue.iter().enumerate() {
            bottom = bottom.push(
                container(
                    row![
                        text("Queued").size(11).color(p.muted()),
                        single_line(queued.replace('\n', " "), 12),
                        button(icons::close(11, p.muted()))
                            .padding(3)
                            .style(style::button(p, Button::Nav))
                            .on_press(Message::Managed(ManagedMessage::Unqueue(index))),
                    ]
                    .spacing(8)
                    .align_y(Center),
                )
                .padding([3, 8])
                .style(style::panel(p, Panel::Glass)),
            );
        }
        let placeholder = if working {
            "Queue a follow-up for when this turn ends…"
        } else {
            "Send a follow-up…"
        };
        column![
            // Stays scrolled to the newest row while the agent streams.
            scrollable(container(list).padding(iced::padding::right(12)))
                .anchor_bottom()
                .width(Fill)
                .height(Fill),
            bottom.push(self.composer(session, placeholder, 34.0)),
        ]
        .spacing(8)
        .padding(10)
        .into()
    }

    fn item<'a>(
        &'a self,
        index: usize,
        item: &'a Item,
        active: bool,
        worktree: &Path,
        p: Palette,
    ) -> Element<'a, Message> {
        // A collapsed row: a caret, an optional status icon and one line of text.
        let fold = |open: bool, icon: Option<Svg<'a>>, label: String, detail: Option<String>| {
            // Rows with nothing to expand keep the caret's space, so labels line up.
            let caret: Element<'a, Message> = match (&detail, open) {
                (None, _) => Space::new().width(11).into(),
                (Some(_), true) => icons::chevron_down(11, p.muted()).into(),
                (Some(_), false) => icons::chevron_right(11, p.muted()).into(),
            };
            let mut line = row![caret].spacing(5).align_y(Center);
            if let Some(icon) = icon {
                line = line.push(icon);
            }
            line = line.push(
                container(
                    text(label)
                        .size(12)
                        .color(p.muted())
                        .wrapping(text::Wrapping::None),
                )
                .clip(true)
                .width(Fill),
            );
            let header = button(line)
                .width(Fill)
                .padding([2, 4])
                .style(style::button(p, Button::Nav))
                .on_press_maybe(
                    detail
                        .is_some()
                        .then_some(Message::Managed(ManagedMessage::Toggle(index))),
                );
            let mut block = column![header].spacing(2);
            if let (true, Some(detail)) = (open, detail) {
                block = block.push(
                    container(text(detail).size(11).color(p.muted()))
                        .padding([5, 8])
                        .width(Fill)
                        .style(style::panel(p, Panel::Glass)),
                );
            }
            Element::from(block)
        };
        match item {
            Item::User(prompt) => container(text(prompt).size(13))
                .padding([6, 9])
                .width(Fill)
                .style(style::panel(p, Panel::Glass))
                .into(),
            Item::Text { content, .. } => {
                let font = self.terminal_preferences.font.font_type;
                let settings =
                    markdown::Settings::with_text_size(13, style::markdown_style(p, font));
                container(
                    markdown::view(content.items(), settings)
                        .map(|url| Message::Managed(ManagedMessage::Link(url))),
                )
                .padding([0, 4])
                .into()
            }
            Item::Thinking { text, open, .. } => {
                let summary = agent::thinking_summary(text);
                let label = match (active, summary) {
                    (true, Some(summary)) => format!("Thinking…  {summary}"),
                    (true, None) => "Thinking…".into(),
                    (false, Some(summary)) => format!("Thought  ·  {summary}"),
                    (false, None) => "Thought".into(),
                };
                let detail = (!text.trim().is_empty()).then(|| text.trim().to_owned());
                fold(*open, None, label, detail)
            }
            Item::Tool {
                name,
                input,
                status,
                output,
                open,
                ..
            } => {
                let icon = match status {
                    ToolStatus::Running => icons::connecting(11, p.purple),
                    ToolStatus::Done => icons::check(11, p.green),
                    ToolStatus::Failed => icons::warning(11, p.red),
                };
                let detail = agent::tool_detail(input, output);
                fold(
                    *open,
                    Some(icon),
                    agent::tool_summary(name, input, worktree),
                    (!detail.is_empty()).then_some(detail),
                )
            }
            Item::Notice(notice) => row![
                icons::warning(12, p.yellow),
                text(notice).size(12).color(p.yellow)
            ]
            .spacing(6)
            .padding([0, 4])
            .into(),
        }
    }
}

fn harness_icon<'a>(harness: Harness, size: u32, color: iced::Color) -> Svg<'a> {
    match harness {
        Harness::Claude => icons::claude(size, color),
        Harness::Codex => icons::codex(size, color),
        Harness::Pi => icons::pi(size, color),
    }
}
