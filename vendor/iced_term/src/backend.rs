use crate::actions::Action;
use crate::settings::BackendSettings;
use alacritty_terminal::event::{
    Event, EventListener, Notify, OnResize, WindowSize,
};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{BidirectionalIterator, Dimensions, Scroll};
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

const URL_REGEX: &str = r#"(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file://|git://|ssh:|ftp://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩`]+"#;

#[derive(Debug, Clone)]
pub enum Command {
    Write(Vec<u8>),
    /// Paste printable, single-line text supplied by the host (e.g. a dropped path).
    PasteSingleLine(String),
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
    // Patch: the PTY's child process id. Worktree Browser runs a tmux client
    // there and finds it by pid to `switch-client` it to another session
    // instead of starting a new terminal.
    #[cfg(unix)]
    pub(crate) child_pid: u32,
}

impl Backend {
    pub fn new(
        id: u64,
        event_proxy: EventProxy,
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
        #[cfg(unix)]
        let child_pid = pty.child().id();

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
            #[cfg(unix)]
            child_pid,
        })
    }

    pub fn handle(&mut self, cmd: Command) -> Action {
        // Patch: alacritty events don't touch the grid, so handle them before
        // taking the terminal lock, which the PTY thread holds while it parses.
        let cmd = match cmd {
            Command::ProcessAlacrittyEvent(event) => {
                return match event {
                    Event::Exit => Action::Shutdown,
                    Event::Title(title) => Action::ChangeTitle(title),
                    Event::PtyWrite(pty) => {
                        self.notifier.notify(pty.into_bytes());
                        Action::default()
                    },
                    _ => Action::default(),
                };
            },
            cmd => cmd,
        };

        let term = self.term.clone();
        let mut term = term.lock();
        match cmd {
            // Handled above, before locking.
            Command::ProcessAlacrittyEvent(_) => {},
            Command::Write(input) => {
                self.write(input);
                term.scroll_display(Scroll::Bottom);
            },
            Command::PasteSingleLine(text) => {
                // Let the foreground program handle the whole text as a paste,
                // rather than as individual keystrokes, when it requests this mode.
                let input = if term.mode().contains(TermMode::BRACKETED_PASTE) {
                    format!("\x1b[200~{text}\x1b[201~").into_bytes()
                } else {
                    text.into_bytes()
                };
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

        Action::default()
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
        url_at(terminal, point, regex)
    }
}

/// The URL under `point`, trimmed the way Alacritty trims hint matches.
///
/// Patch: upstream iced_term returned the raw regex match, so a URL written
/// in parentheses, `(https://example.com/pull/1)`, opened with the closing
/// `)` attached. Alacritty runs `hint_post_processing` on every match.
fn url_at(
    term: &Term<EventProxy>,
    point: Point,
    regex: &mut RegexSearch,
) -> Option<Match> {
    let regex_match =
        visible_regex_match_iter(term, regex).find(|rm| rm.contains(&point))?;
    hint_post_processing(term, regex_match).filter(|rm| rm.contains(&point))
}

/// Copied from alacritty/src/display/hint.rs:
/// Drop unbalanced closing brackets, and trailing characters that are
/// probably punctuation around the URL rather than part of it.
fn hint_post_processing(
    term: &Term<EventProxy>,
    regex_match: Match,
) -> Option<Match> {
    let mut iter = term.grid().iter_from(*regex_match.start());

    let mut c = iter.cell().c;

    // Truncate uneven number of brackets.
    let end = *regex_match.end();
    let mut open_parents = 0;
    let mut open_brackets = 0;
    loop {
        match c {
            '(' => open_parents += 1,
            '[' => open_brackets += 1,
            ')' => {
                if open_parents == 0 {
                    iter.prev();
                    break;
                } else {
                    open_parents -= 1;
                }
            },
            ']' => {
                if open_brackets == 0 {
                    iter.prev();
                    break;
                } else {
                    open_brackets -= 1;
                }
            },
            _ => (),
        }

        if iter.point() == end {
            break;
        }

        match iter.next() {
            Some(indexed) => c = indexed.cell.c,
            None => break,
        }
    }

    // Patch: after dropping a `)` or `]`, look at the cell now at the end, so
    // `(see https://example.com.)` loses the `.` as well as the `)`.
    c = iter.cell().c;

    // Truncate trailing characters which are likely to be delimiters.
    let start = *regex_match.start();
    while iter.point() != start {
        if !matches!(c, '.' | ',' | ':' | ';' | '?' | '!' | '(' | '[' | '\'') {
            break;
        }

        match iter.prev() {
            Some(indexed) => c = indexed.cell.c,
            None => break,
        }
    }

    if start > iter.point() {
        None
    } else {
        Some(start..=iter.point())
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
    use std::time::Duration;

    #[test]
    fn pty_events_never_block_when_the_ui_is_not_draining() {
        // Reproduces the hang: the PTY thread sends events while holding the
        // terminal lock, and nothing drains the queue (the UI thread is stuck
        // in a macOS resize loop). Upstream's bounded channel blocked here
        // after 100 events, holding the lock forever.
        let (proxy, _receiver) = event_channel();
        let lock = Arc::new(FairMutex::new(()));
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let pty_lock = lock.clone();
        std::thread::spawn(move || {
            let _held = pty_lock.lock();
            for _ in 0..10_000 {
                proxy.send_event(Event::PtyWrite("\x1b[0n".into()));
                proxy.send_event(Event::Wakeup);
            }
            let _ = done_tx.send(());
        });

        assert!(
            done_rx.recv_timeout(Duration::from_secs(10)).is_ok(),
            "PTY thread blocked sending events"
        );
        // The UI thread can take the terminal lock again.
        drop(lock.lock());
    }

    #[test]
    fn queued_redraws_collapse_to_one_until_the_ui_takes_it() {
        let (proxy, mut receiver) = event_channel();
        proxy.send_event(Event::Wakeup);
        proxy.send_event(Event::Bell);
        proxy.send_event(Event::Wakeup);
        proxy.send_event(Event::Title("pi".into()));

        assert!(matches!(receiver.try_recv(), Some(Event::Wakeup)));
        assert!(matches!(receiver.try_recv(), Some(Event::Bell)));
        assert!(
            matches!(receiver.try_recv(), Some(Event::Title(ref t)) if t == "pi")
        );
        assert!(receiver.try_recv().is_none());

        // Output after the redraw was taken needs (and gets) a new one.
        proxy.send_event(Event::Wakeup);
        assert!(matches!(receiver.try_recv(), Some(Event::Wakeup)));
        assert!(receiver.try_recv().is_none());
    }

    /// The URL a ⌘-click at `column` of a one-line terminal showing `line`
    /// would open.
    fn url_clicked_at(line: &str, column: usize) -> Option<String> {
        let (proxy, _receiver) = event_channel();
        let size = TermSize::new(200, 1);
        let mut term = Term::new(term::Config::default(), &size, proxy);
        let mut parser: alacritty_terminal::vte::ansi::Processor =
            alacritty_terminal::vte::ansi::Processor::new();
        parser.advance(&mut term, line.as_bytes());

        let mut regex = RegexSearch::new(URL_REGEX).unwrap();
        let point = Point::new(Line(0), Column(column));
        let range = url_at(&term, point, &mut regex)?;
        Some(
            (range.start().column.0..=range.end().column.0)
                .map(|col| term.grid()[Line(0)][Column(col)].c)
                .collect(),
        )
    }

    #[test]
    fn links_in_parentheses_open_without_the_closing_parenthesis() {
        let line = "- Backend #647 (https://github.com/o/r/pull/647) — saves";
        let column = line.find("github").unwrap();
        assert_eq!(
            url_clicked_at(line, column).as_deref(),
            Some("https://github.com/o/r/pull/647")
        );
    }

    #[test]
    fn links_keep_balanced_parentheses_and_drop_trailing_punctuation() {
        let wiki = "see https://en.wikipedia.org/wiki/Rust_(language).";
        assert_eq!(
            url_clicked_at(wiki, 10).as_deref(),
            Some("https://en.wikipedia.org/wiki/Rust_(language)")
        );

        let sentence = "(read https://example.com/docs.) next";
        assert_eq!(
            url_clicked_at(sentence, 10).as_deref(),
            Some("https://example.com/docs")
        );

        let markdown = "[PR](https://github.com/o/r/pull/1)";
        assert_eq!(
            url_clicked_at(markdown, 10).as_deref(),
            Some("https://github.com/o/r/pull/1")
        );
    }

    #[test]
    fn clicking_the_trimmed_parenthesis_opens_nothing() {
        let line = "(https://example.com)";
        assert_eq!(url_clicked_at(line, line.len() - 1), None);
    }

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

/// Patch: creates the queue that carries alacritty events from the PTY thread
/// to the terminal subscription.
///
/// Upstream used a bounded channel with `blocking_send`. alacritty calls
/// `send_event` on its PTY thread *while holding the terminal lock*, and the
/// queue drains only as fast as the UI thread handles messages. When the UI
/// thread stalled (macOS runs window resize and zoom in a modal loop) under
/// heavy output, the queue filled, the PTY thread blocked holding the lock,
/// and the UI thread then waited on that lock forever. So sending must never
/// block: the queue is unbounded, and redraw requests (`Wakeup`) are collapsed
/// to at most one in flight so output alone can't grow it.
pub(crate) fn event_channel() -> (EventProxy, EventReceiver) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let wakeup_pending = Arc::new(AtomicBool::new(false));
    (
        EventProxy {
            sender,
            wakeup_pending: wakeup_pending.clone(),
        },
        EventReceiver {
            receiver,
            wakeup_pending,
        },
    )
}

#[derive(Clone)]
pub struct EventProxy {
    sender: mpsc::UnboundedSender<Event>,
    /// True while a `Wakeup` sits in the queue, not yet taken.
    wakeup_pending: Arc<AtomicBool>,
}

impl EventListener for EventProxy {
    /// Runs on the PTY thread with the terminal locked, so it must not block.
    fn send_event(&self, event: Event) {
        if matches!(event, Event::Wakeup)
            && self.wakeup_pending.swap(true, Ordering::AcqRel)
        {
            // A redraw is already queued. Its grid sync needs the lock this
            // thread holds, so it will include this output too.
            return;
        }
        // Fails only once the terminal (the receiver) is gone.
        let _ = self.sender.send(event);
    }
}

/// The subscription's end of [`event_channel`].
pub(crate) struct EventReceiver {
    receiver: mpsc::UnboundedReceiver<Event>,
    wakeup_pending: Arc<AtomicBool>,
}

impl EventReceiver {
    pub(crate) async fn recv(&mut self) -> Option<Event> {
        let event = self.receiver.recv().await;
        self.taken(event)
    }

    pub(crate) fn try_recv(&mut self) -> Option<Event> {
        let event = self.receiver.try_recv().ok();
        self.taken(event)
    }

    /// Once the queued `Wakeup` is taken, later output needs a new one.
    fn taken(&self, event: Option<Event>) -> Option<Event> {
        if matches!(event, Some(Event::Wakeup)) {
            self.wakeup_pending.store(false, Ordering::Release);
        }
        event
    }
}
