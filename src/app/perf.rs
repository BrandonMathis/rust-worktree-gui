//! Hooks for the keystroke-latency harness (`crate::perf`), compiled only with
//! `--features perf`. They put the app into the state a user would reach by
//! clicking, through the same handlers, without a window or tmux.
//!
//! They live in the `app` module because `App`'s fields are private to it.

use super::terminal_pane::ConsoleState;
use super::{App, MenuAction, Message};
use crate::git::{Checkout, Worktree};
use iced::Task;
use iced::futures::StreamExt;
use iced::futures::stream::BoxStream;
use iced_term::BackendCommand;
use iced_term::settings::BackendSettings;
use std::path::PathBuf;

impl App {
    /// The workspace with `worktree` selected, as after a click on its sidebar
    /// row, at a window `scale` (2 on a Retina display). No terminal yet.
    pub(crate) fn perf_workspace(
        checkout: Checkout,
        inventory: Vec<Worktree>,
        worktree: PathBuf,
        scale: f32,
    ) -> Self {
        // No bookmark store (as in the app tests), so nothing is saved; drop
        // the notice that reports the missing store.
        let (mut app, _) = Self::from_bookmarks(Err(String::new()));
        app.notice = None;
        let id = app.insert_project(checkout.into());
        if let Some(project) = app.projects.iter_mut().find(|p| p.id == id) {
            project.expanded = true;
            project.inventory = Some(inventory);
        }
        app.select(id, Some(worktree));
        app.window_scale = scale;
        app
    }

    /// Starts the terminal with `backend` as a finished tmux `prepare` does
    /// (`Message::Prepared`). Returns the task that focuses it.
    pub(crate) fn perf_attach(&mut self, backend: BackendSettings) -> Task<Message> {
        self.next_console_id += 1;
        let id = self.next_console_id;
        self.console = ConsoleState::Connecting(id);
        self.update(Message::Prepared(id, Ok(backend)))
    }

    /// The terminal's output events: the stream behind its iced subscription.
    pub(crate) fn perf_terminal_events(&self) -> Option<BoxStream<'static, iced_term::Event>> {
        let console = self.console.console()?;
        let recipe = iced::advanced::subscription::into_recipes(console.subscription()).pop()?;
        Some(recipe.stream(iced::futures::stream::empty().boxed()))
    }

    /// Sends `bytes` to the terminal's program, as typing them would.
    pub(crate) fn perf_write_terminal(&mut self, bytes: Vec<u8>) -> Task<Message> {
        let Some(id) = self.console.console().map(|console| console.id()) else {
            return Task::none();
        };
        self.update(Message::Terminal(iced_term::Event::BackendCall(
            id,
            BackendCommand::Write(bytes),
        )))
    }

    /// Opens the selected worktree's Rename field, as its ⋮ menu does.
    pub(crate) fn perf_open_rename(&mut self) -> Task<Message> {
        match self.selection.clone() {
            Some(target) => self.update(Message::MenuAction(target, MenuAction::RenameWorktree)),
            None => Task::none(),
        }
    }

    pub(crate) fn perf_clear_rename(&mut self) -> Task<Message> {
        self.update(Message::EditWorktreeName(String::new()))
    }
}
