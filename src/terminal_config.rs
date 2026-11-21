use iced::Font;
use iced::keyboard::{Modifiers, key::Named};
use iced_term::bindings::{Binding, BindingAction, InputKind};
use iced_term::settings::FontSettings;
use iced_term::{ColorPalette, CursorColor, CursorColors, TermMode};
use std::env;
use std::path::{Path, PathBuf};
use toml::Value;

type KeyBinding = (Binding<InputKind>, BindingAction);

#[derive(Clone)]
pub struct Preferences {
    pub font: FontSettings,
    pub palette: ColorPalette,
    pub bindings: Vec<KeyBinding>,
    #[cfg(target_os = "macos")]
    normal_style: Option<String>,
    #[cfg(target_os = "macos")]
    cell_offset: iced::Size,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            font: FontSettings {
                size: 14.0,
                scale_factor: 1.27,
                ..FontSettings::default()
            },
            palette: dracula(),
            bindings: Vec::new(),
            #[cfg(target_os = "macos")]
            normal_style: None,
            #[cfg(target_os = "macos")]
            cell_offset: iced::Size::ZERO,
        }
    }
}

impl Preferences {
    pub fn load() -> Result<Self, String> {
        let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        match locate(xdg, &home) {
            Some(path) => Ok(Self::from_config(&read_config(&path, &home, 5)?)),
            None => Ok(Self::default()),
        }
    }

    fn from_config(config: &Value) -> Self {
        let mut result = Self::default();
        if let Some(font) = config.get("font") {
            if let Some(size) = font
                .get("size")
                .and_then(number)
                .filter(|size| (5.0..=72.0).contains(size))
            {
                // Alacritty's macOS rasterizer uses Core Text point sizes directly.
                // Iced also draws in logical points; both apply Retina scaling later.
                result.font.size = if cfg!(target_os = "macos") {
                    size as f32
                } else {
                    (size * 4.0 / 3.0) as f32
                };
            }
            if let Some(family) = font
                .get("normal")
                .and_then(|font| font.get("family"))
                .and_then(Value::as_str)
                .filter(|family| !family.is_empty())
            {
                // iced's Font takes a static family name. Preferences are loaded once per app run.
                result.font.font_type = Font::with_name(family.to_owned().leak());
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(font) = config.get("font") {
            result.normal_style = font
                .get("normal")
                .and_then(|normal| normal.get("style"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            if let Some(offset) = font.get("offset") {
                let axis = |name| {
                    offset
                        .get(name)
                        .and_then(Value::as_integer)
                        .unwrap_or(0)
                        .clamp(-128, 128) as f32
                };
                result.cell_offset = iced::Size::new(axis("x"), axis("y"));
            }
        }
        if let Some(colors) = config.get("colors") {
            palette_from_config(&mut result.palette, colors);
        }
        if let Some(bindings) = config
            .get("keyboard")
            .and_then(|keyboard| keyboard.get("bindings"))
            .and_then(Value::as_array)
        {
            result.bindings = bindings.iter().filter_map(key_binding).collect();
        }
        result
    }

    pub fn font_at_scale(&self, scale: f32) -> FontSettings {
        let mut settings = self.font.clone();
        // Keep iced's shaper for text, but use the same Core Text rasterizer as
        // Alacritty to derive the grid pitch in physical pixels.
        #[cfg(target_os = "macos")]
        if let Some(cell_size) = self.native_cell_size(scale) {
            settings.cell_size = Some(cell_size);
            settings.scale_factor = cell_size.height / settings.size;
        }
        let _ = scale;
        settings
    }

    #[cfg(target_os = "macos")]
    fn native_cell_size(&self, scale: f32) -> Option<iced::Size> {
        use crossfont::{FontDesc, Rasterize, Rasterizer, Size, Slant, Style, Weight};
        let iced::font::Family::Name(family) = self.font.font_type.family else {
            return None;
        };
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let size = Size::new(self.font.size).scale(scale);
        let style = self.normal_style.as_ref().map_or(
            Style::Description {
                slant: Slant::Normal,
                weight: Weight::Normal,
            },
            |style| Style::Specific(style.clone()),
        );
        let mut rasterizer = Rasterizer::new().ok()?;
        let font = rasterizer
            .load_font(&FontDesc::new(family, style), size)
            .ok()?;
        let metrics = rasterizer.metrics(font, size).ok()?;
        // Alacritty rounds down after adding the offsets, then lays out in
        // backing pixels. Don't round again after converting to logical points.
        let dimension = |metric: f64, offset: f32| {
            ((metric + f64::from(offset)).floor().max(1.0) as f32) / scale
        };
        Some(iced::Size::new(
            dimension(metrics.average_advance, self.cell_offset.width),
            dimension(metrics.line_height, self.cell_offset.height),
        ))
    }
}

/// Alacritty's own search order for its TOML config file.
fn locate(xdg_config_home: Option<PathBuf>, home: &Path) -> Option<PathBuf> {
    xdg_config_home
        .filter(|p| p.is_absolute())
        .into_iter()
        .flat_map(|p| [p.join("alacritty/alacritty.toml"), p.join("alacritty.toml")])
        .chain(
            [
                home.join(".config/alacritty/alacritty.toml"),
                home.join(".alacritty.toml"),
            ]
            .into_iter()
            .filter(|p| p.is_absolute()),
        )
        .find(|p| p.is_file())
}

fn read_config(path: &Path, home: &Path, depth: u8) -> Result<Value, String> {
    let err = |detail: &dyn std::fmt::Display| {
        format!("Alacritty settings at {}: {detail}", path.display())
    };
    let text = std::fs::read_to_string(path).map_err(|e| err(&e))?;
    let local: Value = toml::from_str(&text).map_err(|e| err(&e))?;
    let mut combined = Value::Table(Default::default());
    let imports = local
        .get("general")
        .and_then(|general| general.get("import"))
        .or_else(|| local.get("import"))
        .and_then(Value::as_array);
    if let Some(imports) = imports {
        if depth == 0 {
            return Err(err(&"too many nested imports (or an import cycle)"));
        }
        for import in imports.iter().filter_map(Value::as_str) {
            let path = if let Some(relative) = import.strip_prefix("~/") {
                home.join(relative)
            } else {
                path.parent().unwrap_or(Path::new("")).join(import)
            };
            if path.is_file() {
                merge(&mut combined, read_config(&path, home, depth - 1)?);
            }
        }
    }
    merge(&mut combined, local);
    Ok(combined)
}

fn merge(base: &mut Value, overlay: Value) {
    if let (Value::Table(base), Value::Table(overlay)) = (&mut *base, &overlay) {
        for (key, val) in overlay {
            match base.get_mut(key) {
                Some(existing) => merge(existing, val.clone()),
                None => {
                    base.insert(key.clone(), val.clone());
                }
            }
        }
    } else {
        *base = overlay;
    }
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|n| n as f64))
        .filter(|n| n.is_finite())
}

fn hex_color(section: &Value, key: &str) -> Option<String> {
    let color = section.get(key)?.as_str()?;
    let digits = color
        .strip_prefix('#')
        .or_else(|| color.strip_prefix("0x"))?;
    (digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{digits}"))
}

fn cursor_color(section: &Value, key: &str) -> Option<CursorColor> {
    match section.get(key)?.as_str()? {
        "CellForeground" => Some(CursorColor::CellForeground),
        "CellBackground" => Some(CursorColor::CellBackground),
        _ => Some(CursorColor::Fixed(hex_color(section, key)?.parse().ok()?)),
    }
}

fn palette_from_config(palette: &mut ColorPalette, colors: &Value) {
    for (key, cursor) in [
        ("cursor", &mut palette.cursor),
        ("vi_mode_cursor", &mut palette.vi_mode_cursor),
    ] {
        if let Some(section) = colors.get(key) {
            if let Some(color) = cursor_color(section, "cursor") {
                cursor.cursor = color;
            }
            if let Some(color) = cursor_color(section, "text") {
                cursor.text = color;
            }
        }
    }
    if let Some(primary) = colors.get("primary") {
        if let Some(color) = hex_color(primary, "foreground") {
            palette.foreground = color;
        }
        if let Some(color) = hex_color(primary, "background") {
            palette.background = color;
        }
        if let Some(color) = hex_color(primary, "bright_foreground") {
            palette.bright_foreground = Some(color);
        }
        if let Some(color) = hex_color(primary, "dim_foreground") {
            palette.dim_foreground = color;
        }
    }
    macro_rules! ansi {
        ($section:literal, $($field:ident => $key:literal),* $(,)?) => {
            if let Some(section) = colors.get($section) {
                $(if let Some(color) = hex_color(section, $key) { palette.$field = color; })*
            }
        };
    }
    ansi!("normal", black => "black", red => "red", green => "green", yellow => "yellow",
        blue => "blue", magenta => "magenta", cyan => "cyan", white => "white");
    ansi!("bright", bright_black => "black", bright_red => "red", bright_green => "green", bright_yellow => "yellow",
        bright_blue => "blue", bright_magenta => "magenta", bright_cyan => "cyan", bright_white => "white");
    ansi!("dim", dim_black => "black", dim_red => "red", dim_green => "green", dim_yellow => "yellow",
        dim_blue => "blue", dim_magenta => "magenta", dim_cyan => "cyan", dim_white => "white");
}

fn key_binding(item: &Value) -> Option<KeyBinding> {
    let chars = item.get("chars")?.as_str()?;
    let name = item.get("key")?.as_str()?;
    let target = match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => InputKind::KeyCode(Named::Enter),
        "tab" => InputKind::KeyCode(Named::Tab),
        "space" => InputKind::KeyCode(Named::Space),
        "backspace" => InputKind::KeyCode(Named::Backspace),
        "escape" => InputKind::KeyCode(Named::Escape),
        "up" => InputKind::KeyCode(Named::ArrowUp),
        "down" => InputKind::KeyCode(Named::ArrowDown),
        "left" => InputKind::KeyCode(Named::ArrowLeft),
        "right" => InputKind::KeyCode(Named::ArrowRight),
        key if key.chars().count() == 1 => InputKind::Char(key.to_owned()),
        _ => return None,
    };
    let mut modifiers = Modifiers::empty();
    for modifier in item
        .get("mods")
        .and_then(Value::as_str)
        .unwrap_or("")
        .split('|')
        .map(str::trim)
    {
        modifiers |= match modifier.to_ascii_lowercase().as_str() {
            "" | "none" => Modifiers::empty(),
            "shift" => Modifiers::SHIFT,
            "control" | "ctrl" => Modifiers::CTRL,
            "alt" | "option" => Modifiers::ALT,
            "super" | "command" => Modifiers::COMMAND,
            _ => return None,
        };
    }
    let mut include = TermMode::empty();
    let mut exclude = TermMode::empty();
    for mode in item
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("")
        .split('|')
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        let name = mode.strip_prefix('~').unwrap_or(mode);
        let flag = match name.to_ascii_lowercase().as_str() {
            "appcursor" => TermMode::APP_CURSOR,
            "appkeypad" => TermMode::APP_KEYPAD,
            "altscreen" => TermMode::ALT_SCREEN,
            "vi" => TermMode::VI,
            _ => return None,
        };
        if name == mode {
            include.insert(flag);
        } else {
            exclude.insert(flag);
        }
    }
    Some((
        Binding {
            target,
            modifiers,
            terminal_mode_include: include,
            terminal_mode_exclude: exclude,
        },
        BindingAction::Esc(chars.to_owned()),
    ))
}

fn dracula() -> ColorPalette {
    ColorPalette {
        foreground: "#f8f8f2".into(),
        background: "#282a36".into(),
        cursor: CursorColors::default(),
        vi_mode_cursor: CursorColors::default(),
        black: "#21222c".into(),
        red: "#ff5555".into(),
        green: "#50fa7b".into(),
        yellow: "#f1fa8c".into(),
        blue: "#bd93f9".into(),
        magenta: "#ff79c6".into(),
        cyan: "#8be9fd".into(),
        white: "#f8f8f2".into(),
        bright_black: "#6272a4".into(),
        bright_red: "#ff6e6e".into(),
        bright_green: "#69ff94".into(),
        bright_yellow: "#ffffa5".into(),
        bright_blue: "#d6acff".into(),
        bright_magenta: "#ff92df".into(),
        bright_cyan: "#a4ffff".into(),
        bright_white: "#ffffff".into(),
        bright_foreground: Some("#ffffff".into()),
        dim_foreground: "#abadc4".into(),
        dim_black: "#191a21".into(),
        dim_red: "#a44451".into(),
        dim_green: "#3ca45c".into(),
        dim_yellow: "#a6aa64".into(),
        dim_blue: "#856aae".into(),
        dim_magenta: "#a7588a".into(),
        dim_cyan: "#5e9ead".into(),
        dim_white: "#bcbdbb".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_imports_and_applies_local_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        std::fs::write(home.join("theme.toml"), "[colors.primary]\nbackground = '0xabcdef'\nforeground = '#111111'\n[colors.normal]\ngreen = '#112233'").unwrap();
        let path = home.join("alacritty.toml");
        std::fs::write(
            &path,
            r##"
[general]
import = ["missing.toml", "~/theme.toml"]
[colors.primary]
foreground = "#ffffff"
[colors.cursor]
cursor = "#f8f8f2"
text = "#282a36"
[colors.vi_mode_cursor]
cursor = "CellBackground"
text = "CellForeground"
[font]
size = 12
[font.normal]
family = "MonoLisa Nerd Font"
[keyboard]
bindings = [
  { key = "Enter", mods = "Alt", chars = "\u001b[13;3u" },
  { key = "F", mods = "Command|Shift", chars = "\u001b[102;6u" },
  { key = "Q", action = "Quit" },
]
"##,
        )
        .unwrap();
        let prefs = Preferences::from_config(&read_config(&path, home, 5).unwrap());
        assert_eq!(prefs.palette.background, "#abcdef");
        assert_eq!(prefs.palette.foreground, "#ffffff");
        assert_eq!(
            prefs.palette.cursor,
            CursorColors {
                cursor: CursorColor::Fixed(iced::Color::from_rgb8(248, 248, 242)),
                text: CursorColor::Fixed(iced::Color::from_rgb8(40, 42, 54)),
            }
        );
        assert_eq!(
            prefs.palette.vi_mode_cursor,
            CursorColors {
                cursor: CursorColor::CellBackground,
                text: CursorColor::CellForeground,
            }
        );
        assert_eq!(prefs.palette.green, "#112233");
        assert_eq!(
            prefs.font.size,
            if cfg!(target_os = "macos") {
                12.0
            } else {
                16.0
            }
        );
        assert_eq!(prefs.font.font_type, Font::with_name("MonoLisa Nerd Font"));
        assert_eq!(prefs.bindings.len(), 2);
        assert_eq!(prefs.bindings[0].0.target, InputKind::KeyCode(Named::Enter));
        assert_eq!(prefs.bindings[0].1, BindingAction::Esc("\x1b[13;3u".into()));
        assert_eq!(
            prefs.bindings[1].0.modifiers,
            Modifiers::COMMAND | Modifiers::SHIFT
        );
    }

    #[test]
    fn finds_config_in_alacritty_search_order() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let xdg = temp.path().join("xdg");
        let candidates = [
            xdg.join("alacritty/alacritty.toml"),
            xdg.join("alacritty.toml"),
            home.join(".config/alacritty/alacritty.toml"),
            home.join(".alacritty.toml"),
        ];
        assert_eq!(locate(Some(xdg.clone()), &home), None);
        // Create from lowest to highest priority; each new file must win.
        for expected in candidates.iter().rev() {
            std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
            std::fs::write(expected, "").unwrap();
            assert_eq!(locate(Some(xdg.clone()), &home).as_ref(), Some(expected));
        }
        // A relative XDG_CONFIG_HOME is ignored, as Alacritty does.
        assert_eq!(
            locate(Some(PathBuf::from("relative")), &home).as_ref(),
            Some(&candidates[2])
        );
    }

    #[test]
    fn defaults_to_dracula_and_ignores_unsupported_settings() {
        let defaults = Preferences::default();
        assert_eq!(defaults.palette.background, "#282a36");
        assert_eq!(defaults.palette.foreground, "#f8f8f2");
        assert!(defaults.bindings.is_empty());

        let config: Value = toml::from_str(
            r##"
[font]
size = 200
[colors.primary]
background = "red"
foreground = "#12345"
[keyboard]
bindings = [
  { key = "Q", action = "Quit" },
  { key = "PageUp", chars = "x" },
  { key = "A", mods = "Hyper", chars = "x" },
  { key = "B", mode = "Search", chars = "x" },
  { key = "Up", mode = "~AppCursor", chars = "\u001bOA" },
]
"##,
        )
        .unwrap();
        let prefs = Preferences::from_config(&config);
        assert_eq!(prefs.font.size, defaults.font.size);
        assert_eq!(prefs.palette.background, defaults.palette.background);
        assert_eq!(prefs.palette.foreground, defaults.palette.foreground);
        assert_eq!(prefs.bindings.len(), 1);
        assert_eq!(
            prefs.bindings[0].0.terminal_mode_exclude,
            TermMode::APP_CURSOR
        );
    }

    #[test]
    fn rejects_cyclic_imports() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("alacritty.toml");
        std::fs::write(&path, "[general]\nimport = ['alacritty.toml']").unwrap();
        assert!(
            read_config(&path, temp.path(), 5)
                .unwrap_err()
                .contains("import cycle")
        );
    }
}
