use crate::terminal_config::Preferences;
use iced::{Element, Subscription, Task};
use iced_term::settings::{BackendSettings, Settings, ThemeSettings};
use iced_term::{AlacrittyEvent, BackendCommand, Command, Event, Terminal, TerminalView};

pub struct Console {
    widget: Terminal,
    exited: bool,
    status: Option<i32>,
    scale: f32,
}

impl Console {
    pub fn open(
        id: u64,
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
            exited: false,
            status: None,
            scale,
        })
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
        self.status.filter(|&status| status != 0).map(|status| {
            format!("Terminal client exited with status {status}. Reconnect to try again.")
        })
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
