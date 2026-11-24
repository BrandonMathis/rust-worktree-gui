//! The program the latency harness runs in the embedded terminal instead of
//! tmux: a tiny line editor that answers each keystroke with the bytes a real
//! one would send back (an echoed character, a cursor move, a scroll, a
//! repainted screen) and nothing else.
//!
//! A shell or tmux in its place would add its own, variable, latency. This
//! program answers immediately and always the same way, so what the harness
//! measures is Worktree Browser's own pipeline: key event → bytes to the PTY →
//! parsed into the terminal grid → view → draw → pixels.
//!
//! [`LineEditor`] is pure (bytes in, bytes out) so it can be unit tested;
//! [`run`] wires it to the PTY.

use std::io::{self, Read, Write};
use std::process::{Command, Stdio};

/// Ctrl-]: the next byte is a harness command (unmeasured setup), not a key.
pub const COMMAND: u8 = 0x1d;
/// Re-reads the terminal size, paints the first page and puts an empty prompt
/// on the bottom row.
pub const RESET: u8 = b'R';
/// Replaces the line with [`FILL_TEXT`] and puts the cursor at its end.
pub const FILL: u8 = b'F';

/// What [`FILL`] types: long enough for 20 backspaces or cursor moves.
pub const FILL_TEXT: &[u8] = b"git log --oneline --graph --decorate --all --max-count=40";

/// The prompt's width: `~/wt 0000$ ` (the number counts lines entered).
const PROMPT_WIDTH: usize = 11;

pub struct LineEditor {
    rows: usize,
    cols: usize,
    line: Vec<u8>,
    /// The cursor's position in `line`.
    cursor: usize,
    /// How many rows the cursor has moved up from the prompt line.
    raised: usize,
    page: usize,
    lines_entered: usize,
    /// The start of an escape sequence split across two reads.
    pending: Vec<u8>,
}

impl Default for LineEditor {
    fn default() -> Self {
        Self {
            rows: 24,
            cols: 80,
            line: Vec::new(),
            cursor: 0,
            raised: 0,
            page: 0,
            lines_entered: 0,
            pending: Vec::new(),
        }
    }
}

/// One decoded piece of terminal input.
#[derive(Debug, PartialEq, Eq)]
enum Input {
    Char(u8),
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    PageDown,
    Command(u8),
    Ignored,
}

/// Decodes the first input in `bytes`, returning it with the number of bytes
/// it used, or `None` when `bytes` ends partway through a sequence.
fn decode(bytes: &[u8]) -> Option<(Input, usize)> {
    // Slice patterns match on the shape of the byte slice, like destructuring
    // an array in JavaScript; `..` matches whatever follows.
    Some(match bytes {
        [] | [COMMAND] | [0x1b] | [0x1b, b'O'] => return None,
        [COMMAND, command, ..] => (Input::Command(*command), 2),
        // CSI (`ESC [`): parameter bytes, then one final byte in `@`..=`~`.
        [0x1b, b'[', rest @ ..] => {
            let end = rest.iter().position(|byte| (0x40..=0x7e).contains(byte))?;
            let input = match &rest[..=end] {
                b"A" => Input::Up,
                b"B" => Input::Down,
                b"C" => Input::Right,
                b"D" => Input::Left,
                b"6~" => Input::PageDown,
                _ => Input::Ignored,
            };
            (input, 2 + end + 1)
        }
        // SS3 (`ESC O`): arrow keys in application cursor mode.
        [0x1b, b'O', last, ..] => (
            match last {
                b'A' => Input::Up,
                b'B' => Input::Down,
                b'C' => Input::Right,
                b'D' => Input::Left,
                _ => Input::Ignored,
            },
            3,
        ),
        // Alt+key and the like: drop the ESC and read on.
        [0x1b, ..] => (Input::Ignored, 1),
        [b'\r', ..] => (Input::Enter, 1),
        [0x7f | 0x08, ..] => (Input::Backspace, 1),
        [byte @ 0x20..=0x7e, ..] => (Input::Char(*byte), 1),
        [_, ..] => (Input::Ignored, 1),
    })
}

impl LineEditor {
    /// Handles `input` from the terminal and returns the bytes to send back.
    /// `size` reads the terminal's (rows, columns), for [`RESET`].
    pub fn feed(
        &mut self,
        input: &[u8],
        size: &mut dyn FnMut() -> Option<(usize, usize)>,
    ) -> Vec<u8> {
        self.pending.extend_from_slice(input);
        let mut out = Vec::new();
        while let Some((input, used)) = decode(&self.pending) {
            self.pending.drain(..used);
            self.handle(input, &mut out, size);
        }
        out
    }

    fn handle(
        &mut self,
        input: Input,
        out: &mut Vec<u8>,
        size: &mut dyn FnMut() -> Option<(usize, usize)>,
    ) {
        if matches!(
            input,
            Input::Char(_) | Input::Backspace | Input::Enter | Input::Command(FILL)
        ) {
            self.lower(out);
        }
        match input {
            Input::Char(byte) => self.insert(byte, out),
            Input::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.line.remove(self.cursor);
                out.push(0x08);
                out.extend_from_slice(&self.line[self.cursor..]);
                out.push(b' ');
                move_left(out, self.line.len() - self.cursor + 1);
            }
            Input::Enter => {
                self.lines_entered += 1;
                self.line.clear();
                self.cursor = 0;
                // On the bottom row, the newline scrolls the whole screen up.
                out.extend_from_slice(b"\r\n");
                self.prompt(out);
            }
            Input::Left if self.cursor > 0 => {
                self.cursor -= 1;
                out.extend_from_slice(b"\x1b[D");
            }
            Input::Right if self.cursor < self.line.len() => {
                self.cursor += 1;
                out.extend_from_slice(b"\x1b[C");
            }
            Input::Up if self.raised + 1 < self.rows => {
                self.raised += 1;
                out.extend_from_slice(b"\x1b[A");
            }
            Input::Down if self.raised > 0 => {
                self.raised -= 1;
                out.extend_from_slice(b"\x1b[B");
            }
            Input::PageDown => {
                self.page += 1;
                // Save the cursor, repaint every row above the prompt, restore it.
                out.extend_from_slice(b"\x1b7");
                self.paint_page(out);
                out.extend_from_slice(b"\x1b8");
            }
            Input::Command(RESET) => self.reset(out, size),
            Input::Command(FILL) => {
                let room = self.cols.saturating_sub(PROMPT_WIDTH + 2);
                self.line = FILL_TEXT[..FILL_TEXT.len().min(room)].to_vec();
                self.cursor = self.line.len();
                out.extend_from_slice(b"\r\x1b[K");
                self.prompt(out);
                out.extend_from_slice(&self.line);
            }
            // Keys with nothing to do (Left at the start of the line, …).
            _ => {}
        }
    }

    fn insert(&mut self, byte: u8, out: &mut Vec<u8>) {
        if PROMPT_WIDTH + self.line.len() + 2 > self.cols {
            // The line is full: start an empty one in its place.
            self.line.clear();
            self.cursor = 0;
            out.extend_from_slice(b"\r\x1b[K");
            self.prompt(out);
        }
        self.line.insert(self.cursor, byte);
        self.cursor += 1;
        out.extend_from_slice(&self.line[self.cursor - 1..]);
        move_left(out, self.line.len() - self.cursor);
    }

    /// Moves the cursor back down to the prompt line before an edit.
    fn lower(&mut self, out: &mut Vec<u8>) {
        if self.raised > 0 {
            out.extend_from_slice(format!("\x1b[{}B", self.raised).as_bytes());
            self.raised = 0;
        }
    }

    fn reset(&mut self, out: &mut Vec<u8>, size: &mut dyn FnMut() -> Option<(usize, usize)>) {
        if let Some((rows, cols)) = size() {
            self.rows = rows.max(2);
            self.cols = cols.max(PROMPT_WIDTH + 4);
        }
        self.line.clear();
        self.cursor = 0;
        self.raised = 0;
        self.page = 0;
        // The alternate screen keeps no scrollback, like the tmux client the
        // real app runs, so scrolling never grows the grid being copied.
        out.extend_from_slice(b"\x1b[?1049h\x1b[0m\x1b[H\x1b[2J");
        self.paint_page(out);
        out.extend_from_slice(format!("\x1b[{};1H", self.rows).as_bytes());
        self.prompt(out);
    }

    fn prompt(&self, out: &mut Vec<u8>) {
        let number = self.lines_entered % 10_000;
        out.extend_from_slice(format!("\x1b[1;35m~/wt {number:04}$\x1b[0m ").as_bytes());
    }

    /// Paints every row above the prompt with this page's text.
    fn paint_page(&self, out: &mut Vec<u8>) {
        for row in 0..self.rows - 1 {
            out.extend_from_slice(format!("\x1b[{};1H", row + 1).as_bytes());
            page_line(self.page, row, self.cols, out);
        }
    }
}

/// One row of a page: syntax-highlighted code with a line number, and on some
/// rows a full-width background like an editor's current-line highlight, so
/// drawing exercises colored text and background runs.
fn page_line(page: usize, row: usize, cols: usize, out: &mut Vec<u8>) {
    let number = format!("{:>4} ", (page * 100 + row) % 10_000);
    let name = format!("handle_{page}_{row}");
    let comment = format!("// page {page}, row {row}");
    let spans: [(&str, &str); 11] = [
        ("38;5;242", &number),
        ("35", "fn "),
        ("32", &name),
        ("0", "("),
        ("33", "event"),
        ("0", ": &"),
        ("36", "Event"),
        ("0", ") -> "),
        ("36", "Result"),
        ("0", "<(), Error> { "),
        ("38;5;244", &comment),
    ];
    let highlighted = row % 7 == 3;
    if highlighted {
        out.extend_from_slice(b"\x1b[48;5;237m");
    }
    let mut room = cols.saturating_sub(1);
    for (color, text) in spans {
        if room == 0 {
            break;
        }
        let shown = &text[..text.len().min(room)];
        room -= shown.len();
        out.extend_from_slice(format!("\x1b[{color}m{shown}").as_bytes());
        if highlighted {
            // SGR 0 above resets the background too; restore it.
            out.extend_from_slice(b"\x1b[48;5;237m");
        }
    }
    // Erase to the end of the row (in the highlight color, if any).
    out.extend_from_slice(b"\x1b[K\x1b[0m");
}

fn move_left(out: &mut Vec<u8>, columns: usize) {
    if columns > 0 {
        out.extend_from_slice(format!("\x1b[{columns}D").as_bytes());
    }
}

/// Reads the terminal size with `stty size` (no libc needed for the ioctl).
fn terminal_size() -> Option<(usize, usize)> {
    // `output()` would give stty a closed stdin; it needs the terminal.
    let output = Command::new("stty")
        .arg("size")
        .stdin(Stdio::inherit())
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let mut numbers = text.split_whitespace().map(str::parse);
    Some((numbers.next()?.ok()?, numbers.next()?.ok()?))
}

/// The terminal program's `main`: answers keystrokes until the PTY closes.
/// Returns the process exit code.
pub fn run() -> i32 {
    // Raw mode, as any full-screen program uses: each key arrives at once,
    // unechoed and untranslated. stty acts on its stdin, our terminal.
    match Command::new("stty").args(["raw", "-echo"]).status() {
        Ok(status) if status.success() => {}
        _ => {
            eprintln!("latency echo: could not put the terminal in raw mode");
            return 1;
        }
    }
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    // The harness waits for this before it sends anything.
    if stdout
        .write_all(b"\x1b[?1049h\x1b[H\x1b[2Jready")
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return 1;
    }
    let mut editor = LineEditor::default();
    let mut buffer = [0; 4096];
    loop {
        let read = match stdin.read(&mut buffer) {
            Ok(0) | Err(_) => return 0,
            Ok(read) => read,
        };
        let reply = editor.feed(&buffer[..read], &mut terminal_size);
        // Write and flush once per read, as a real program does per keystroke.
        if stdout
            .write_all(&reply)
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(rows: usize, cols: usize) -> LineEditor {
        let mut editor = LineEditor::default();
        editor.feed(&[COMMAND, RESET], &mut || Some((rows, cols)));
        editor
    }

    fn feed(editor: &mut LineEditor, input: &[u8]) -> Vec<u8> {
        editor.feed(input, &mut || None)
    }

    #[test]
    fn typed_characters_echo_and_insert_at_the_cursor() {
        let mut editor = editor(24, 80);
        assert_eq!(feed(&mut editor, b"a"), b"a");
        assert_eq!(feed(&mut editor, b"c"), b"c");
        assert_eq!(feed(&mut editor, b"\x1b[D"), b"\x1b[D");
        // Inserting before "c" rewrites it and steps back over it.
        assert_eq!(feed(&mut editor, b"b"), b"bc\x1b[1D");
        assert_eq!(editor.line, b"abc");
        assert_eq!(feed(&mut editor, b"\x7f"), b"\x08c \x1b[2D");
        assert_eq!(editor.line, b"ac");
    }

    #[test]
    fn cursor_keys_move_only_within_the_line_and_the_screen() {
        let mut editor = editor(3, 80);
        assert!(feed(&mut editor, b"\x1b[D").is_empty());
        assert!(feed(&mut editor, b"\x1b[C").is_empty());
        assert!(feed(&mut editor, b"\x1b[B").is_empty());
        assert_eq!(feed(&mut editor, b"\x1b[A"), b"\x1b[A");
        assert_eq!(feed(&mut editor, b"\x1bOA"), b"\x1b[A");
        assert!(
            feed(&mut editor, b"\x1b[A").is_empty(),
            "already on the top row"
        );
        // Typing returns to the prompt line first.
        assert_eq!(feed(&mut editor, b"x"), b"\x1b[2Bx");
    }

    #[test]
    fn escape_sequences_split_across_reads_are_decoded_once_complete() {
        let mut editor = editor(24, 80);
        feed(&mut editor, b"ab");
        assert!(feed(&mut editor, b"\x1b").is_empty());
        assert!(feed(&mut editor, b"[").is_empty());
        assert_eq!(feed(&mut editor, b"D"), b"\x1b[D");
        assert!(feed(&mut editor, b"\x1b[6").is_empty());
        assert!(!feed(&mut editor, b"~").is_empty());
        // An unknown sequence (PageUp) is skipped whole, never typed.
        assert!(feed(&mut editor, b"\x1b[5~").is_empty());
        assert_eq!(editor.line, b"ab");
    }

    #[test]
    fn reset_reads_the_size_and_puts_the_prompt_on_the_bottom_row() {
        let mut editor = LineEditor::default();
        let out = editor.feed(&[COMMAND, RESET], &mut || Some((30, 100)));
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("\x1b[?1049h"), "uses the alternate screen");
        assert!(text.ends_with("\x1b[30;1H\x1b[1;35m~/wt 0000$\x1b[0m "));
        assert_eq!((editor.rows, editor.cols), (30, 100));
    }

    #[test]
    fn a_full_line_starts_over_instead_of_wrapping() {
        let mut editor = editor(24, PROMPT_WIDTH + 5);
        // Room for 4 characters, keeping the last column free.
        assert_eq!(feed(&mut editor, b"abcd"), b"abcd");
        let out = String::from_utf8_lossy(&feed(&mut editor, b"e")).into_owned();
        assert!(out.starts_with("\r\x1b[K"), "{out:?}");
        assert_eq!(editor.line, b"e");
    }
}
