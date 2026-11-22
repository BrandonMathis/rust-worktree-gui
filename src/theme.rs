//! The app's color palette: the eleven colors of the Dracula specification,
//! the built-in presets, hex and HSV conversion, and the saved `theme.json`.
//!
//! The palette only themes the app's own chrome. The embedded terminal keeps
//! the colors from the user's Alacritty config (see `terminal_config.rs`).

use crate::config;
use iced::Color;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::PathBuf;

/// One named color of the palette, in the order the Dracula spec lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Background,
    Selection,
    Foreground,
    Comment,
    Cyan,
    Green,
    Orange,
    Pink,
    Purple,
    Red,
    Yellow,
}

impl Slot {
    pub const ALL: [Self; 11] = [
        Self::Background,
        Self::Selection,
        Self::Foreground,
        Self::Comment,
        Self::Cyan,
        Self::Green,
        Self::Orange,
        Self::Pink,
        Self::Purple,
        Self::Red,
        Self::Yellow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Background => "Background",
            Self::Selection => "Current Line",
            Self::Foreground => "Foreground",
            Self::Comment => "Comment",
            Self::Cyan => "Cyan",
            Self::Green => "Green",
            Self::Orange => "Orange",
            Self::Pink => "Pink",
            Self::Purple => "Purple",
            Self::Red => "Red",
            Self::Yellow => "Yellow",
        }
    }

    /// The key used in `theme.json`.
    fn key(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Selection => "selection",
            Self::Foreground => "foreground",
            Self::Comment => "comment",
            Self::Cyan => "cyan",
            Self::Green => "green",
            Self::Orange => "orange",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Red => "red",
            Self::Yellow => "yellow",
        }
    }
}

/// The colors every view draws with. It is `Copy` (eleven small floats), so
/// views pass it by value into style closures without borrowing the app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub selection: Color,
    pub foreground: Color,
    pub comment: Color,
    pub cyan: Color,
    pub green: Color,
    pub orange: Color,
    pub pink: Color,
    pub purple: Color,
    pub red: Color,
    pub yellow: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Preset::Dracula.palette()
    }
}

impl Palette {
    pub fn get(&self, slot: Slot) -> Color {
        // Reborrowing through `slot_ref` keeps the slot-to-field mapping in one place.
        *self.slot_ref(slot)
    }

    pub fn set(&mut self, slot: Slot, color: Color) {
        *self.slot_mut(slot) = color;
    }

    fn slot_ref(&self, slot: Slot) -> &Color {
        match slot {
            Slot::Background => &self.background,
            Slot::Selection => &self.selection,
            Slot::Foreground => &self.foreground,
            Slot::Comment => &self.comment,
            Slot::Cyan => &self.cyan,
            Slot::Green => &self.green,
            Slot::Orange => &self.orange,
            Slot::Pink => &self.pink,
            Slot::Purple => &self.purple,
            Slot::Red => &self.red,
            Slot::Yellow => &self.yellow,
        }
    }

    fn slot_mut(&mut self, slot: Slot) -> &mut Color {
        match slot {
            Slot::Background => &mut self.background,
            Slot::Selection => &mut self.selection,
            Slot::Foreground => &mut self.foreground,
            Slot::Comment => &mut self.comment,
            Slot::Cyan => &mut self.cyan,
            Slot::Green => &mut self.green,
            Slot::Orange => &mut self.orange,
            Slot::Pink => &mut self.pink,
            Slot::Purple => &mut self.purple,
            Slot::Red => &mut self.red,
            Slot::Yellow => &mut self.yellow,
        }
    }

    /// True for dark themes; light themes (such as Alucard) shade less.
    pub fn is_dark(&self) -> bool {
        let Color { r, g, b, .. } = self.background;
        0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5
    }

    /// Secondary text: between the foreground and the comment color, so it
    /// stays readable on glass panels.
    pub fn muted(&self) -> Color {
        mix(self.foreground, self.comment, 0.45)
    }

    /// Hairline borders around panels and controls.
    pub fn edge(&self) -> Color {
        mix(self.purple, self.foreground, 0.35).scale_alpha(0.22)
    }

    /// The translucent window base. macOS blur shows through it.
    pub fn window(&self) -> Color {
        self.deepen(self.background, 0.4).scale_alpha(0.91)
    }

    /// Top and bottom stops of the glass panel gradient.
    pub fn glass(&self) -> (Color, Color) {
        (
            mix(self.background, self.selection, 0.7).scale_alpha(0.72),
            self.deepen(self.background, 0.15).scale_alpha(0.85),
        )
    }

    /// An opaque panel that floats above other content (sidebar preview).
    pub fn floating(&self) -> Color {
        mix(self.background, self.selection, 0.2)
    }

    /// The iced theme that built-in widgets (text inputs, pick lists,
    /// scrollbars, tooltips) derive their colors from.
    pub fn iced_theme(&self) -> iced::Theme {
        iced::Theme::custom(
            "Worktree Browser",
            iced::theme::Palette {
                background: self.background,
                text: self.foreground,
                primary: self.purple,
                success: self.green,
                warning: self.yellow,
                danger: self.red,
            },
        )
    }

    /// Darkens toward black. Light themes darken a quarter as much, so they
    /// don't turn grey.
    fn deepen(&self, color: Color, amount: f32) -> Color {
        let amount = if self.is_dark() { amount } else { amount / 4.0 };
        mix(color, Color::BLACK, amount)
    }
}

/// Linear blend: `amount` 0.0 is `from`, 1.0 is `to`.
fn mix(from: Color, to: Color, amount: f32) -> Color {
    let lerp = |a: f32, b: f32| a + (b - a) * amount;
    Color {
        r: lerp(from.r, to.r),
        g: lerp(from.g, to.g),
        b: lerp(from.b, to.b),
        a: lerp(from.a, to.a),
    }
}

/// Built-in palettes: Dracula, its official light theme Alucard, and the
/// Dracula Pro family. Pro variants share Pro's accents and differ in their
/// background, current-line and comment colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Dracula,
    Alucard,
    Pro,
    Blade,
    Buffy,
    Lincoln,
    Morbius,
    VanHelsing,
}

impl Preset {
    pub const ALL: [Self; 8] = [
        Self::Dracula,
        Self::Alucard,
        Self::Pro,
        Self::Blade,
        Self::Buffy,
        Self::Lincoln,
        Self::Morbius,
        Self::VanHelsing,
    ];

    pub fn palette(self) -> Palette {
        match self {
            Self::Dracula => Palette {
                background: rgb(0x282A36),
                selection: rgb(0x44475A),
                foreground: rgb(0xF8F8F2),
                comment: rgb(0x6272A4),
                cyan: rgb(0x8BE9FD),
                green: rgb(0x50FA7B),
                orange: rgb(0xFFB86C),
                pink: rgb(0xFF79C6),
                purple: rgb(0xBD93F9),
                red: rgb(0xFF5555),
                yellow: rgb(0xF1FA8C),
            },
            Self::Alucard => Palette {
                background: rgb(0xFFFBEB),
                selection: rgb(0xCFCFDE),
                foreground: rgb(0x1F1F1F),
                comment: rgb(0x6C664B),
                cyan: rgb(0x036A96),
                green: rgb(0x14710A),
                orange: rgb(0xA34D14),
                pink: rgb(0xA3144D),
                purple: rgb(0x644AC9),
                red: rgb(0xCB3A2A),
                yellow: rgb(0x846E15),
            },
            Self::Pro => pro(0x22212C, 0x454158, 0x7970A9),
            Self::Blade => pro(0x212C2A, 0x415854, 0x70A99F),
            Self::Buffy => pro(0x2A212C, 0x544158, 0x9F70A9),
            Self::Lincoln => pro(0x2C2A21, 0x585441, 0xA99F70),
            Self::Morbius => pro(0x2C2121, 0x584141, 0xA97079),
            Self::VanHelsing => pro(0x0B0D0F, 0x414D58, 0x708CA9),
        }
    }

    /// The preset whose colors match `palette` exactly, if any.
    pub fn matching(palette: &Palette) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|preset| same_colors(&preset.palette(), palette))
    }
}

impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Dracula => "Dracula",
            Self::Alucard => "Alucard (light)",
            Self::Pro => "Dracula Pro",
            Self::Blade => "Dracula Pro Blade",
            Self::Buffy => "Dracula Pro Buffy",
            Self::Lincoln => "Dracula Pro Lincoln",
            Self::Morbius => "Dracula Pro Morbius",
            Self::VanHelsing => "Dracula Pro Van Helsing",
        })
    }
}

fn pro(background: u32, selection: u32, comment: u32) -> Palette {
    Palette {
        background: rgb(background),
        selection: rgb(selection),
        foreground: rgb(0xF8F8F2),
        comment: rgb(comment),
        cyan: rgb(0x80FFEA),
        green: rgb(0x8AFF80),
        orange: rgb(0xFFCA80),
        pink: rgb(0xFF80BF),
        purple: rgb(0x9580FF),
        red: rgb(0xFF9580),
        yellow: rgb(0xFFFF80),
    }
}

/// Compares at 8-bit precision, the precision the hex inputs and file use.
fn same_colors(a: &Palette, b: &Palette) -> bool {
    Slot::ALL
        .into_iter()
        .all(|slot| a.get(slot).into_rgba8() == b.get(slot).into_rgba8())
}

/// `0xRRGGBB` to a color.
fn rgb(hex: u32) -> Color {
    // `to_be_bytes` splits the number into bytes, most significant first.
    let [_, r, g, b] = hex.to_be_bytes();
    Color::from_rgb8(r, g, b)
}

/// Parses `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB` (any case, surrounding spaces allowed).
pub fn parse_hex(input: &str) -> Option<Color> {
    let digits = input.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let expanded: String = match digits.len() {
        6 => digits.to_owned(),
        // `#abc` is shorthand for `#aabbcc`.
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        _ => return None,
    };
    u32::from_str_radix(&expanded, 16).ok().map(rgb)
}

/// Formats a color as `#RRGGBB`, ignoring alpha.
pub fn to_hex(color: Color) -> String {
    let [r, g, b, _] = color.into_rgba8();
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// A color as hue (degrees, 0..360), saturation and value (both 0..=1). The
/// color wheel edits hue and saturation; the brightness slider edits value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
}

impl Hsv {
    pub fn from_color(color: Color) -> Self {
        let Color { r, g, b, .. } = color;
        let max = r.max(g).max(b);
        let delta = max - r.min(g).min(b);
        let hue = if delta <= f32::EPSILON {
            0.0
        } else if max == r {
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        Self {
            hue,
            saturation: if max <= f32::EPSILON {
                0.0
            } else {
                delta / max
            },
            value: max,
        }
    }

    /// Converts to a color rounded to 8 bits per channel, so the result is
    /// exactly what its hex code says.
    pub fn to_color(self) -> Color {
        let value = self.value.clamp(0.0, 1.0);
        let chroma = value * self.saturation.clamp(0.0, 1.0);
        let sector = self.hue.rem_euclid(360.0) / 60.0;
        let x = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
        let (r, g, b) = match sector as u8 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };
        let offset = value - chroma;
        // The value is clamped to 0..=255 first, so `as u8` cannot wrap.
        let byte = |channel: f32| ((channel + offset) * 255.0).round().clamp(0.0, 255.0) as u8;
        Color::from_rgb8(byte(r), byte(g), byte(b))
    }
}

/// The saved theme, `theme.json`, next to the bookmarks file.
pub struct Store {
    path: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct ThemeFile {
    colors: BTreeMap<String, String>,
}

impl Store {
    pub fn open() -> Result<Self, String> {
        Ok(Self::at(config::support_dir()?.join("theme.json")))
    }

    pub(crate) fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Reads the saved palette. A missing file means the default (Dracula).
    /// Colors missing from the file also fall back to Dracula, so files from
    /// older versions keep working if new colors are added.
    pub fn load(&self) -> Result<Palette, String> {
        let error =
            |err: &dyn std::fmt::Display| format!("Could not read {}: {err}", self.path.display());
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Palette::default()),
            Err(err) => return Err(error(&err)),
        };
        let saved: ThemeFile =
            serde_json::from_reader(BufReader::new(file)).map_err(|err| error(&err))?;
        let mut palette = Palette::default();
        for slot in Slot::ALL {
            if let Some(hex) = saved.colors.get(slot.key()) {
                let color = parse_hex(hex)
                    .ok_or_else(|| error(&format!("{} is not a hex color: {hex}", slot.key())))?;
                palette.set(slot, color);
            }
        }
        Ok(palette)
    }

    pub fn save(&self, palette: &Palette) -> Result<(), String> {
        let colors = Slot::ALL
            .into_iter()
            .map(|slot| (slot.key().to_owned(), to_hex(palette.get(slot))))
            .collect();
        config::write_json(&self.path, &ThemeFile { colors })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_codes_parse_in_long_and_short_forms_and_reject_anything_else() {
        assert_eq!(parse_hex("#BD93F9"), Some(rgb(0xBD93F9)));
        assert_eq!(parse_hex(" bd93f9 "), Some(rgb(0xBD93F9)));
        assert_eq!(parse_hex("#abc"), Some(rgb(0xAABBCC)));
        for invalid in [
            "", "#", "#12345", "#1234567", "#GG0000", "+12345", "#-12345",
        ] {
            assert_eq!(parse_hex(invalid), None, "{invalid:?}");
        }
        assert_eq!(to_hex(rgb(0x0B0D0F)), "#0B0D0F");
    }

    #[test]
    fn hsv_round_trips_every_preset_color_exactly() {
        for preset in Preset::ALL {
            let palette = preset.palette();
            for slot in Slot::ALL {
                let color = palette.get(slot);
                assert_eq!(
                    to_hex(Hsv::from_color(color).to_color()),
                    to_hex(color),
                    "{preset} {slot:?}"
                );
            }
        }
        let red = Hsv {
            hue: 360.0,
            saturation: 1.0,
            value: 1.0,
        };
        assert_eq!(to_hex(red.to_color()), "#FF0000");
    }

    #[test]
    fn presets_are_recognized_only_when_every_color_matches() {
        for preset in Preset::ALL {
            assert_eq!(Preset::matching(&preset.palette()), Some(preset));
        }
        let mut edited = Preset::Blade.palette();
        edited.set(Slot::Pink, rgb(0x123456));
        assert_eq!(Preset::matching(&edited), None);
        assert!(Preset::Dracula.palette().is_dark());
        assert!(!Preset::Alucard.palette().is_dark());
    }

    #[test]
    fn saved_theme_round_trips_and_missing_file_means_dracula() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().join("nested dir/theme.json"));
        assert_eq!(store.load().unwrap(), Palette::default());

        let palette = Preset::Morbius.palette();
        store.save(&palette).unwrap();
        assert_eq!(store.load().unwrap(), palette);
    }

    #[test]
    fn theme_file_with_a_bad_color_is_an_error_but_missing_colors_use_dracula() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().join("theme.json"));
        std::fs::write(&store.path, r##"{"colors":{"red":"#010203"}}"##).unwrap();
        let palette = store.load().unwrap();
        assert_eq!(palette.red, rgb(0x010203));
        assert_eq!(palette.background, Palette::default().background);

        std::fs::write(&store.path, r#"{"colors":{"red":"nope"}}"#).unwrap();
        assert!(store.load().unwrap_err().contains("red is not a hex color"));
        std::fs::write(&store.path, "not JSON").unwrap();
        assert!(store.load().unwrap_err().contains("Could not read"));
    }
}
