//! Widget styles, drawn from the active [`Palette`].
//!
//! Each function takes the palette by value and returns a closure that iced
//! calls when it draws the widget, e.g. `.style(style::panel(p, Panel::Glass))`.
//! The closure owns its copy of the palette (`move`), so it can outlive the
//! view call that built it. Like passing theme values as props in React rather
//! than reading globals.

use crate::theme::Palette;
use iced::widget::button as btn;
use iced::widget::{container, pick_list, slider, text_input};
use iced::{Background, Border, Color, Gradient, Theme, gradient, theme};

/// Container surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    /// Translucent glass: sidebar, headers, forms.
    Glass,
    /// Opaque with a drop shadow: the sidebar shown over the workspace.
    Floating,
    /// Solid background: the terminal pane and tooltips.
    Console,
}

/// Button looks. An enum (not one function per look) means `if active {
/// Button::Selected } else { Button::Nav }` has a single closure type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Primary,
    Secondary,
    /// Transparent until hovered: toolbar icons and worktree rows.
    Nav,
    /// Project rows: a faint fill at rest so they read as section headers
    /// above their (transparent) worktree rows.
    Project,
    Selected,
}

/// The corner radius of panels, buttons and menus.
const RADIUS: f32 = 3.0;

/// Space between the window edge and its panels (the padding in `App::view`).
pub const WINDOW_GAP: f32 = 4.0;

/// Rounding for a panel's two bottom corners.
///
/// Panels that sit in a bottom corner of the window take the window's own
/// radius minus [`WINDOW_GAP`], so their curve runs parallel to the macOS
/// window corner instead of a tight 3px curve inside a wide one. The value is
/// handed down through the view functions, like a prop, so each panel knows
/// where it sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corners {
    bottom_left: f32,
    bottom_right: f32,
}

impl Corners {
    /// A panel away from the window corners.
    pub const NONE: Self = Self {
        bottom_left: RADIUS,
        bottom_right: RADIUS,
    };

    /// Both bottom corners of a window whose own corners have `window_radius`.
    pub fn window(window_radius: f32) -> Self {
        let radius = (window_radius - WINDOW_GAP).max(RADIUS);
        Self {
            bottom_left: radius,
            bottom_right: radius,
        }
    }

    /// The share of these corners for the leftmost panel in a row.
    pub fn left(self) -> Self {
        Self {
            bottom_right: RADIUS,
            ..self
        }
    }

    /// The share of these corners for the rightmost panel in a row.
    pub fn right(self) -> Self {
        Self {
            bottom_left: RADIUS,
            ..self
        }
    }

    /// These corners as an iced border radius, with 3px top corners.
    pub fn radius(self) -> iced::border::Radius {
        iced::border::Radius::new(RADIUS)
            .bottom_left(self.bottom_left)
            .bottom_right(self.bottom_right)
    }
}

/// The corner radius macOS draws on a titled window, in points.
///
/// AppKit reports 16 on macOS 26 (Tahoe) and 10 on earlier releases, but has
/// no public API for it, so this reads the OS version. Other platforms return
/// 0, which leaves every panel at the plain 3px rounding.
pub fn window_radius() -> f32 {
    if !cfg!(target_os = "macos") {
        return 0.0;
    }
    std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
        .ok()
        .and_then(|plist| macos_major_version(&plist))
        .map_or(16.0, macos_window_radius)
}

fn macos_window_radius(major_version: u32) -> f32 {
    if major_version >= 26 { 16.0 } else { 10.0 }
}

/// The major version from `SystemVersion.plist`, e.g. 26 for `26.6.2`.
fn macos_major_version(plist: &str) -> Option<u32> {
    let (_, after_key) = plist.split_once("<key>ProductVersion</key>")?;
    let (_, value) = after_key.split_once("<string>")?;
    let (version, _) = value.split_once("</string>")?;
    version.trim().split('.').next()?.parse().ok()
}

pub fn application(p: Palette) -> theme::Style {
    theme::Style {
        background_color: p.window(),
        text_color: p.foreground,
    }
}

pub fn panel(p: Palette, kind: Panel) -> impl Fn(&Theme) -> container::Style {
    panel_in(p, kind, Corners::NONE)
}

/// The dimmed backdrop behind a modal, so the app beneath reads as inactive.
pub fn scrim(p: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(p.scrim().into()),
        ..container::Style::default()
    }
}

/// A panel that may sit in a bottom corner of the window. See [`Corners`].
pub fn panel_in(p: Palette, kind: Panel, corners: Corners) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let mut style = panel_style(p, kind);
        style.border.radius = corners.radius();
        style
    }
}

fn panel_style(p: Palette, kind: Panel) -> container::Style {
    match kind {
        Panel::Glass => {
            let (top, bottom) = p.glass();
            container::Style {
                background: Some(Background::Gradient(Gradient::Linear(
                    gradient::Linear::new(std::f32::consts::PI * 0.72)
                        .add_stop(0.0, top)
                        .add_stop(1.0, bottom),
                ))),
                border: border(p.edge(), 1.0),
                ..container::Style::default()
            }
        }
        Panel::Floating => container::Style {
            background: Some(p.floating().into()),
            border: border(p.edge(), 1.0),
            shadow: iced::Shadow {
                color: Color::BLACK.scale_alpha(0.45),
                offset: iced::Vector::new(2.0, 4.0),
                blur_radius: 12.0,
            },
            ..container::Style::default()
        },
        Panel::Console => container::Style {
            background: Some(p.background.into()),
            border: border(p.edge(), 1.0),
            ..container::Style::default()
        },
    }
}

pub fn button(p: Palette, kind: Button) -> impl Fn(&Theme, btn::Status) -> btn::Style {
    move |theme, status| {
        use btn::Status::{Active, Disabled, Hovered, Pressed};
        let tint = |alpha| Some(p.purple.scale_alpha(alpha).into());
        let plain = |background| btn::Style {
            background,
            text_color: p.foreground,
            border: border(Color::TRANSPARENT, 0.0),
            ..btn::Style::default()
        };
        match kind {
            Button::Primary => btn::Style {
                border: border(Color::TRANSPARENT, 0.0),
                ..btn::primary(theme, status)
            },
            Button::Secondary => btn::Style {
                background: match status {
                    Hovered => tint(0.25),
                    Pressed => tint(0.32),
                    Disabled => Some(Color::TRANSPARENT.into()),
                    Active => tint(0.10),
                },
                text_color: if status == Disabled {
                    p.muted().scale_alpha(0.55)
                } else {
                    p.foreground
                },
                border: border(p.edge(), 1.0),
                ..btn::Style::default()
            },
            Button::Nav => plain(match status {
                Hovered => tint(0.14),
                Pressed => tint(0.23),
                _ => Some(Color::TRANSPARENT.into()),
            }),
            Button::Project => plain(match status {
                Hovered => tint(0.16),
                Pressed => tint(0.24),
                _ => Some(p.foreground.scale_alpha(0.05).into()),
            }),
            Button::Selected => btn::Style {
                background: tint(if status == Hovered { 0.24 } else { 0.18 }),
                text_color: p.foreground,
                border: border(p.purple.scale_alpha(0.55), 1.0),
                ..btn::Style::default()
            },
        }
    }
}

pub fn action_picker(p: Palette) -> impl Fn(&Theme, pick_list::Status) -> pick_list::Style {
    move |theme, status| pick_list::Style {
        placeholder_color: p.foreground,
        handle_color: p.purple,
        border: border(p.edge(), 1.0),
        ..pick_list::default(theme, status)
    }
}

pub fn action_menu(p: Palette) -> impl Fn(&Theme) -> iced::widget::overlay::menu::Style {
    move |theme| iced::widget::overlay::menu::Style {
        border: border(p.edge(), 1.0),
        ..iced::widget::overlay::menu::default(theme)
    }
}

/// Hex code fields in the theme editor.
pub fn input(p: Palette) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |theme, status| text_input::Style {
        border: border(
            if matches!(status, text_input::Status::Focused { .. }) {
                p.purple
            } else {
                p.edge()
            },
            1.0,
        ),
        ..text_input::default(theme, status)
    }
}

/// A rail filled with the selected color, so brightness reads at a glance.
pub fn brightness(p: Palette, color: Color) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |theme, status| {
        let mut style = slider::default(theme, status);
        style.rail.backgrounds = (color.into(), p.selection.into());
        style.rail.width = 6.0;
        style.rail.border = border(p.edge(), 1.0);
        style.handle.background = p.foreground.into();
        style.handle.border_color = p.background;
        style.handle.border_width = 2.0;
        style
    }
}

/// A small square of solid color.
pub fn swatch(p: Palette, color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        border: border(p.foreground.scale_alpha(0.35), 1.0),
        ..container::Style::default()
    }
}

fn border(color: Color, width: f32) -> Border {
    Border {
        color,
        width,
        radius: RADIUS.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Preset;

    #[test]
    fn panels_and_controls_use_three_pixel_dracula_rounding() {
        let p = Palette::default();
        let theme = p.iced_theme();
        let radius: iced::border::Radius = 3.0.into();
        assert_eq!(panel(p, Panel::Glass)(&theme).border.radius, radius);
        assert_eq!(panel(p, Panel::Console)(&theme).border.radius, radius);
        assert_eq!(action_menu(p)(&theme).border.radius, radius);
        for status in [btn::Status::Active, btn::Status::Hovered] {
            assert_eq!(
                button(p, Button::Secondary)(&theme, status).border.radius,
                radius
            );
            assert_eq!(
                button(p, Button::Selected)(&theme, status).border.radius,
                radius
            );
        }
        // Translucent window base lets macOS blur show through.
        assert!(application(p).background_color.a < 1.0);
        assert_eq!(
            panel(p, Panel::Console)(&theme).background,
            Some(Color::from_rgb8(40, 42, 54).into())
        );
    }

    #[test]
    fn panels_in_the_bottom_window_corners_run_parallel_to_the_window_curve() {
        let p = Palette::default();
        let theme = p.iced_theme();
        let radius = |corners| panel_in(p, Panel::Console, corners)(&theme).border.radius;
        // A 16pt macOS window corner, less the 4px gap: 12px bottom corners.
        let window = Corners::window(16.0);
        let both = radius(window);
        assert_eq!((both.top_left, both.top_right), (3.0, 3.0));
        assert_eq!((both.bottom_left, both.bottom_right), (12.0, 12.0));
        // Side-by-side panels each take only their own corner.
        let left = radius(window.left());
        assert_eq!((left.bottom_left, left.bottom_right), (12.0, 3.0));
        let right = radius(window.right());
        assert_eq!((right.bottom_left, right.bottom_right), (3.0, 12.0));
        // Square-cornered windows (other platforms) keep the 3px rounding.
        assert_eq!(radius(Corners::window(0.0)), 3.0.into());
    }

    #[test]
    fn macos_26_and_later_windows_have_wider_corners_than_earlier_releases() {
        let plist = |version: &str| {
            format!(
                "<dict>\n\t<key>ProductName</key>\n\t<string>macOS</string>\n\t\
                 <key>ProductVersion</key>\n\t<string>{version}</string>\n</dict>"
            )
        };
        assert_eq!(macos_major_version(&plist("26.6.2")), Some(26));
        assert_eq!(macos_major_version(&plist("15.5")), Some(15));
        assert_eq!(macos_major_version("<dict></dict>"), None);
        assert_eq!(macos_window_radius(26), 16.0);
        assert_eq!(macos_window_radius(27), 16.0);
        assert_eq!(macos_window_radius(15), 10.0);
    }

    #[test]
    fn styles_follow_the_palette_they_are_given() {
        let p = Preset::Blade.palette();
        let theme = p.iced_theme();
        assert_eq!(
            panel(p, Panel::Console)(&theme).background,
            Some(p.background.into())
        );
        assert_eq!(application(p).text_color, p.foreground);
        assert_eq!(theme.palette().primary, p.purple);
        let selected = button(p, Button::Selected)(&theme, btn::Status::Active);
        assert_eq!(selected.border.color, p.purple.scale_alpha(0.55));
    }
}
