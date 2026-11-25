//! A headless copy of iced's event loop around the real [`App`], timed.
//!
//! iced normally runs the app in a window: an input event goes to the widgets
//! (`UserInterface::update`), the messages they publish go to `App::update`,
//! the UI is rebuilt from `App::view`, and a frame is drawn and presented.
//! Terminal output arrives the same way, as subscription messages. This does
//! each of those steps itself, in the same order, so it can time them, and
//! renders every frame offscreen and reads its pixels back instead of
//! presenting it. A keystroke's latency ends at the first frame whose pixels
//! differ from the frame on screen when the key was pressed.
//!
//! Absolute numbers are a proxy for the real app, which also waits for the
//! display's refresh. They are consistent from run to run on one machine,
//! which is what comparing two builds needs. Without a GPU, frames are drawn
//! by tiny-skia on the CPU, which is far slower at text, so its numbers only
//! compare with other tiny-skia runs.

use super::Options;
use super::scenarios::{Keystroke, Step};
use crate::app::{App, Message};
use crate::git::{Checkout, Worktree};
use crate::style;
use iced::advanced::renderer::{self, Headless};
use iced::advanced::{clipboard, mouse, widget};
use iced::futures::stream::BoxStream;
use iced::futures::{FutureExt, StreamExt};
use iced::{Size, Task, window};
use iced_runtime::user_interface::{self, UserInterface};
use iced_runtime::{Action, task};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long a measured key may take to change the screen before the run fails.
const KEY_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for the terminal program to start.
const START_TIMEOUT: Duration = Duration::from_secs(15);

/// Where one keystroke's time went. `wait` is time the UI thread was idle,
/// waiting for the terminal program's answer; the rest is UI-thread work.
/// `raster` includes the renderer's own preparation (glyph layout, uploads)
/// and reading the frame back.
#[derive(Clone, Copy, Debug, Default)]
pub struct Phases {
    /// Handling input: widget events and `App::update` (incl. terminal output).
    pub update: Duration,
    /// Rebuilding the widget tree from `App::view`, with layout.
    pub view: Duration,
    /// Drawing the widget tree into primitives.
    pub draw: Duration,
    /// Rasterizing the primitives into pixels.
    pub raster: Duration,
    /// Waiting for the terminal program's output.
    pub wait: Duration,
}

impl Phases {
    /// Key press to changed pixels.
    pub fn total(&self) -> Duration {
        self.app() + self.raster + self.wait
    }

    /// The app's own work before the renderer takes the frame: input,
    /// `App::update`, `App::view` and drawing the widgets.
    pub fn app(&self) -> Duration {
        self.update + self.view + self.draw
    }
}

/// The worktree the harness opens.
pub struct Workspace {
    pub checkout: Checkout,
    pub inventory: Vec<Worktree>,
    pub worktree: PathBuf,
}

pub struct Harness {
    app: App,
    renderer: iced::Renderer,
    /// Widget state (focus, text field contents) between rebuilds.
    cache: user_interface::Cache,
    runtime: tokio::runtime::Runtime,
    terminal: Option<BoxStream<'static, iced_term::Event>>,
    /// The window size in logical pixels, and its scale factor.
    bounds: Size,
    scale: f32,
    /// The pixels "on screen": the last frame drawn.
    frame: Vec<u8>,
    /// Messages widgets published while a frame was drawn (the terminal
    /// sizing its PTY to the pane, for one). iced handles them with the next
    /// update, and so does the harness.
    queued: Vec<Message>,
}

/// What became of a key sent to the widgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivery {
    /// No widget handled it.
    Ignored,
    /// A widget handled it without changing anything iced draws, so iced
    /// draws no frame for it.
    Captured,
    /// It changed the app, or a widget asked for a redraw: iced draws a frame.
    Redraw,
}

type Ui<'a> = UserInterface<'a, Message, iced::Theme, iced::Renderer>;

/// Builds the widget tree for `app`'s current state. A free function, not a
/// method, so the borrow checker can see that the tree borrows only `app`
/// while `renderer` is borrowed mutably.
fn build<'a>(
    app: &'a App,
    renderer: &mut iced::Renderer,
    cache: user_interface::Cache,
    bounds: Size,
) -> Ui<'a> {
    UserInterface::build(app.view(), bounds, cache, renderer)
}

impl Harness {
    /// Opens `workspace` with a terminal running the echo program, ready for
    /// the first step.
    /// Creates the renderer. One serves every scenario, as one serves the
    /// app for its whole life: a new GPU device starts with cold caches
    /// (shaders, glyph atlas) and clocks, which would skew the first frames.
    pub fn renderer_for(options: &Options) -> Result<iced::Renderer, String> {
        let preferences = crate::terminal_config::Preferences::default();
        // `None` picks wgpu (the GPU, as the app draws) and falls back to
        // tiny-skia (the CPU) on a machine without a GPU.
        runtime()?
            .block_on(<iced::Renderer as Headless>::new(
                preferences.font.font_type,
                iced::Pixels(16.0),
                options.renderer.as_deref(),
            ))
            .ok_or_else(|| "Could not create the renderer".to_owned())
    }

    /// Gives the renderer back for the next scenario, ending this one's app
    /// and terminal.
    pub fn into_renderer(self) -> iced::Renderer {
        self.renderer
    }

    pub fn start(
        options: &Options,
        workspace: &Workspace,
        renderer: iced::Renderer,
    ) -> Result<Self, String> {
        let runtime = runtime()?;
        let app = App::perf_workspace(
            workspace.checkout.clone(),
            workspace.inventory.clone(),
            workspace.worktree.clone(),
            options.scale,
        );
        let mut harness = Self {
            app,
            renderer,
            cache: user_interface::Cache::default(),
            runtime,
            terminal: None,
            bounds: options.window,
            scale: options.scale,
            frame: Vec::new(),
            queued: Vec::new(),
        };

        let program = std::env::current_exe()
            .map_err(|error| format!("Could not find this executable: {error}"))?;
        let backend = iced_term::settings::BackendSettings {
            program: program
                .to_str()
                .ok_or("This executable's path must be UTF-8")?
                .to_owned(),
            args: vec![super::ECHO_FLAG.to_owned()],
            working_directory: Some(workspace.worktree.clone()),
            ..Default::default()
        };
        let focus = harness.app.perf_attach(backend);
        harness.run_task(focus)?;
        harness.terminal = harness.app.perf_terminal_events();
        if harness.terminal.is_none() {
            return Err("The terminal did not start".into());
        }
        // Wait for the program's "ready". Drawing it also lets the terminal
        // widget size the PTY to its pane (settling applies that message).
        harness.expect_output(START_TIMEOUT)?;
        Ok(harness)
    }

    /// Runs one unmeasured step, or measures a keystroke.
    pub fn step(&mut self, step: Step) -> Result<Option<Phases>, String> {
        match step {
            Step::Key(key) => {
                let phases = self.press(key)?;
                // Let any trailing output land before the next key.
                self.settle_for(Duration::from_millis(5))?;
                Ok(Some(phases))
            }
            Step::Terminal(command) => {
                let task = self
                    .app
                    .perf_write_terminal(vec![super::echo::COMMAND, command]);
                self.run_task(task)?;
                self.expect_output(KEY_TIMEOUT).map(|()| None)
            }
        }
    }

    /// Presses `key` and returns how long the screen took to change.
    fn press(&mut self, key: Keystroke) -> Result<Phases, String> {
        let mut phases = Phases::default();
        let delivery = self.deliver(&[key.event()], &mut phases)?;
        if delivery == Delivery::Ignored {
            return Err(format!(
                "{key:?} reached no widget (is the terminal or field focused?)"
            ));
        }
        if delivery == Delivery::Redraw && self.render(&mut phases) {
            return Ok(phases);
        }
        let deadline = Instant::now() + KEY_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let events = self
                .terminal_output(remaining, &mut phases)
                .ok_or_else(|| format!("{key:?} changed nothing on screen in {KEY_TIMEOUT:?}"))?;
            self.apply_terminal_output(events, &mut phases)?;
            if self.render(&mut phases) {
                return Ok(phases);
            }
        }
    }

    /// Sends `events` to the widgets, as iced does with window input, and
    /// gives `App::update` every message they publish. Says whether iced
    /// would draw a frame now: only when a widget published a message or
    /// asked for a redraw. A widget can handle a key with neither (the
    /// terminal writes typed bytes straight to its PTY).
    fn deliver(&mut self, events: &[iced::Event], phases: &mut Phases) -> Result<Delivery, String> {
        // iced keeps the tree it built for the last frame; rebuilding it from
        // the cache here gives the same tree, so it isn't timed.
        let mut ui = build(
            &self.app,
            &mut self.renderer,
            std::mem::take(&mut self.cache),
            self.bounds,
        );
        let start = Instant::now();
        let mut messages = Vec::new();
        let (state, statuses) = ui.update(
            events,
            mouse::Cursor::Unavailable,
            &mut self.renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        self.cache = ui.into_cache();
        phases.update += start.elapsed();
        // iced_winit's rule after window events: redraw for published
        // messages, an outdated tree, or a widget's redraw request.
        let redraw = !messages.is_empty()
            || match state {
                user_interface::State::Outdated => true,
                user_interface::State::Updated { redraw_request, .. } => {
                    redraw_request != window::RedrawRequest::Wait
                }
            };
        let captured = statuses.contains(&iced::event::Status::Captured);
        self.apply(messages, phases)?;
        Ok(if redraw {
            Delivery::Redraw
        } else if captured {
            Delivery::Captured
        } else {
            Delivery::Ignored
        })
    }

    fn apply_terminal_output(
        &mut self,
        events: Vec<iced_term::Event>,
        phases: &mut Phases,
    ) -> Result<(), String> {
        self.apply(events.into_iter().map(Message::Terminal), phases)
    }

    /// Gives `App::update` any queued messages, then `messages`.
    fn apply(
        &mut self,
        messages: impl IntoIterator<Item = Message>,
        phases: &mut Phases,
    ) -> Result<(), String> {
        let start = Instant::now();
        let queued = std::mem::take(&mut self.queued);
        for message in queued.into_iter().chain(messages) {
            let task = self.app.update(message);
            self.run_task(task)?;
        }
        phases.update += start.elapsed();
        Ok(())
    }

    /// Rebuilds the view, draws it and rasterizes the frame, as iced does
    /// after an update. Returns whether its pixels differ from the frame
    /// before, which it then replaces.
    fn render(&mut self, phases: &mut Phases) -> bool {
        let start = Instant::now();
        let mut ui = build(
            &self.app,
            &mut self.renderer,
            std::mem::take(&mut self.cache),
            self.bounds,
        );
        phases.view += start.elapsed();

        let start = Instant::now();
        // Widgets see a redraw event before each frame; what they publish
        // then waits for the next update.
        let _ = ui.update(
            &[redraw_event()],
            mouse::Cursor::Unavailable,
            &mut self.renderer,
            &mut clipboard::Null,
            &mut self.queued,
        );
        let theme = self.app.theme();
        let window = style::application(self.app.palette());
        ui.draw(
            &mut self.renderer,
            &theme,
            &renderer::Style {
                text_color: window.text_color,
            },
            mouse::Cursor::Unavailable,
        );
        self.cache = ui.into_cache();
        phases.draw += start.elapsed();

        let start = Instant::now();
        let physical = Size::new(
            (self.bounds.width * self.scale).round() as u32,
            (self.bounds.height * self.scale).round() as u32,
        );
        let pixels = self
            .renderer
            .screenshot(physical, self.scale, window.background_color);
        phases.raster += start.elapsed();

        // Comparing frames is the harness's own work: not timed.
        let changed = pixels != self.frame;
        self.frame = pixels;
        changed
    }

    /// Waits up to `limit` for terminal output, then takes whatever else is
    /// already queued, as one iced update would.
    fn terminal_output(
        &mut self,
        limit: Duration,
        phases: &mut Phases,
    ) -> Option<Vec<iced_term::Event>> {
        let stream = self.terminal.as_mut()?;
        let start = Instant::now();
        let first = self
            .runtime
            .block_on(async { tokio::time::timeout(limit, stream.next()).await })
            .ok()
            .flatten();
        phases.wait += start.elapsed();
        let mut events = vec![first?];
        // The subscription stream alternates between its queue and the task
        // that fills it, so only two empty polls in a row mean it's drained.
        let mut empty = 0;
        while empty < 2 {
            match self
                .runtime
                .block_on(async { stream.next().now_or_never() })
            {
                Some(Some(event)) => {
                    events.push(event);
                    empty = 0;
                }
                Some(None) => break,
                None => empty += 1,
            }
        }
        Some(events)
    }

    /// Fails unless the terminal program writes something within `limit`;
    /// then lets it finish and draws the result.
    fn expect_output(&mut self, limit: Duration) -> Result<(), String> {
        let mut unused = Phases::default();
        let events = self
            .terminal_output(limit, &mut unused)
            .ok_or_else(|| format!("The terminal program wrote nothing in {limit:?}"))?;
        self.apply_terminal_output(events, &mut unused)?;
        self.settle()
    }

    /// Applies terminal output until there has been none for a while, then
    /// draws the frame "on screen" for the next measurement.
    fn settle(&mut self) -> Result<(), String> {
        let mut unused = Phases::default();
        // Handle what drawing published (a PTY resize makes the program
        // repaint), until nothing new comes in.
        for _ in 0..10 {
            self.settle_for(Duration::from_millis(100))?;
            self.render(&mut unused);
            if self.queued.is_empty() {
                return Ok(());
            }
            self.apply(Vec::new(), &mut unused)?;
        }
        Err("The app kept publishing messages while idle".into())
    }

    /// Applies terminal output until none arrives for `quiet`, drawing a
    /// frame only if any did.
    fn settle_for(&mut self, quiet: Duration) -> Result<(), String> {
        let deadline = Instant::now() + KEY_TIMEOUT;
        let mut unused = Phases::default();
        while let Some(events) = self.terminal_output(quiet, &mut unused) {
            self.apply_terminal_output(events, &mut unused)?;
            self.render(&mut unused);
            if Instant::now() > deadline {
                return Err("The terminal program never stopped writing".into());
            }
        }
        Ok(())
    }

    /// Runs `task` to completion the way iced's runtime would: messages go
    /// back to `App::update`, and widget operations (such as focusing the
    /// terminal) run against the widget tree. Window, clipboard and system
    /// actions have no window to act on here and are dropped.
    fn run_task(&mut self, task: Task<Message>) -> Result<(), String> {
        let mut queue = VecDeque::from([task]);
        while let Some(task) = queue.pop_front() {
            let Some(stream) = task::into_stream(task) else {
                continue;
            };
            // `timeout` starts its timer when created, which needs the
            // runtime, so create it inside the `async` block `block_on` runs.
            let actions = self
                .runtime
                .block_on(async {
                    tokio::time::timeout(START_TIMEOUT, stream.collect::<Vec<_>>()).await
                })
                .map_err(|_| "An app task did not finish".to_owned())?;
            for action in actions {
                match action {
                    Action::Output(message) => queue.push_back(self.app.update(message)),
                    Action::Widget(operation) => self.operate(operation),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn operate(&mut self, operation: Box<dyn widget::Operation>) {
        let mut ui = build(
            &self.app,
            &mut self.renderer,
            std::mem::take(&mut self.cache),
            self.bounds,
        );
        let mut current = Some(operation);
        // An operation can hand on to another (`Outcome::Chain`); run them all.
        while let Some(mut operation) = current.take() {
            ui.operate(&self.renderer, operation.as_mut());
            if let widget::operation::Outcome::Chain(next) = operation.finish() {
                current = Some(next);
            }
        }
        self.cache = ui.into_cache();
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Could not start a Tokio runtime: {error}"))
}

fn redraw_event() -> iced::Event {
    iced::Event::Window(window::Event::RedrawRequested(iced::time::Instant::now()))
}
