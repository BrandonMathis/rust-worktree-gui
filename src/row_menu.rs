use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, overlay, renderer,
    widget::{Tree, tree},
};
use iced::widget::overlay::menu;
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use iced::{keyboard, mouse};

use crate::{style, theme::Palette};

const WIDTH: f32 = 174.0;

/// An overflow button with a dropdown wider than the trigger.
struct RowMenu<T, Message> {
    key: String,
    options: Vec<T>,
    on_select: Box<dyn Fn(T) -> Message>,
    appearance: <Theme as menu::Catalog>::Class<'static>,
    plus: bool,
    palette: Palette,
}

struct State {
    key: String,
    open: bool,
    menu: menu::State,
    hovered: Option<usize>,
    pointer_over: bool,
}

pub fn show<T, Message>(
    key: String,
    options: Vec<T>,
    on_select: impl Fn(T) -> Message + 'static,
    palette: Palette,
) -> Element<'static, Message>
where
    T: Clone + ToString + 'static,
    Message: 'static,
{
    trigger(key, options, on_select, false, palette)
}

pub fn add<T: Clone + ToString + 'static, Message: 'static>(
    options: Vec<T>,
    on_select: impl Fn(T) -> Message + 'static,
    palette: Palette,
) -> Element<'static, Message> {
    trigger("projects-add".into(), options, on_select, true, palette)
}

fn trigger<T: Clone + ToString + 'static, Message: 'static>(
    key: String,
    options: Vec<T>,
    on_select: impl Fn(T) -> Message + 'static,
    plus: bool,
    palette: Palette,
) -> Element<'static, Message> {
    Element::new(RowMenu {
        key,
        options,
        on_select: Box::new(on_select),
        appearance: Box::new(style::action_menu(palette)),
        plus,
        palette,
    })
}

impl<T: Clone + ToString, Message> Widget<Message, Theme, Renderer> for RowMenu<T, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            key: self.key.clone(),
            open: false,
            menu: menu::State::new(),
            hovered: None,
            pointer_over: false,
        })
    }

    fn diff(&self, tree: &mut Tree) {
        if tree.state.downcast_ref::<State>().key != self.key {
            tree.state = self.state();
        }
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(22.0), Length::Fixed(18.0))
    }

    fn layout(&mut self, _: &mut Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        let size = self.size();
        layout::Node::new(limits.resolve(size.width, size.height, Size::ZERO))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let over = cursor.is_over(layout.bounds()) && cursor.is_over(*viewport);
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) && (over || state.open)
        {
            state.open = over && !state.open;
            state.hovered = None;
            shell.capture_event();
            shell.request_redraw();
        } else if state.open
            && matches!(
                event,
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Escape),
                    ..
                })
            )
        {
            state.open = false;
            shell.capture_event();
            shell.request_redraw();
        } else if over != state.pointer_over {
            shell.request_redraw();
        }
        state.pointer_over = over;
    }

    fn mouse_interaction(
        &self,
        _: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        _: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) && cursor.is_over(*viewport) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _: &Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let p = self.palette;
        let bounds = layout.bounds();
        let highlight = cursor.is_over(bounds) || tree.state.downcast_ref::<State>().open;
        if highlight {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: iced::border::rounded(3),
                    ..Default::default()
                },
                p.purple.scale_alpha(0.22),
            );
        }
        let mut paint = |x, y, width, height, color| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle::new(Point::new(x, y), Size::new(width, height)),
                    border: iced::border::rounded(1),
                    ..Default::default()
                },
                color,
            )
        };
        let (x, y) = (bounds.center_x(), bounds.center_y());
        if self.plus {
            paint(x - 5.0, y - 0.75, 10.0, 1.5, p.purple);
            paint(x - 0.75, y - 5.0, 1.5, 10.0, p.purple);
        } else {
            for offset in [-4.0, 0.0, 4.0] {
                paint(
                    x - 1.0,
                    y + offset - 1.0,
                    2.0,
                    2.0,
                    if highlight { p.foreground } else { p.muted() },
                );
            }
        }
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        _: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        let state = tree.state.downcast_mut::<State>();
        if !state.open {
            return None;
        }
        let bounds = layout.bounds() + translation;
        let width = if self.plus { 194.0 } else { WIDTH };
        let position = Point::new((bounds.x + bounds.width - width).max(0.0), bounds.y);
        Some(
            menu::Menu::new(
                &mut state.menu,
                &self.options,
                &mut state.hovered,
                |option| {
                    state.open = false;
                    (self.on_select)(option)
                },
                None,
                &self.appearance,
            )
            .width(width)
            .padding([5, 8])
            .text_size(12)
            .overlay(position, *viewport, bounds.height, Length::Shrink),
        )
    }
}
