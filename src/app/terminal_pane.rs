//! The embedded terminal: connecting it to a worktree's tmux session, and its
//! status in the worktree header.

use super::view::placeholder;
use super::{App, Message, background};
use crate::git::Worktree;
use crate::icons;
use crate::style::{self, Button, Panel};
use crate::terminal::Console;
use crate::tmux;
use iced::widget::{button, column, container, row, text, tooltip};
use iced::{Center, Element, Fill, Task};

/// The terminal pane's connection. `Connecting` carries the attempt's id:
/// a `Prepared` result for any other id is stale and ignored.
#[derive(Default)]
pub(super) enum ConsoleState {
    #[default]
    Detached,
    Connecting(u64),
    Attached(Box<Console>),
    Failed(String),
}

impl App {
    /// Connects the terminal to the selected worktree's tmux session, or
    /// focuses it if it is already attached. Does nothing while connecting,
    /// or for a project overview or an unavailable worktree.
    pub(super) fn connect(&mut self) -> Task<Message> {
        if let ConsoleState::Attached(console) = &self.console
            && console.is_open()
        {
            return console.focus().map(Message::Terminal);
        }
        if matches!(self.console, ConsoleState::Connecting(_)) {
            return Task::none();
        }
        let Some((project, selection)) = self.selected() else {
            return Task::none();
        };
        let Some(tree) = selection
            .worktree
            .as_deref()
            .and_then(|path| project.worktree(path))
        else {
            return Task::none();
        };
        if tree.unavailable_reason().is_some() {
            return Task::none();
        }
        let worktree = tree.path.clone();
        let common_dir = project.checkout.common_dir.clone();
        self.next_console_id += 1;
        let id = self.next_console_id;
        self.console = ConsoleState::Connecting(id);
        background(
            move || tmux::prepare(&common_dir, &worktree),
            move |result| Message::Prepared(id, result),
        )
    }

    pub(super) fn prepared(
        &mut self,
        id: u64,
        result: Result<iced_term::settings::BackendSettings, String>,
    ) -> Task<Message> {
        if !matches!(self.console, ConsoleState::Connecting(current) if current == id) {
            return Task::none();
        }
        match result.and_then(|backend| {
            Console::open(id, backend, &self.terminal_preferences, self.window_scale)
        }) {
            Ok(console) => {
                self.console = ConsoleState::Attached(Box::new(console));
                if let Some((project, selection)) = self.selected()
                    && let Some(path) = selection.worktree.clone()
                {
                    let id = project.id;
                    self.mark_session_seen(id, &path);
                }
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
            if let ConsoleState::Attached(console) = &mut self.console {
                console.rescale(&self.terminal_preferences, scale);
            }
        }
    }

    pub(super) fn terminal_event(&mut self, event: iced_term::Event) {
        if let ConsoleState::Attached(console) = &mut self.console {
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
                    && self.creation.is_none() =>
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
                    let mut content = column![console.view().map(Message::Terminal)]
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
