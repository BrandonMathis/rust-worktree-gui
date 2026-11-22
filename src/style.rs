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

pub fn application(p: Palette) -> theme::Style {
    theme::Style {
        background_color: p.window(),
        text_color: p.foreground,
    }
}

pub fn panel(p: Palette, kind: Panel) -> impl Fn(&Theme) -> container::Style {
    move |_| match kind {
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
        radius: 3.0.into(),
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
