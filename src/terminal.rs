use crate::terminal_config::Preferences;
use iced::{Element, Subscription, Task};
use iced_term::settings::{BackendSettings, Settings, ThemeSettings};
use iced_term::{AlacrittyEvent, BackendCommand, Command, Event, Terminal, TerminalView};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

pub struct Console {
    widget: Terminal,
    /// The worktree whose tmux session the client in this terminal shows.
    worktree: PathBuf,
    exited: bool,
    /// How the tmux client exited, once alacritty_terminal reports it.
    status: Option<ExitStatus>,
    scale: f32,
}

impl Console {
    pub fn open(
        id: u64,
        worktree: PathBuf,
        backend: BackendSettings,
        preferences: &Preferences,
        scale: f32,
    ) -> Result<Self, String> {
        let mut widget = Terminal::new(
            id,
            Settings {
                backend,
                font: preferences.font_at_scale(scale),
                theme: ThemeSettings::new(Box::new(preferences.palette.clone())),
            },
        )
        .map_err(|error| format!("Could not start the embedded terminal: {error}"))?;
        if !preferences.bindings.is_empty() {
            widget.handle(Command::AddBindings(preferences.bindings.clone()));
        }
        Ok(Self {
            widget,
            worktree,
            exited: false,
            status: None,
            scale,
        })
    }

    pub fn worktree(&self) -> &Path {
        &self.worktree
    }

    /// Records that the tmux client now shows `worktree`'s session.
    pub fn switched_to(&mut self, worktree: PathBuf) {
        self.worktree = worktree;
    }

    /// The process id of the tmux client running in this terminal.
    pub fn client_pid(&self) -> u32 {
        self.widget.child_pid()
    }

    /// Identifies this terminal instance; a reconnect creates a new id.
    #[cfg(test)]
    pub fn id(&self) -> u64 {
        self.widget.id
    }

    pub fn rescale(&mut self, preferences: &Preferences, scale: f32) {
        self.scale = scale;
        self.widget
            .handle(Command::ChangeFont(preferences.font_at_scale(scale)));
    }

    pub fn focus(&self) -> Task<Event> {
        TerminalView::focus(self.widget.widget_id().clone())
    }

    pub fn subscription(&self) -> Subscription<Event> {
        if self.exited {
            Subscription::none()
        } else {
            self.widget.subscription()
        }
    }

    pub fn view(&self) -> Element<'_, Event> {
        // Embed edge-to-edge: standalone Alacritty window padding does not apply.
        TerminalView::show_at_scale(&self.widget, self.scale)
    }

    pub fn is_open(&self) -> bool {
        !self.exited
    }

    pub fn failure(&self) -> Option<String> {
        self.status.and_then(exit_failure)
    }

    pub fn update(&mut self, Event::BackendCall(id, cmd): Event) {
        if id != self.widget.id || self.exited {
            return;
        }
        if let BackendCommand::ProcessAlacrittyEvent(AlacrittyEvent::ChildExit(status)) = &cmd {
            self.status = Some(*status);
        }
        let action = self.widget.handle(Command::ProxyToBackend(cmd));
        self.exited = matches!(action, iced_term::actions::Action::Shutdown);
    }
}

/// The notice for a tmux client that exited unsuccessfully, or `None` for a
/// clean exit (for example, the session's last shell exited).
fn exit_failure(status: ExitStatus) -> Option<String> {
    if status.success() {
        return None;
    }
    // `code()` is `None` when a signal ended the client; ExitStatus's
    // Display then names the signal, e.g. "signal: 9 (SIGKILL)".
    let reason = match status.code() {
        Some(code) => format!("exited with status {code}"),
        None => format!("was stopped ({status})"),
    };
    Some(format!("Terminal client {reason}. Reconnect to try again."))
}

#[cfg(test)]
mod tests {
    use super::*;
    // `from_raw` builds an ExitStatus from a raw wait(2) status word: the exit
    // code sits in the second byte, a terminating signal in the low bits.
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn a_clean_client_exit_is_not_reported_as_a_failure() {
        assert_eq!(exit_failure(ExitStatus::from_raw(0)), None);
    }

    #[test]
    fn a_failed_client_exit_reports_its_exit_code() {
        assert_eq!(
            exit_failure(ExitStatus::from_raw(1 << 8)).as_deref(),
            Some("Terminal client exited with status 1. Reconnect to try again.")
        );
    }

    #[test]
    fn a_client_killed_by_a_signal_reports_the_signal() {
        let message = exit_failure(ExitStatus::from_raw(9)).unwrap_or_default();
        assert!(message.starts_with("Terminal client was stopped (signal: 9"));
        assert!(message.ends_with("Reconnect to try again."));
    }
}
