use crate::actions::Action;
use crate::backend;
use crate::bindings::{Binding, BindingAction, BindingsLayout, InputKind};
use crate::font::TermFont;
use crate::settings::{FontSettings, Settings, ThemeSettings};
use crate::theme::{ColorPalette, Theme};
use crate::AlacrittyEvent;
use iced::futures::stream::BoxStream;
use iced::futures::{SinkExt, StreamExt};
use iced::widget::canvas::Cache;
use iced::Subscription;
use std::hash::{Hash, Hasher};
use std::io::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub enum Event {
    BackendCall(u64, backend::Command),
}

#[derive(Debug, Clone)]
pub enum Command {
    ChangeTheme(Box<ColorPalette>),
    ChangeFont(FontSettings),
    AddBindings(Vec<(Binding<InputKind>, BindingAction)>),
    ProxyToBackend(backend::Command),
}

pub struct Terminal {
    pub id: u64,
    widget_id: iced::widget::Id,
    pub(crate) font: TermFont,
    pub(crate) theme: Theme,
    pub(crate) cache: Cache,
    pub(crate) bindings: BindingsLayout,
    pub(crate) backend: backend::Backend,
    backend_event_rx: Arc<Mutex<backend::EventReceiver>>,
}

impl Terminal {
    pub fn new(id: u64, settings: Settings) -> Result<Self> {
        let (event_proxy, backend_event_rx) = backend::event_channel();
        let theme = Theme::new(settings.theme);
        let font = TermFont::new(settings.font);

        let mut terminal = Self {
            id,
            widget_id: iced::widget::Id::unique(),
            font,
            theme,
            bindings: BindingsLayout::default(),
            cache: Cache::default(),
            backend: backend::Backend::new(id, event_proxy, settings.backend)?,
            backend_event_rx: Arc::new(Mutex::new(backend_event_rx)),
        };
        // Give the backend real cell metrics before the first event, as the
        // per-command font sync used to.
        terminal.sync_font();
        terminal.backend.sync();
        Ok(terminal)
    }

    pub fn widget_id(&self) -> &iced::widget::Id {
        &self.widget_id
    }

    /// Patch: the process id of the program running in the PTY.
    #[cfg(unix)]
    pub fn child_pid(&self) -> u32 {
        self.backend.child_pid
    }

    pub fn subscription(&self) -> Subscription<Event> {
        let data = TerminalSubscriptionData {
            id: self.id,
            event_receiver: self.backend_event_rx.clone(),
        };

        Subscription::run_with(data, terminal_subscription_stream)
    }

    pub fn handle(&mut self, cmd: Command) -> Action {
        let mut action = Action::default();

        match cmd {
            Command::ChangeTheme(color_pallete) => {
                self.theme = Theme::new(ThemeSettings::new(color_pallete));
            },
            Command::ChangeFont(font_settings) => {
                self.font = TermFont::new(font_settings);
                self.sync_font();
            },
            Command::AddBindings(bindings) => {
                self.bindings.add_bindings(bindings);
            },
            Command::ProxyToBackend(cmd) => {
                action = self.backend.handle(cmd);
            },
        };

        // Patch: upstream re-measured the font and resized the PTY here on
        // every command, so each keystroke and PTY wakeup also sent a resize
        // to the PTY thread. Only a font change can alter the cell metrics.
        self.backend.sync();
        self.redraw();
        action
    }

    fn sync_font(&mut self) {
        self.font.sync();
        self.backend
            .handle(backend::Command::Resize(None, Some(self.font.measure)));
    }

    fn redraw(&mut self) {
        self.cache.clear();
    }
}

#[derive(Clone)]
struct TerminalSubscriptionData {
    id: u64,
    event_receiver: Arc<Mutex<backend::EventReceiver>>,
}

impl Hash for TerminalSubscriptionData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

fn terminal_subscription_stream(
    data: &TerminalSubscriptionData,
) -> BoxStream<'static, Event> {
    let id = data.id;
    let event_receiver = data.event_receiver.clone();
    iced::stream::channel(1000, async move |mut output| {
        let mut shutdown = false;
        loop {
            let mut event_receiver = event_receiver.lock().await;
            match event_receiver.recv().await {
                Some(event) => {
                    // Patch: drain whatever else is already queued so a burst of
                    // output costs one grid sync and redraw, not one per wakeup.
                    let mut batch = vec![event];
                    while let Some(next) = event_receiver.try_recv() {
                        batch.push(next);
                    }
                    for event in coalesce_wakeups(batch) {
                        if let AlacrittyEvent::Exit = event {
                            shutdown = true
                        };

                        output
                            .send(Event::BackendCall(id, backend::Command::ProcessAlacrittyEvent(event)))
                            .await
                            .unwrap_or_else(|_| {
                                panic!("iced_term stream {}: sending BackendEventReceived event is failed", id)
                            });
                    }
                },
                None => {
                    if !shutdown {
                        panic!("iced_term stream {}: terminal event channel closed unexpected", id);
                    }
                },
            }
        }
    })
    .boxed()
}

/// Keeps every event in order but collapses repeated `Wakeup`s (which only
/// mean "the grid changed, redraw") into a single one at the end of the batch.
fn coalesce_wakeups(batch: Vec<AlacrittyEvent>) -> Vec<AlacrittyEvent> {
    let mut woke = false;
    let mut events: Vec<AlacrittyEvent> = batch
        .into_iter()
        .filter(|event| {
            let wakeup = matches!(event, AlacrittyEvent::Wakeup);
            woke |= wakeup;
            !wakeup
        })
        .collect();
    if woke {
        events.push(AlacrittyEvent::Wakeup);
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_of_wakeups_become_one_redraw_after_other_events() {
        let events = coalesce_wakeups(vec![
            AlacrittyEvent::Wakeup,
            AlacrittyEvent::Bell,
            AlacrittyEvent::Wakeup,
            AlacrittyEvent::Title("t".into()),
            AlacrittyEvent::Wakeup,
        ]);
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], AlacrittyEvent::Bell));
        assert!(matches!(events[1], AlacrittyEvent::Title(ref t) if t == "t"));
        assert!(matches!(events[2], AlacrittyEvent::Wakeup));
    }

    #[test]
    fn batches_without_wakeups_are_unchanged() {
        let events =
            coalesce_wakeups(vec![AlacrittyEvent::Bell, AlacrittyEvent::Exit]);
        assert!(matches!(
            events[..],
            [AlacrittyEvent::Bell, AlacrittyEvent::Exit]
        ));
    }
}
