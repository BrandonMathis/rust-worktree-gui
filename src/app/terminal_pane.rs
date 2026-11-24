//! The embedded terminal: connecting it to a worktree's tmux session, and its
//! status in the worktree header.
//!
//! Switching between worktrees reuses the running terminal: its tmux client is
//! told to `switch-client` to the other worktree's session, which is much
//! faster than starting a new terminal and client. Any failure falls back to a
//! fresh connection.

use super::view::placeholder;
use super::{App, Message, Project, background};
use crate::git::Worktree;
use crate::icons;
use crate::style::{self, Button, Corners, Panel};
use crate::terminal::Console;
use crate::tmux;
use iced::widget::{button, column, container, row, text, tooltip};
use iced::{Center, Element, Fill, Task};
use std::path::{Path, PathBuf};

/// The terminal pane's connection. `Connecting` and `Switching` carry the
/// attempt's id: a `Prepared` or `Switched` result for any other id is stale
/// and ignored.
#[derive(Default)]
pub(super) enum ConsoleState {
    #[default]
    Detached,
    Connecting(u64),
    Attached(Box<Console>),
    /// The running terminal's tmux client is being switched to `target`'s
    /// session. The terminal stays on screen (and keeps processing output)
    /// until the switch finishes.
    Switching {
        id: u64,
        console: Box<Console>,
        target: PathBuf,
    },
    Failed(String),
}

impl ConsoleState {
    /// The live terminal widget, attached or mid-switch.
    pub(super) fn console(&self) -> Option<&Console> {
        match self {
            Self::Attached(console) | Self::Switching { console, .. } => Some(console),
            _ => None,
        }
    }

    fn console_mut(&mut self) -> Option<&mut Console> {
        match self {
            Self::Attached(console) | Self::Switching { console, .. } => Some(console),
            _ => None,
        }
    }
}

impl App {
    /// The selected worktree, if it is one that can have a terminal.
    fn selected_worktree(&self) -> Option<(&Project, &Worktree)> {
        let (project, selection) = self.selected()?;
        let tree = project.worktree(selection.worktree.as_deref()?)?;
        tree.unavailable_reason()
            .is_none()
            .then_some((project, tree))
    }

    /// Called when the selection changes. A running terminal is kept when the
    /// new selection is another available worktree, so `connect` can switch
    /// its tmux client there. Otherwise the terminal detaches: dropping it ends
    /// its tmux client, never the session.
    pub(super) fn keep_or_detach_terminal(&mut self) {
        let reusable = self.selected_worktree().is_some();
        // `mem::take` moves the state out (leaving `Detached`), so the boxed
        // terminal can be moved back in without cloning it.
        match std::mem::take(&mut self.console) {
            state @ (ConsoleState::Attached(_) | ConsoleState::Switching { .. })
                if reusable && state.console().is_some_and(Console::is_open) =>
            {
                self.console = state;
            }
            _ => {}
        }
    }

    /// Connects the terminal to the selected worktree's tmux session. If a
    /// terminal is already running, it focuses it (same worktree) or switches
    /// its client (another worktree). Does nothing while connecting or
    /// switching, or for a project overview or an unavailable worktree.
    pub(super) fn connect(&mut self) -> Task<Message> {
        if matches!(
            self.console,
            ConsoleState::Connecting(_) | ConsoleState::Switching { .. }
        ) {
            return Task::none();
        }
        let Some((common_dir, worktree)) = self
            .selected_worktree()
            .map(|(project, tree)| (project.checkout.common_dir.clone(), tree.path.clone()))
        else {
            return Task::none();
        };
        if let ConsoleState::Attached(console) = &self.console
            && console.is_open()
        {
            if console.worktree() == worktree {
                return console.focus().map(Message::Terminal);
            }
            if let ConsoleState::Attached(console) = std::mem::take(&mut self.console) {
                return self.start_switch(console, common_dir, worktree);
            }
        }
        self.next_console_id += 1;
        let id = self.next_console_id;
        self.console = ConsoleState::Connecting(id);
        background(
            move || tmux::prepare(&common_dir, &worktree),
            move |result| Message::Prepared(id, result),
        )
    }

    /// Asks tmux, in the background, to move the terminal's client to
    /// `target`'s session.
    fn start_switch(
        &mut self,
        console: Box<Console>,
        common_dir: PathBuf,
        target: PathBuf,
    ) -> Task<Message> {
        self.next_console_id += 1;
        let id = self.next_console_id;
        let client = console.client_pid();
        self.console = ConsoleState::Switching {
            id,
            console,
            target: target.clone(),
        };
        background(
            move || tmux::switch_client(client, &common_dir, &target),
            move |result| Message::Switched(id, result),
        )
    }

    pub(super) fn switched(&mut self, id: u64, result: Result<(), String>) -> Task<Message> {
        if !matches!(self.console, ConsoleState::Switching { id: current, .. } if current == id) {
            return Task::none();
        }
        let ConsoleState::Switching {
            mut console,
            target,
            ..
        } = std::mem::take(&mut self.console)
        else {
            return Task::none();
        };
        if result.is_err() {
            // Start a fresh terminal instead. `console` is dropped here, which
            // ends its tmux client; the sessions keep running.
            return self.connect();
        }
        console.switched_to(target.clone());
        self.console = ConsoleState::Attached(console);
        match self.selected_worktree() {
            Some((project, tree)) if tree.path == target => {
                let id = project.id;
                self.mark_session_seen(id, &target);
                self.focus_terminal()
            }
            // The user picked another worktree while this switch ran: follow.
            _ => self.connect(),
        }
    }

    pub(super) fn prepared(
        &mut self,
        id: u64,
        result: Result<iced_term::settings::BackendSettings, String>,
    ) -> Task<Message> {
        if !matches!(self.console, ConsoleState::Connecting(current) if current == id) {
            return Task::none();
        }
        let Some((project_id, worktree)) = self
            .selection
            .as_ref()
            .and_then(|s| Some((s.project, s.worktree.clone()?)))
        else {
            self.console = ConsoleState::Detached;
            return Task::none();
        };
        match result.and_then(|backend| {
            Console::open(
                id,
                worktree.clone(),
                backend,
                &self.terminal_preferences,
                self.window_scale,
            )
        }) {
            Ok(console) => {
                self.console = ConsoleState::Attached(Box::new(console));
                self.mark_session_seen(project_id, &worktree);
                self.focus_terminal()
            }
            Err(error) => {
                self.console = ConsoleState::Failed(error);
                Task::none()
            }
        }
    }

    /// Drops the terminal view. The tmux session (and its shell) keeps running.
    pub(super) fn detach(&mut self) {
        self.leave_session();
        self.console = ConsoleState::Detached;
    }

    pub(super) fn window_rescaled(&mut self, scale: f32) {
        if self.window_scale != scale {
            self.window_scale = scale;
            if let Some(console) = self.console.console_mut() {
                console.rescale(&self.terminal_preferences, scale);
            }
        }
    }

    pub(super) fn paste_dropped_path(&mut self, path: &Path) {
        // File drops arrive as window events. Send only to a settled attachment,
        // never to a hidden terminal or a client that might still be switching.
        if !self.picking
            && self.pending_confirmation.is_none()
            && self.settings.is_none()
            && self.creation.is_none()
            && self.worktree_name_draft.is_none()
            && let ConsoleState::Attached(console) = &mut self.console
        {
            console.paste_path(path);
        }
    }

    pub(super) fn terminal_event(&mut self, event: iced_term::Event) {
        if let Some(console) = self.console.console_mut() {
            console.update(event);
        }
    }

    /// Gives keyboard focus back to the terminal, unless a text field (rename
    /// or the creation form) should keep it.
    pub(super) fn focus_terminal(&self) -> Task<Message> {
        match &self.console {
            ConsoleState::Attached(console)
                if console.is_open()
                    && self.worktree_name_draft.is_none()
                    && self.creation.is_none()
                    && self.unavailable_list.is_none() =>
            {
                console.focus().map(Message::Terminal)
            }
            _ => Task::none(),
        }
    }

    /// The terminal's status control for the worktree header, and the pane
    /// below it (the terminal itself, or a placeholder explaining its state).
    pub(super) fn terminal_pane<'a>(
        &'a self,
        tree: &'a Worktree,
        corners: Corners,
    ) -> (Element<'a, Message>, Element<'a, Message>) {
        let p = self.palette();
        let (badge, badge_color, action, body) = if let Some(reason) = tree.unavailable_reason() {
            let detail = tree
                .prunable
                .as_ref()
                .filter(|detail| !detail.is_empty())
                .map(String::as_str);
            ("Unavailable", p.red, None, placeholder(reason, detail, p))
        } else {
            match &self.console {
                ConsoleState::Attached(console) => {
                    let failure = console.failure();
                    let open = console.is_open();
                    // The terminal takes the pane's rounded corners unless an
                    // error line sits below it, in the pane's bottom corners.
                    let corners = if failure.is_some() {
                        Corners::NONE
                    } else {
                        corners
                    };
                    let mut content =
                        column![console.view(corners.radius()).map(Message::Terminal)]
                            .height(Fill)
                            .spacing(6);
                    if let Some(error) = failure {
                        content = content.push(text(error).size(13).color(p.red));
                    }
                    (
                        if open { "Attached" } else { "Detached" },
                        if open { p.green } else { p.muted() },
                        Some(if open {
                            ("Detach", Message::Detach)
                        } else {
                            ("Reconnect", Message::Reconnect)
                        }),
                        content.into(),
                    )
                }
                // Same wrapper as `Attached`: iced keeps widget state by tree
                // position, so the terminal keeps its focus and scroll state
                // through the switch.
                ConsoleState::Switching { console, .. } => (
                    "Connecting…",
                    p.muted(),
                    Some(("Cancel", Message::Detach)),
                    column![console.view(corners.radius()).map(Message::Terminal)]
                        .height(Fill)
                        .spacing(6)
                        .into(),
                ),
                ConsoleState::Failed(error) => (
                    "Could not connect",
                    p.red,
                    Some(("Retry", Message::Reconnect)),
                    placeholder("Terminal not started", Some(error), p),
                ),
                ConsoleState::Connecting(_) => (
                    "Connecting…",
                    p.muted(),
                    Some(("Cancel", Message::Detach)),
                    placeholder(
                        "Opening tmux",
                        Some("Connecting to this worktree’s session."),
                        p,
                    ),
                ),
                ConsoleState::Detached => (
                    "Detached",
                    p.muted(),
                    Some(("Connect", Message::Reconnect)),
                    placeholder(
                        "No terminal attached",
                        Some("Connect to start or resume the tmux session for this worktree."),
                        p,
                    ),
                ),
            }
        };
        let status = match badge {
            "Attached" => icons::connected(13, badge_color),
            "Connecting…" => icons::connecting(13, p.purple),
            "Detached" => icons::empty(13, badge_color),
            _ => icons::warning(13, badge_color),
        };
        let terminal_status: Element<'_, Message> = if let Some((label, message)) = action {
            tooltip(
                button(
                    row![status, text(label).size(11)]
                        .spacing(3)
                        .align_y(Center),
                )
                .padding([2, 5])
                .style(style::button(p, Button::Secondary))
                .on_press(message),
                container(text(format!("Terminal: {badge}")).size(11))
                    .padding([3, 5])
                    .style(style::panel(p, Panel::Console)),
                tooltip::Position::Bottom,
            )
            .delay(std::time::Duration::from_millis(500))
            .into()
        } else {
            row![status, text(badge).size(11).color(badge_color)]
                .spacing(3)
                .align_y(Center)
                .into()
        };
        (terminal_status, body)
    }
}
