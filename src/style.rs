use iced::widget::{button, container};
use iced::{Background, Border, Color, Gradient, Theme, gradient, theme};

pub const TEXT: Color = Color::from_rgb8(248, 248, 242);
pub const MUTED: Color = Color::from_rgb8(171, 173, 196);
pub const PURPLE: Color = Color::from_rgb8(189, 147, 249);
pub const GREEN: Color = Color::from_rgb8(80, 250, 123);
pub const RED: Color = Color::from_rgb8(255, 85, 85);
pub const ORANGE: Color = Color::from_rgb8(255, 184, 108);
pub const CYAN: Color = Color::from_rgb8(139, 233, 253);
const EDGE: Color = Color::from_rgba8(189, 174, 234, 0.22);
const BASE: Color = Color::from_rgba8(24, 23, 35, 0.91);

pub fn application(_: &Theme) -> theme::Style {
    theme::Style {
        background_color: BASE,
        text_color: TEXT,
    }
}

pub fn glass(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            gradient::Linear::new(std::f32::consts::PI * 0.72)
                .add_stop(0.0, Color::from_rgba8(63, 57, 87, 0.72))
                .add_stop(1.0, Color::from_rgba8(33, 32, 50, 0.85)),
        ))),
        border: border(EDGE, 1.0),
        ..container::Style::default()
    }
}

pub fn sidebar_preview(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgb8(40, 38, 56).into()),
        border: border(EDGE, 1.0),
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.45),
            offset: iced::Vector::new(2.0, 4.0),
            blur_radius: 12.0,
        },
        ..container::Style::default()
    }
}

pub fn console(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgb8(40, 42, 54).into()),
        border: border(EDGE, 1.0),
        ..container::Style::default()
    }
}

pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);
    style.border = border(Color::TRANSPARENT, 0.0);
    style
}

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    let (fill, color) = match status {
        button::Status::Hovered => (Color::from_rgba8(189, 147, 249, 0.25), TEXT),
        button::Status::Pressed => (Color::from_rgba8(189, 147, 249, 0.32), TEXT),
        button::Status::Disabled => (Color::TRANSPARENT, MUTED.scale_alpha(0.55)),
        button::Status::Active => (Color::from_rgba8(189, 147, 249, 0.10), TEXT),
    };
    button::Style {
        background: Some(fill.into()),
        text_color: color,
        border: border(EDGE, 1.0),
        ..button::Style::default()
    }
}

pub fn nav(_: &Theme, status: button::Status) -> button::Style {
    let fill = match status {
        button::Status::Hovered => Color::from_rgba8(189, 147, 249, 0.14),
        button::Status::Pressed => Color::from_rgba8(189, 147, 249, 0.23),
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(fill.into()),
        text_color: TEXT,
        border: border(Color::TRANSPARENT, 0.0),
        ..button::Style::default()
    }
}

/// Project rows in the sidebar: a faint fill at rest so they read as section
/// headers above their (transparent) worktree rows.
pub fn project(_: &Theme, status: button::Status) -> button::Style {
    let fill = match status {
        button::Status::Hovered => Color::from_rgba8(189, 147, 249, 0.16),
        button::Status::Pressed => Color::from_rgba8(189, 147, 249, 0.24),
        _ => Color::from_rgba8(248, 248, 242, 0.05),
    };
    button::Style {
        background: Some(fill.into()),
        text_color: TEXT,
        border: border(Color::TRANSPARENT, 0.0),
        ..button::Style::default()
    }
}

pub fn selected(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(
            Color::from_rgba8(
                189,
                147,
                249,
                if status == button::Status::Hovered {
                    0.24
                } else {
                    0.18
                },
            )
            .into(),
        ),
        text_color: TEXT,
        border: border(PURPLE.scale_alpha(0.55), 1.0),
        ..button::Style::default()
    }
}

pub fn action_picker(
    theme: &Theme,
    status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    let mut style = iced::widget::pick_list::default(theme, status);
    style.placeholder_color = TEXT;
    style.handle_color = PURPLE;
    style.border = border(EDGE, 1.0);
    style
}

pub fn action_menu(theme: &Theme) -> iced::widget::overlay::menu::Style {
    let mut style = iced::widget::overlay::menu::default(theme);
    style.border = border(EDGE, 1.0);
    style
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

    #[test]
    fn panels_and_controls_use_three_pixel_dracula_rounding() {
        let theme = Theme::Dracula;
        let radius: iced::border::Radius = 3.0.into();
        assert_eq!(glass(&theme).border.radius, radius);
        assert_eq!(console(&theme).border.radius, radius);
        assert_eq!(action_menu(&theme).border.radius, radius);
        for status in [button::Status::Active, button::Status::Hovered] {
            assert_eq!(secondary(&theme, status).border.radius, radius);
            assert_eq!(selected(&theme, status).border.radius, radius);
        }
        // Translucent window base lets macOS blur show through.
        assert!(application(&theme).background_color.a < 1.0);
        assert_eq!(
            console(&theme).background,
            Some(Color::from_rgb8(40, 42, 54).into())
        );
    }
}
