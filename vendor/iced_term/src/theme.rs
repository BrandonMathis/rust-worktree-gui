use crate::settings::ThemeSettings;
use alacritty_terminal::vte::ansi::{self, NamedColor};
use iced::{widget::container, Color};
use std::collections::HashMap;

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

#[derive(Debug, Clone)]
pub struct Theme {
    palette: Box<ColorPalette>,
    ansi256_colors: HashMap<u8, Color>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            palette: Box::<ColorPalette>::default(),
            ansi256_colors: build_ansi256_colors(),
        }
    }
}

impl Theme {
    pub fn new(settings: ThemeSettings) -> Self {
        Self {
            palette: settings.color_pallete,
            ansi256_colors: build_ansi256_colors(),
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
            ansi::Color::Indexed(index) => {
                if index <= 15 {
                    let color = match index {
                        // Normal terminal colors
                        0 => &self.palette.black,
                        1 => &self.palette.red,
                        2 => &self.palette.green,
                        3 => &self.palette.yellow,
                        4 => &self.palette.blue,
                        5 => &self.palette.magenta,
                        6 => &self.palette.cyan,
                        7 => &self.palette.white,
                        // Bright terminal colors
                        8 => &self.palette.bright_black,
                        9 => &self.palette.bright_red,
                        10 => &self.palette.bright_green,
                        11 => &self.palette.bright_yellow,
                        12 => &self.palette.bright_blue,
                        13 => &self.palette.bright_magenta,
                        14 => &self.palette.bright_cyan,
                        15 => &self.palette.bright_white,
                        _ => &self.palette.background,
                    };

                    return hex_to_color(color)
                        .unwrap_or_else(|_| panic!("invalid color {}", color));
                }

                // Other colors
                match self.ansi256_colors.get(&index) {
                    Some(color) => *color,
                    None => Color::from_rgb8(0, 0, 0),
                }
            },
            ansi::Color::Named(c) => {
                let color = match c {
                    NamedColor::Foreground => &self.palette.foreground,
                    NamedColor::Background => &self.palette.background,
                    // Normal terminal colors
                    NamedColor::Black => &self.palette.black,
                    NamedColor::Red => &self.palette.red,
                    NamedColor::Green => &self.palette.green,
                    NamedColor::Yellow => &self.palette.yellow,
                    NamedColor::Blue => &self.palette.blue,
                    NamedColor::Magenta => &self.palette.magenta,
                    NamedColor::Cyan => &self.palette.cyan,
                    NamedColor::White => &self.palette.white,
                    // Bright terminal colors
                    NamedColor::BrightBlack => &self.palette.bright_black,
                    NamedColor::BrightRed => &self.palette.bright_red,
                    NamedColor::BrightGreen => &self.palette.bright_green,
                    NamedColor::BrightYellow => &self.palette.bright_yellow,
                    NamedColor::BrightBlue => &self.palette.bright_blue,
                    NamedColor::BrightMagenta => &self.palette.bright_magenta,
                    NamedColor::BrightCyan => &self.palette.bright_cyan,
                    NamedColor::BrightWhite => &self.palette.bright_white,
                    NamedColor::BrightForeground => {
                        match &self.palette.bright_foreground {
                            Some(color) => color,
                            None => &self.palette.foreground,
                        }
                    },
                    // Dim terminal colors
                    NamedColor::DimForeground => &self.palette.dim_foreground,
                    NamedColor::DimBlack => &self.palette.dim_black,
                    NamedColor::DimRed => &self.palette.dim_red,
                    NamedColor::DimGreen => &self.palette.dim_green,
                    NamedColor::DimYellow => &self.palette.dim_yellow,
                    NamedColor::DimBlue => &self.palette.dim_blue,
                    NamedColor::DimMagenta => &self.palette.dim_magenta,
                    NamedColor::DimCyan => &self.palette.dim_cyan,
                    NamedColor::DimWhite => &self.palette.dim_white,
                    _ => &self.palette.background,
                };

                hex_to_color(color)
                    .unwrap_or_else(|_| panic!("invalid color {}", color))
            },
        }
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

fn build_ansi256_colors() -> HashMap<u8, Color> {
    let mut ansi256_colors = HashMap::new();

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
                ansi256_colors.insert(index, color);
            }
        }
    }

    let index: u8 = 232;
    for i in 0..24 {
        let value = i * 10 + 8;
        ansi256_colors.insert(index + i, Color::from_rgb8(value, value, value));
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
}
