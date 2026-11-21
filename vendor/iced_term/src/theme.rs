use crate::settings::ThemeSettings;
use alacritty_terminal::vte::ansi::{self, NamedColor};
use iced::{widget::container, Color};

pub(crate) trait TerminalStyle {
    fn container_style(&self) -> container::Style;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorColor {
    CellForeground,
    CellBackground,
    Fixed(Color),
}

impl CursorColor {
    fn resolve(self, fg: Color, bg: Color) -> Color {
        let color = match self {
            Self::CellForeground => fg,
            Self::CellBackground => bg,
            Self::Fixed(color) => color,
        };
        Color { a: 1.0, ..color }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorColors {
    pub cursor: CursorColor,
    pub text: CursorColor,
}

impl Default for CursorColors {
    fn default() -> Self {
        Self {
            cursor: CursorColor::CellForeground,
            text: CursorColor::CellBackground,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColorPalette {
    pub foreground: String,
    pub background: String,
    pub cursor: CursorColors,
    pub vi_mode_cursor: CursorColors,
    pub black: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub magenta: String,
    pub cyan: String,
    pub white: String,
    pub bright_black: String,
    pub bright_red: String,
    pub bright_green: String,
    pub bright_yellow: String,
    pub bright_blue: String,
    pub bright_magenta: String,
    pub bright_cyan: String,
    pub bright_white: String,
    pub bright_foreground: Option<String>,
    pub dim_foreground: String,
    pub dim_black: String,
    pub dim_red: String,
    pub dim_green: String,
    pub dim_yellow: String,
    pub dim_blue: String,
    pub dim_magenta: String,
    pub dim_cyan: String,
    pub dim_white: String,
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self {
            foreground: String::from("#d8d8d8"),
            background: String::from("#181818"),
            cursor: CursorColors::default(),
            vi_mode_cursor: CursorColors::default(),
            black: String::from("#181818"),
            red: String::from("#ac4242"),
            green: String::from("#90a959"),
            yellow: String::from("#f4bf75"),
            blue: String::from("#6a9fb5"),
            magenta: String::from("#aa759f"),
            cyan: String::from("#75b5aa"),
            white: String::from("#d8d8d8"),
            bright_black: String::from("#6b6b6b"),
            bright_red: String::from("#c55555"),
            bright_green: String::from("#aac474"),
            bright_yellow: String::from("#feca88"),
            bright_blue: String::from("#82b8c8"),
            bright_magenta: String::from("#c28cb8"),
            bright_cyan: String::from("#93d3c3"),
            bright_white: String::from("#f8f8f8"),
            bright_foreground: None,
            dim_foreground: String::from("#828482"),
            dim_black: String::from("#0f0f0f"),
            dim_red: String::from("#712b2b"),
            dim_green: String::from("#5f6f3a"),
            dim_yellow: String::from("#a17e4d"),
            dim_blue: String::from("#456877"),
            dim_magenta: String::from("#704d68"),
            dim_cyan: String::from("#4d7770"),
            dim_white: String::from("#8e8e8e"),
        }
    }
}

/// Every `NamedColor` in declaration order. Variants 0..=15 are the ANSI
/// colors and the rest start at 256, so `named_slot` packs them into 29 slots.
const NAMED_COLORS: [NamedColor; 29] = [
    NamedColor::Black,
    NamedColor::Red,
    NamedColor::Green,
    NamedColor::Yellow,
    NamedColor::Blue,
    NamedColor::Magenta,
    NamedColor::Cyan,
    NamedColor::White,
    NamedColor::BrightBlack,
    NamedColor::BrightRed,
    NamedColor::BrightGreen,
    NamedColor::BrightYellow,
    NamedColor::BrightBlue,
    NamedColor::BrightMagenta,
    NamedColor::BrightCyan,
    NamedColor::BrightWhite,
    NamedColor::Foreground,
    NamedColor::Background,
    NamedColor::Cursor,
    NamedColor::DimBlack,
    NamedColor::DimRed,
    NamedColor::DimGreen,
    NamedColor::DimYellow,
    NamedColor::DimBlue,
    NamedColor::DimMagenta,
    NamedColor::DimCyan,
    NamedColor::DimWhite,
    NamedColor::BrightForeground,
    NamedColor::DimForeground,
];

fn named_slot(color: NamedColor) -> usize {
    let index = color as usize;
    if index < 16 {
        index
    } else {
        index - 256 + 16
    }
}

// Patch: the palette's hex strings are parsed once here instead of in
// `get_color`, which runs twice per cell on every frame. Profiling a debug
// build showed hex parsing as the top app-level cost in the draw loop.
#[derive(Debug, Clone)]
pub struct Theme {
    palette: Box<ColorPalette>,
    named: [Color; NAMED_COLORS.len()],
    indexed: [Color; 256],
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_palette(Box::<ColorPalette>::default())
    }
}

impl Theme {
    pub fn new(settings: ThemeSettings) -> Self {
        Self::from_palette(settings.color_pallete)
    }

    fn from_palette(palette: Box<ColorPalette>) -> Self {
        let named = NAMED_COLORS.map(|color| {
            let hex = named_hex(&palette, color);
            hex_to_color(hex)
                .unwrap_or_else(|_| panic!("invalid color {}", hex))
        });
        let mut indexed = build_ansi256_colors();
        indexed[..16].copy_from_slice(&named[..16]);
        Self {
            palette,
            named,
            indexed,
        }
    }

    pub(crate) fn cursor_colors(
        &self,
        fg: Color,
        bg: Color,
        override_color: Option<ansi::Rgb>,
        vi: bool,
    ) -> (Color, Color) {
        let config = if vi {
            self.palette.vi_mode_cursor
        } else {
            self.palette.cursor
        };
        let cursor = override_color.map_or_else(
            || config.cursor.resolve(fg, bg),
            |rgb| Color::from_rgb8(rgb.r, rgb.g, rgb.b),
        );
        let cursor = contrasting(cursor, bg, 3.0);
        let text = contrasting(config.text.resolve(fg, bg), cursor, 4.5);
        (cursor, text)
    }

    pub fn get_color(&self, c: ansi::Color) -> Color {
        match c {
            ansi::Color::Spec(rgb) => Color::from_rgb8(rgb.r, rgb.g, rgb.b),
            ansi::Color::Indexed(index) => self.indexed[usize::from(index)],
            ansi::Color::Named(c) => self.named[named_slot(c)],
        }
    }
}

fn named_hex(palette: &ColorPalette, c: NamedColor) -> &str {
    match c {
        NamedColor::Foreground => &palette.foreground,
        NamedColor::Background => &palette.background,
        // Normal terminal colors
        NamedColor::Black => &palette.black,
        NamedColor::Red => &palette.red,
        NamedColor::Green => &palette.green,
        NamedColor::Yellow => &palette.yellow,
        NamedColor::Blue => &palette.blue,
        NamedColor::Magenta => &palette.magenta,
        NamedColor::Cyan => &palette.cyan,
        NamedColor::White => &palette.white,
        // Bright terminal colors
        NamedColor::BrightBlack => &palette.bright_black,
        NamedColor::BrightRed => &palette.bright_red,
        NamedColor::BrightGreen => &palette.bright_green,
        NamedColor::BrightYellow => &palette.bright_yellow,
        NamedColor::BrightBlue => &palette.bright_blue,
        NamedColor::BrightMagenta => &palette.bright_magenta,
        NamedColor::BrightCyan => &palette.bright_cyan,
        NamedColor::BrightWhite => &palette.bright_white,
        NamedColor::BrightForeground => match &palette.bright_foreground {
            Some(color) => color,
            None => &palette.foreground,
        },
        // Dim terminal colors
        NamedColor::DimForeground => &palette.dim_foreground,
        NamedColor::DimBlack => &palette.dim_black,
        NamedColor::DimRed => &palette.dim_red,
        NamedColor::DimGreen => &palette.dim_green,
        NamedColor::DimYellow => &palette.dim_yellow,
        NamedColor::DimBlue => &palette.dim_blue,
        NamedColor::DimMagenta => &palette.dim_magenta,
        NamedColor::DimCyan => &palette.dim_cyan,
        NamedColor::DimWhite => &palette.dim_white,
        _ => &palette.background,
    }
}

fn contrasting(
    preferred: Color,
    background: Color,
    minimum_ratio: f32,
) -> Color {
    if contrast(preferred, background) >= minimum_ratio {
        preferred
    } else if contrast(Color::WHITE, background)
        >= contrast(Color::BLACK, background)
    {
        Color::WHITE
    } else {
        Color::BLACK
    }
}

fn contrast(a: Color, b: Color) -> f32 {
    let luminance = |color: Color| {
        let [r, g, b, _] = color.into_linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn build_ansi256_colors() -> [Color; 256] {
    // Slots 0..16 are filled from the palette by `Theme::from_palette`.
    let mut ansi256_colors = [Color::BLACK; 256];

    for r in 0..6 {
        for g in 0..6 {
            for b in 0..6 {
                // Reserve the first 16 colors for config.
                let index = 16 + r * 36 + g * 6 + b;
                let color = Color::from_rgb8(
                    if r == 0 { 0 } else { r * 40 + 55 },
                    if g == 0 { 0 } else { g * 40 + 55 },
                    if b == 0 { 0 } else { b * 40 + 55 },
                );
                ansi256_colors[usize::from(index)] = color;
            }
        }
    }

    let index: u8 = 232;
    for i in 0..24 {
        let value = i * 10 + 8;
        ansi256_colors[usize::from(index + i)] =
            Color::from_rgb8(value, value, value);
    }

    ansi256_colors
}

fn hex_to_color(hex: &str) -> anyhow::Result<Color> {
    if hex.len() != 7 {
        return Err(anyhow::format_err!("input string is in non valid format"));
    }

    let r = u8::from_str_radix(&hex[1..3], 16)?;
    let g = u8::from_str_radix(&hex[3..5], 16)?;
    let b = u8::from_str_radix(&hex[5..7], 16)?;

    Ok(Color::from_rgb8(r, g, b))
}

impl TerminalStyle for Theme {
    fn container_style(&self) -> container::Style {
        container::Style {
            background: Some(
                hex_to_color(&self.palette.background)
                    .unwrap_or_else(|_| {
                        panic!(
                            "invalid background color {}",
                            self.palette.background
                        )
                    })
                    .into(),
            ),
            ..container::Style::default()
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn caret_contrast_is_preserved_on_dark_light_and_highlighted_cells() {
        use super::*;
        let light = Color::from_rgb8(248, 248, 242);
        let dark = Color::from_rgb8(40, 42, 54);
        let highlight = Color::from_rgb8(68, 71, 90);
        let palette = ColorPalette {
            cursor: CursorColors {
                cursor: CursorColor::Fixed(light),
                text: CursorColor::Fixed(dark),
            },
            ..ColorPalette::default()
        };
        let theme = Theme::new(ThemeSettings::new(Box::new(palette)));
        assert_eq!(
            theme.cursor_colors(light, dark, None, false),
            (light, dark)
        );
        for color in [dark, light, highlight] {
            let rgb = ansi::Rgb {
                r: (color.r * 255.0).round() as u8,
                g: (color.g * 255.0).round() as u8,
                b: (color.b * 255.0).round() as u8,
            };
            let (cursor, text) =
                theme.cursor_colors(color, color, Some(rgb), false);
            assert!(contrast(cursor, color) >= 3.0);
            assert!(contrast(text, cursor) >= 4.5);
        }
    }

    use super::*;
    use alacritty_terminal::vte::ansi;
    use std::collections::HashMap;

    #[test]
    fn hex_to_color_valid_convertion() {
        assert!(hex_to_color("#000000").is_ok())
    }

    #[test]
    fn hex_to_color_short_string() {
        assert!(hex_to_color("GG").is_err());
    }

    #[test]
    fn hex_to_color_long_string() {
        assert!(hex_to_color("GG000000").is_err());
    }

    #[test]
    fn hex_to_color_non_valid_hex_string() {
        assert!(hex_to_color("#KKLLOO").is_err());
    }

    #[test]
    fn get_basic_indexed_colors() {
        let default_theme = Theme::default();
        let basic_indexed_colors_map: HashMap<u8, String> = HashMap::from([
            (0, default_theme.palette.black.clone()),
            (1, default_theme.palette.red.clone()),
            (2, default_theme.palette.green.clone()),
            (3, default_theme.palette.yellow.clone()),
            (4, default_theme.palette.blue.clone()),
            (5, default_theme.palette.magenta.clone()),
            (6, default_theme.palette.cyan.clone()),
            (7, default_theme.palette.white.clone()),
            (8, default_theme.palette.bright_black.clone()),
            (9, default_theme.palette.bright_red.clone()),
            (10, default_theme.palette.bright_green.clone()),
            (11, default_theme.palette.bright_yellow.clone()),
            (12, default_theme.palette.bright_blue.clone()),
            (13, default_theme.palette.bright_magenta.clone()),
            (14, default_theme.palette.bright_cyan.clone()),
            (15, default_theme.palette.bright_white.clone()),
        ]);

        for index in 0..16 {
            let color = default_theme.get_color(ansi::Color::Indexed(index));
            let expected_color = basic_indexed_colors_map.get(&index).unwrap();
            assert_eq!(color, hex_to_color(expected_color).unwrap())
        }
    }
    #[test]
    fn named_colors_resolve_from_the_palette_without_reparsing() {
        let palette = ColorPalette {
            bright_foreground: Some(String::from("#010203")),
            ..ColorPalette::default()
        };
        let theme = Theme::new(ThemeSettings::new(Box::new(palette.clone())));
        for named in NAMED_COLORS {
            assert_eq!(
                theme.get_color(ansi::Color::Named(named)),
                hex_to_color(named_hex(&palette, named)).unwrap(),
                "{named:?}"
            );
        }
        assert_eq!(
            theme.get_color(ansi::Color::Named(NamedColor::BrightForeground)),
            Color::from_rgb8(1, 2, 3)
        );
        assert_eq!(
            theme.get_color(ansi::Color::Named(NamedColor::Cursor)),
            hex_to_color(&palette.background).unwrap()
        );
    }

    #[test]
    fn extended_indexed_colors_use_the_xterm_cube_and_gray_ramp() {
        let theme = Theme::default();
        assert_eq!(
            theme.get_color(ansi::Color::Indexed(16)),
            Color::from_rgb8(0, 0, 0)
        );
        assert_eq!(
            theme.get_color(ansi::Color::Indexed(196)),
            Color::from_rgb8(255, 0, 0)
        );
        assert_eq!(
            theme.get_color(ansi::Color::Indexed(231)),
            Color::from_rgb8(255, 255, 255)
        );
        assert_eq!(
            theme.get_color(ansi::Color::Indexed(232)),
            Color::from_rgb8(8, 8, 8)
        );
        assert_eq!(
            theme.get_color(ansi::Color::Indexed(255)),
            Color::from_rgb8(238, 238, 238)
        );
    }
}
