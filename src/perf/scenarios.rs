//! What the latency harness types. Each scenario repeats a cycle of steps:
//! measured keystrokes, plus unmeasured setup that keeps every measured key
//! visible (refilling a line before deleting from it, and so on).

use super::echo;
use iced::keyboard::{self, Key, key::Named};

/// A key the harness presses, as macOS would deliver it to iced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keystroke {
    Char(char),
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    PageDown,
}

impl Keystroke {
    pub fn event(self) -> iced::Event {
        let (key, text) = match self {
            Self::Char(c) => (Key::Character(c.to_string().into()), Some(c.to_string())),
            Self::Backspace => (Key::Named(Named::Backspace), None),
            Self::Enter => (Key::Named(Named::Enter), Some("\r".to_owned())),
            Self::Left => (Key::Named(Named::ArrowLeft), None),
            Self::Right => (Key::Named(Named::ArrowRight), None),
            Self::Up => (Key::Named(Named::ArrowUp), None),
            Self::Down => (Key::Named(Named::ArrowDown), None),
            Self::PageDown => (Key::Named(Named::PageDown), None),
        };
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: text.map(Into::into),
            repeat: false,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A keystroke whose key-to-pixels latency is recorded.
    Key(Keystroke),
    /// Unmeasured: a command for the terminal program (`echo::RESET`, …).
    Terminal(u8),
    /// Unmeasured: opens the worktree's Rename field and focuses it.
    OpenRename,
    /// Unmeasured: empties the Rename field.
    ClearRename,
}

pub struct Scenario {
    pub name: &'static str,
    pub description: &'static str,
    pub setup: Vec<Step>,
    pub cycle: Vec<Step>,
}

fn keys(keys: impl IntoIterator<Item = Keystroke>) -> impl Iterator<Item = Step> {
    keys.into_iter().map(Step::Key)
}

fn letters(count: usize) -> impl Iterator<Item = Keystroke> {
    ('a'..='z').cycle().take(count).map(Keystroke::Char)
}

pub fn all() -> Vec<Scenario> {
    use Keystroke::*;
    let reset = vec![Step::Terminal(echo::RESET)];
    let fill = Step::Terminal(echo::FILL);
    vec![
        Scenario {
            name: "type_character",
            description: "Type a letter at the terminal prompt until it appears",
            setup: reset.clone(),
            cycle: keys(letters(26)).collect(),
        },
        Scenario {
            name: "backspace",
            description: "Delete the character before the cursor",
            setup: reset.clone(),
            cycle: std::iter::once(fill).chain(keys([Backspace; 20])).collect(),
        },
        Scenario {
            name: "cursor_horizontal",
            description: "Move the cursor left and right along a typed line",
            setup: reset.clone(),
            cycle: std::iter::once(fill)
                .chain(keys([Left; 20]))
                .chain(keys([Right; 20]))
                .collect(),
        },
        Scenario {
            name: "cursor_vertical",
            description: "Move the cursor up and down over a full screen of text",
            setup: reset.clone(),
            cycle: keys([Up; 10]).chain(keys([Down; 10])).collect(),
        },
        Scenario {
            name: "newline_scroll",
            description: "Press Return on the bottom row, scrolling the whole screen",
            setup: reset.clone(),
            cycle: vec![Step::Key(Enter)],
        },
        Scenario {
            name: "page_redraw",
            description: "Page down, repainting every row (colored text and backgrounds)",
            setup: reset,
            cycle: vec![Step::Key(PageDown)],
        },
        Scenario {
            name: "rename_field_typing",
            description: "Type into the worktree Rename field in the header (app UI, no terminal)",
            setup: vec![Step::OpenRename],
            cycle: std::iter::once(Step::ClearRename)
                .chain(keys(letters(20)))
                .collect(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::echo::{COMMAND, LineEditor};

    /// The bytes iced_term sends for a key (its default bindings).
    fn bytes(key: Keystroke) -> Vec<u8> {
        match key {
            Keystroke::Char(c) => c.to_string().into_bytes(),
            Keystroke::Backspace => b"\x7f".to_vec(),
            Keystroke::Enter => b"\r".to_vec(),
            Keystroke::Left => b"\x1b[D".to_vec(),
            Keystroke::Right => b"\x1b[C".to_vec(),
            Keystroke::Up => b"\x1b[A".to_vec(),
            Keystroke::Down => b"\x1b[B".to_vec(),
            Keystroke::PageDown => b"\x1b[6~".to_vec(),
        }
    }

    // A measured key that changes nothing on screen would stall the harness
    // until its timeout, so every one must make the terminal program answer,
    // at the smallest window size (820×440 is about 20 rows of 60 columns).
    #[test]
    fn every_measured_terminal_key_changes_the_screen() {
        for scenario in all().iter().filter(|s| s.name != "rename_field_typing") {
            let mut editor = LineEditor::default();
            let mut size = || Some((20, 60));
            let steps = scenario
                .setup
                .iter()
                .chain(scenario.cycle.iter().cycle().take(200));
            for step in steps {
                match *step {
                    Step::Key(key) => assert!(
                        !editor.feed(&bytes(key), &mut size).is_empty(),
                        "{}: {key:?} changed nothing",
                        scenario.name
                    ),
                    Step::Terminal(command) => {
                        editor.feed(&[COMMAND, command], &mut size);
                    }
                    Step::OpenRename | Step::ClearRename => {
                        panic!("{}: not a terminal step", scenario.name)
                    }
                }
            }
        }
    }

    #[test]
    fn scenario_names_are_unique_identifiers() {
        let names: Vec<_> = all().iter().map(|s| s.name).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(names.len(), unique.len());
        assert!(
            names
                .iter()
                .all(|n| n.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        );
    }
}
