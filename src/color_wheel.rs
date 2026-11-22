//! A hue and saturation color wheel, drawn on an iced canvas.
//!
//! Angle is hue (red at 3 o'clock, counterclockwise), distance from the center
//! is saturation. The wheel is drawn at the color's current value
//! (brightness), which a separate slider controls. Press and drag anywhere on
//! the wheel; the drag keeps tracking outside it, clamped to the rim.

use crate::theme::{Hsv, Palette};
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke, gradient};
use iced::{Color, Element, Point, Rectangle, Renderer, Size, Theme, mouse};

/// Wedges in the wheel. Each one is a gradient from grey (center) to full
/// saturation (rim). At 2° per wedge, the edges are invisible.
const SEGMENTS: u16 = 180;
/// Space around the wheel so the selection marker isn't clipped at the rim.
const MARGIN: f32 = 8.0;

pub fn view<'a, Message: 'a>(
    hsv: Hsv,
    palette: Palette,
    size: f32,
    on_change: impl Fn(Hsv) -> Message + 'a,
) -> Element<'a, Message> {
    canvas::Canvas::new(ColorWheel {
        hsv,
        palette,
        on_change: Box::new(on_change),
    })
    .width(size)
    .height(size)
    .into()
}

struct ColorWheel<'a, Message> {
    hsv: Hsv,
    palette: Palette,
    on_change: Box<dyn Fn(Hsv) -> Message + 'a>,
}

/// Where the wheel sits inside the canvas.
#[derive(Clone, Copy, Debug)]
struct Wheel {
    center: Point,
    radius: f32,
}

impl Wheel {
    fn new(size: Size) -> Self {
        Self {
            center: Point::new(size.width / 2.0, size.height / 2.0),
            radius: (size.width.min(size.height) / 2.0 - MARGIN).max(1.0),
        }
    }

    /// The point for a hue (degrees) and saturation (0..=1).
    fn point(self, hue: f32, saturation: f32) -> Point {
        let angle = hue.to_radians();
        let distance = self.radius * saturation;
        // Screen y grows downward, so subtract to turn counterclockwise.
        Point::new(
            self.center.x + distance * angle.cos(),
            self.center.y - distance * angle.sin(),
        )
    }

    fn contains(self, point: Point) -> bool {
        point.distance(self.center) <= self.radius + MARGIN / 2.0
    }

    /// The hue and saturation under `point`, clamped to the rim, keeping `value`.
    fn pick(self, point: Point, value: f32) -> Hsv {
        let dx = point.x - self.center.x;
        let dy = self.center.y - point.y;
        Hsv {
            hue: dy.atan2(dx).to_degrees().rem_euclid(360.0),
            saturation: (dx.hypot(dy) / self.radius).min(1.0),
            value,
        }
    }
}

impl<Message> canvas::Program<Message> for ColorWheel<'_, Message> {
    /// True while a drag that started on the wheel is in progress. iced keeps
    /// this state in its widget tree between frames, like `useState`.
    type State = bool;

    fn update(
        &self,
        dragging: &mut bool,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let wheel = Wheel::new(bounds.size());
        let pick = |point| (self.on_change)(wheel.pick(point, self.hsv.value));
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let point = cursor.position_in(bounds).filter(|p| wheel.contains(*p))?;
                *dragging = true;
                Some(Action::publish(pick(point)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if *dragging => {
                // Relative to the canvas even when the pointer has left it.
                let point = cursor.position_from(bounds.position())?;
                Some(Action::publish(pick(point)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if *dragging => {
                *dragging = false;
                Some(Action::capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _: &bool,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let wheel = Wheel::new(bounds.size());
        let value = self.hsv.value;
        let step = 360.0 / f32::from(SEGMENTS);
        for segment in 0..SEGMENTS {
            let start = f32::from(segment) * step;
            let hue = start + step / 2.0;
            // Overlap neighbors slightly so anti-aliasing leaves no seams.
            let wedge = Path::new(|path| {
                path.move_to(wheel.center);
                path.line_to(wheel.point(start - 0.5, 1.0));
                path.line_to(wheel.point(start + step + 0.5, 1.0));
                path.close();
            });
            let color = |saturation| {
                Hsv {
                    hue,
                    saturation,
                    value,
                }
                .to_color()
            };
            frame.fill(
                &wedge,
                gradient::Linear::new(wheel.center, wheel.point(hue, 1.0))
                    .add_stop(0.0, color(0.0))
                    .add_stop(1.0, color(1.0)),
            );
        }
        frame.stroke(
            &Path::circle(wheel.center, wheel.radius),
            Stroke::default()
                .with_color(self.palette.edge())
                .with_width(1.0),
        );

        // The marker: the current color, ringed in white and black so it shows
        // on both light and dark parts of the wheel.
        let marker = wheel.point(self.hsv.hue, self.hsv.saturation);
        frame.fill(&Path::circle(marker, 6.0), self.hsv.to_color());
        frame.stroke(
            &Path::circle(marker, 6.0),
            Stroke::default().with_color(Color::WHITE).with_width(2.0),
        );
        frame.stroke(
            &Path::circle(marker, 7.5),
            Stroke::default()
                .with_color(Color::BLACK.scale_alpha(0.6))
                .with_width(1.0),
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        dragging: &bool,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let wheel = Wheel::new(bounds.size());
        let over = cursor
            .position_in(bounds)
            .is_some_and(|point| wheel.contains(point));
        if *dragging || over {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_positions_map_to_hue_and_saturation_and_clamp_to_the_rim() {
        let wheel = Wheel::new(Size::new(216.0, 216.0));
        let at = |x, y| wheel.pick(Point::new(x, y), 0.5);
        // Center is grey; the right edge is red, the top is 90°, the left 180°.
        assert_eq!(at(108.0, 108.0).saturation, 0.0);
        let right = at(208.0, 108.0);
        assert_eq!((right.hue, right.saturation, right.value), (0.0, 1.0, 0.5));
        assert!((at(108.0, 8.0).hue - 90.0).abs() < 1e-3);
        assert!((at(8.0, 108.0).hue - 180.0).abs() < 1e-3);
        // Dragging past the rim stays at full saturation.
        assert_eq!(at(1000.0, 108.0).saturation, 1.0);
        // `point` is the inverse of `pick`.
        let back = wheel.pick(wheel.point(237.0, 0.4), 0.5);
        assert!((back.hue - 237.0).abs() < 1e-3 && (back.saturation - 0.4).abs() < 1e-4);
    }
}
