use crate::actions::Action;
use crate::settings::BackendSettings;
use alacritty_terminal::event::{
    Event, EventListener, Notify, OnResize, WindowSize,
};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::{
    self, cell::Cell, test::TermSize, viewport_to_point, RenderableCursor,
    Term, TermMode,
};
use alacritty_terminal::vte::ansi::{CursorShape, NamedColor, Rgb};
use alacritty_terminal::{tty, Grid};
use iced::keyboard::Modifiers;
use iced_core::Size;
use std::borrow::Cow;
use std::cmp::min;
use std::io::Result;
use std::ops::{Index, RangeInclusive};
use std::sync::Arc;
use tokio::sync::mpsc;

const URL_REGEX: &str = r#"(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file://|git://|ssh:|ftp://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩`]+"#;

#[derive(Debug, Clone)]
pub enum Command {
    Write(Vec<u8>),
    WheelScroll(i32, Modifiers, Point),
    Resize(Option<Size<f32>>, Option<Size<f32>>),
    SelectStart(SelectionType, (f32, f32)),
    SelectUpdate((f32, f32)),
    ProcessLink(LinkAction, Point),
    MouseReport(MouseButton, Modifiers, Point, bool),
    ProcessAlacrittyEvent(Event),
}

#[derive(Debug, Clone)]
pub enum MouseMode {
    Sgr,
    Normal(bool),
}

impl From<TermMode> for MouseMode {
    fn from(term_mode: TermMode) -> Self {
        if term_mode.contains(TermMode::SGR_MOUSE) {
            MouseMode::Sgr
        } else if term_mode.contains(TermMode::UTF8_MOUSE) {
            MouseMode::Normal(true)
        } else {
            MouseMode::Normal(false)
        }
    }
}

#[derive(Debug, Clone)]
pub enum MouseButton {
    LeftButton = 0,
    MiddleButton = 1,
    RightButton = 2,
    LeftMove = 32,
    MiddleMove = 33,
    RightMove = 34,
    NoneMove = 35,
    ScrollUp = 64,
    ScrollDown = 65,
    Other = 99,
}

#[derive(Debug, Clone)]
pub enum LinkAction {
    Clear,
    Hover,
    Open,
}

#[derive(Clone, Copy, Debug)]
pub struct TerminalSize {
    pub cell_width: f32,
    pub cell_height: f32,
    num_cols: u16,
    num_lines: u16,
    layout_width: f32,
    layout_height: f32,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            cell_width: 1.0,
            cell_height: 1.0,
            num_cols: 80,
            num_lines: 50,
            layout_width: 80.0,
            layout_height: 50.0,
        }
    }
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn columns(&self) -> usize {
        self.num_cols as usize
    }

    fn last_column(&self) -> Column {
        Column(self.num_cols as usize - 1)
    }

    fn bottommost_line(&self) -> Line {
        Line(self.num_lines as i32 - 1)
    }

    fn screen_lines(&self) -> usize {
        self.num_lines as usize
    }
}

impl From<TerminalSize> for WindowSize {
    fn from(size: TerminalSize) -> Self {
        Self {
            num_lines: size.num_lines,
            num_cols: size.num_cols,
            cell_width: size.cell_width.round() as u16,
            cell_height: size.cell_height.round() as u16,
        }
    }
}

pub struct Backend {
    term: Arc<FairMutex<Term<EventProxy>>>,
    size: TerminalSize,
    notifier: Notifier,
    last_content: RenderableContent,
    pub(crate) url_regex: RegexSearch,
}

impl Backend {
    pub fn new(
        id: u64,
        pty_event_proxy_sender: mpsc::Sender<Event>,
        settings: BackendSettings,
    ) -> Result<Self> {
        let pty_config = tty::Options {
            shell: Some(tty::Shell::new(settings.program, settings.args)),
            working_directory: settings.working_directory,
            env: settings.env,
            ..tty::Options::default()
        };

        let config = term::Config::default();
        let terminal_size = TerminalSize::default();
        let pty = tty::new(&pty_config, terminal_size.into(), id)?;

        let event_proxy = EventProxy(pty_event_proxy_sender);

        let term = Term::new(config, &terminal_size, event_proxy.clone());
        let cursor = term.renderable_content().cursor;

        let initial_content = RenderableContent {
            grid: term.grid().clone(),
            selectable_range: None,
            terminal_mode: *term.mode(),
            terminal_size,
            cursor,
            cursor_override: None,
            hovered_hyperlink: None,
        };

        let term = Arc::new(FairMutex::new(term));

        let pty_event_loop =
            EventLoop::new(term.clone(), event_proxy, pty, false, false)?;

        let notifier = Notifier(pty_event_loop.channel());

        let _ = pty_event_loop.spawn();

        Ok(Self {
            term: term.clone(),
            size: terminal_size,
            notifier,
            last_content: initial_content,
            url_regex: RegexSearch::new(URL_REGEX).expect("invalid url regexp"),
        })
    }

    pub fn handle(&mut self, cmd: Command) -> Action {
        let mut action = Action::default();
        let term = self.term.clone();
        let mut term = term.lock();
        match cmd {
            Command::ProcessAlacrittyEvent(event) => {
                match event {
                    Event::Exit => {
                        action = Action::Shutdown;
                    },
                    Event::Title(title) => {
                        action = Action::ChangeTitle(title);
                    },
                    Event::PtyWrite(pty) => {
                        self.notifier.notify(pty.into_bytes())
                    },
                    _ => {},
                };
            },
            Command::Write(input) => {
                self.write(input);
                term.scroll_display(Scroll::Bottom);
            },
            Command::WheelScroll(lines, modifiers, point) => {
                self.scroll(&mut term, lines, modifiers, point);
            },
            Command::Resize(layout_size, font_measure) => {
                self.resize(&mut term, layout_size, font_measure);
            },
            Command::SelectStart(selection_type, (x, y)) => {
                self.start_selection(&mut term, selection_type, x, y);
            },
            Command::SelectUpdate((x, y)) => {
                self.update_selection(&mut term, x, y);
            },
            Command::ProcessLink(link_action, point) => {
                self.process_link_action(&term, link_action, point);
            },
            Command::MouseReport(button, modifiers, point, pressed) => {
                self.process_mouse_report(button, modifiers, point, pressed);
            },
        };

        action
    }

    fn process_link_action(
        &mut self,
        terminal: &Term<EventProxy>,
        link_action: LinkAction,
        point: Point,
    ) {
        match link_action {
            LinkAction::Hover => {
                self.last_content.hovered_hyperlink = self.regex_match_at(
                    terminal,
                    point,
                    &mut self.url_regex.clone(),
                );
            },
            LinkAction::Clear => {
                self.last_content.hovered_hyperlink = None;
            },
            LinkAction::Open => {
                self.open_link();
            },
        };
    }

    fn open_link(&self) {
        if let Some(range) = &self.last_content.hovered_hyperlink {
            let start = range.start();
            let end = range.end();

            let mut url = String::from(self.last_content.grid.index(*start).c);
            for indexed in self.last_content.grid.iter_from(*start) {
                url.push(indexed.c);
                if indexed.point == *end {
                    break;
                }
            }

            open::that(url).unwrap_or_else(|_| {
                panic!("link opening is failed");
            })
        }
    }

    fn process_mouse_report(
        &self,
        button: MouseButton,
        modifiers: Modifiers,
        point: Point,
        pressed: bool,
    ) {
        if let Some(bytes) = Self::mouse_report_sequence(
            self.last_content.terminal_mode,
            button,
            modifiers,
            point,
            pressed,
        ) {
            self.notifier.notify(bytes);
        }
    }

    fn mouse_report_sequence(
        mode: TermMode,
        button: MouseButton,
        modifiers: Modifiers,
        point: Point,
        pressed: bool,
    ) -> Option<Vec<u8>> {
        let mods = (if modifiers.shift() { 4 } else { 0 })
            + (if modifiers.alt() { 8 } else { 0 })
            + (if modifiers.control() { 16 } else { 0 });
        match MouseMode::from(mode) {
            MouseMode::Sgr => {
                let suffix = if pressed { 'M' } else { 'm' };
                Some(
                    format!(
                        "\x1b[<{};{};{}{suffix}",
                        button as u8 + mods,
                        point.column + 1,
                        point.line + 1
                    )
                    .into_bytes(),
                )
            },
            MouseMode::Normal(is_utf8) => {
                let code = if pressed { button as u8 } else { 3 };
                Self::normal_mouse_report(point, code + mods, is_utf8)
            },
        }
    }

    fn normal_mouse_report(
        point: Point,
        button: u8,
        is_utf8: bool,
    ) -> Option<Vec<u8>> {
        let Point { line, column } = point;
        let max_point = if is_utf8 { 2015 } else { 223 };

        if line.0 < 0 || line >= max_point || column >= max_point {
            return None;
        }

        let mut msg = vec![b'\x1b', b'[', b'M', 32 + button];

        let mouse_pos_encode = |pos: usize| -> Vec<u8> {
            let pos = 32 + 1 + pos;
            let first = 0xC0 + pos / 64;
            let second = 0x80 + (pos & 63);
            vec![first as u8, second as u8]
        };

        if is_utf8 && column >= Column(95) {
            msg.append(&mut mouse_pos_encode(column.0));
        } else {
            msg.push(32 + 1 + column.0 as u8);
        }

        if is_utf8 && line >= 95 {
            msg.append(&mut mouse_pos_encode(line.0 as usize));
        } else {
            msg.push(32 + 1 + line.0 as u8);
        }

        Some(msg)
    }

    fn start_selection(
        &mut self,
        terminal: &mut Term<EventProxy>,
        selection_type: SelectionType,
        x: f32,
        y: f32,
    ) {
        let location = Self::selection_point(
            x,
            y,
            &self.size,
            terminal.grid().display_offset(),
        );
        terminal.selection = Some(Selection::new(
            selection_type,
            location,
            self.selection_side(x),
        ));
    }

    fn update_selection(
        &mut self,
        terminal: &mut Term<EventProxy>,
        x: f32,
        y: f32,
    ) {
        let display_offset = terminal.grid().display_offset();
        if let Some(ref mut selection) = terminal.selection {
            let location =
                Self::selection_point(x, y, &self.size, display_offset);
            selection.update(location, self.selection_side(x));
        }
    }

    pub fn selection_point(
        x: f32,
        y: f32,
        terminal_size: &TerminalSize,
        display_offset: usize,
    ) -> Point {
        let col = (x / terminal_size.cell_width) as usize;
        let col = min(Column(col), Column(terminal_size.num_cols as usize - 1));

        let line = (y / terminal_size.cell_height) as usize;
        let line = min(line, terminal_size.num_lines as usize - 1);

        viewport_to_point(display_offset, Point::new(line, col))
    }

    fn selection_side(&self, x: f32) -> Side {
        let cell_x = x.rem_euclid(self.size.cell_width);
        let half_cell_width = self.size.cell_width / 2.0;

        if cell_x > half_cell_width {
            Side::Right
        } else {
            Side::Left
        }
    }

    fn resize(
        &mut self,
        terminal: &mut Term<EventProxy>,
        layout_size: Option<Size<f32>>,
        font_measure: Option<Size<f32>>,
    ) {
        if let Some(size) = layout_size {
            self.size.layout_height = size.height;
            self.size.layout_width = size.width;
        };

        if let Some(size) = font_measure {
            self.size.cell_height = size.height.max(1.0);
            self.size.cell_width = size.width.max(1.0);
        }

        let lines =
            (self.size.layout_height / self.size.cell_height).floor() as u16;
        let cols =
            (self.size.layout_width / self.size.cell_width).floor() as u16;
        if lines > 0 && cols > 0 {
            self.size.num_lines = lines;
            self.size.num_cols = cols;
            self.notifier.on_resize(self.size.into());
            terminal.resize(TermSize::new(
                self.size.num_cols as usize,
                self.size.num_lines as usize,
            ));
        }
    }

    fn write<I: Into<Cow<'static, [u8]>>>(&self, input: I) {
        self.notifier.notify(input);
    }

    fn scroll(
        &mut self,
        terminal: &mut Term<EventProxy>,
        lines: i32,
        modifiers: Modifiers,
        point: Point,
    ) {
        if lines == 0 {
            return;
        }
        let mode = *terminal.mode();
        match scroll_route(mode, modifiers) {
            ScrollRoute::Mouse => {
                let button = if lines > 0 {
                    MouseButton::ScrollUp
                } else {
                    MouseButton::ScrollDown
                };
                // Wheel events are presses only. Keep the position so tmux can
                // scroll the pane under the pointer, not just the focused pane.
                if let Some(report) = Self::mouse_report_sequence(
                    mode, button, modifiers, point, true,
                ) {
                    self.notifier.notify(
                        report.repeat(lines.unsigned_abs().min(100) as usize),
                    );
                }
            },
            ScrollRoute::Alternate => {
                let key = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                self.notifier
                    .notify(key.repeat(lines.unsigned_abs().min(100) as usize));
            },
            ScrollRoute::History => {
                terminal.scroll_display(Scroll::Delta(lines))
            },
        }
    }

    pub fn selectable_content(&self) -> String {
        let content = self.renderable_content();
        let mut result = String::new();
        if let Some(range) = content.selectable_range {
            for indexed in content.grid.display_iter() {
                if range.contains(indexed.point) {
                    result.push(indexed.c);
                }
            }
        }
        result
    }

    pub fn sync(&mut self) {
        let term = self.term.clone();
        let mut term = term.lock();
        self.internal_sync(&mut term);
    }

    fn internal_sync(&mut self, terminal: &mut Term<EventProxy>) {
        let selectable_range = match &terminal.selection {
            Some(s) => s.to_range(terminal),
            None => None,
        };

        let content = terminal.renderable_content();
        self.last_content.cursor = content.cursor;
        self.last_content.cursor_override = content.colors[NamedColor::Cursor];
        self.last_content.grid = terminal.grid().clone();
        self.last_content.selectable_range = selectable_range;
        self.last_content.terminal_mode = *terminal.mode();
        self.last_content.terminal_size = self.size;
    }

    pub fn renderable_content(&self) -> &RenderableContent {
        &self.last_content
    }

    /// Based on alacritty/src/display/hint.rs > regex_match_at
    /// Retrieve the match, if the specified point is inside the content matching the regex.
    fn regex_match_at(
        &self,
        terminal: &Term<EventProxy>,
        point: Point,
        regex: &mut RegexSearch,
    ) -> Option<Match> {
        let x = visible_regex_match_iter(terminal, regex)
            .find(|rm| rm.contains(&point));
        x
    }
}

/// Copied from alacritty/src/display/hint.rs:
/// Iterate over all visible regex matches.
fn visible_regex_match_iter<'a>(
    term: &'a Term<EventProxy>,
    regex: &'a mut RegexSearch,
) -> impl Iterator<Item = Match> + 'a {
    let viewport_start = Line(-(term.grid().display_offset() as i32));
    let viewport_end = viewport_start + term.bottommost_line();
    let mut start =
        term.line_search_left(Point::new(viewport_start, Column(0)));
    let mut end = term.line_search_right(Point::new(viewport_end, Column(0)));
    start.line = start.line.max(viewport_start - 100);
    end.line = end.line.min(viewport_end + 100);

    RegexIter::new(start, end, Direction::Right, term, regex)
        .skip_while(move |rm| rm.end().line < viewport_start)
        .take_while(move |rm| rm.start().line <= viewport_end)
}

#[derive(Debug, PartialEq, Eq)]
enum ScrollRoute {
    Mouse,
    Alternate,
    History,
}

fn scroll_route(mode: TermMode, modifiers: Modifiers) -> ScrollRoute {
    if modifiers.shift() {
        ScrollRoute::History
    } else if mode.intersects(TermMode::MOUSE_MODE) {
        ScrollRoute::Mouse
    } else if mode.contains(TermMode::ALTERNATE_SCROLL | TermMode::ALT_SCREEN) {
        ScrollRoute::Alternate
    } else {
        ScrollRoute::History
    }
}

pub struct RenderableContent {
    pub grid: Grid<Cell>,
    pub hovered_hyperlink: Option<RangeInclusive<Point>>,
    pub selectable_range: Option<SelectionRange>,
    pub cursor: RenderableCursor,
    pub cursor_override: Option<Rgb>,
    pub terminal_mode: TermMode,
    pub terminal_size: TerminalSize,
}

impl Default for RenderableContent {
    fn default() -> Self {
        Self {
            grid: Grid::new(0, 0, 0),
            hovered_hyperlink: None,
            selectable_range: None,
            cursor: RenderableCursor {
                shape: CursorShape::Hidden,
                point: Point::default(),
            },
            cursor_override: None,
            terminal_mode: TermMode::empty(),
            terminal_size: TerminalSize::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_prioritizes_tmux_mouse_mode_over_alternate_screen() {
        let alternate = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;
        let mouse = alternate | TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(scroll_route(mouse, Modifiers::empty()), ScrollRoute::Mouse);
        assert_eq!(scroll_route(mouse, Modifiers::SHIFT), ScrollRoute::History);
        assert_eq!(
            scroll_route(alternate, Modifiers::empty()),
            ScrollRoute::Alternate
        );
        assert_eq!(
            scroll_route(TermMode::empty(), Modifiers::empty()),
            ScrollRoute::History
        );
    }

    #[test]
    fn wheel_reports_direction_and_pointer_cell_to_tmux() {
        let point = Point::new(Line(4), Column(17));
        let mode = TermMode::SGR_MOUSE | TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(
            Backend::mouse_report_sequence(
                mode,
                MouseButton::ScrollUp,
                Modifiers::empty(),
                point,
                true
            )
            .unwrap(),
            b"\x1b[<64;18;5M"
        );
        assert_eq!(
            Backend::mouse_report_sequence(
                mode,
                MouseButton::ScrollDown,
                Modifiers::CTRL,
                point,
                true
            )
            .unwrap(),
            b"\x1b[<81;18;5M"
        );
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

#[derive(Clone)]
pub struct EventProxy(mpsc::Sender<Event>);

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.0.blocking_send(event);
    }
}
