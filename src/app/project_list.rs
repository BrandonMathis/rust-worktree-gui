//! Adds pointer drag-and-drop to the sidebar's scrollable column of projects.

use super::{Message, ProjectId};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, overlay, renderer};
use iced::widget::Column;
use iced::{Color, Element, Event, Fill, Length, Rectangle, Renderer, Size, Theme, Vector};
use iced::{keyboard, mouse, window};
use std::collections::BTreeMap;

/// Leftmost part of a project header, reserved for the drag grip.
pub(super) const GRIP_WIDTH: u32 = 22;
const LINE_HEIGHT: f32 = 2.0;

pub(super) fn reorderable<'a>(
    projects: Vec<(ProjectId, Element<'a, Message>)>,
    accent: Color,
) -> Element<'a, Message> {
    let (ids, groups) = projects.into_iter().unzip();
    Element::new(ProjectList {
        ids,
        column: Column::from_vec(groups)
            .width(Fill)
            .spacing(LINE_HEIGHT)
            .padding(iced::padding::top(LINE_HEIGHT).bottom(LINE_HEIGHT)),
        accent,
    })
}

struct ProjectList<'a> {
    ids: Vec<ProjectId>,
    column: Column<'a, Message>,
    accent: Color,
}

struct State {
    ids: Vec<ProjectId>,
    grabbed: Option<ProjectId>,
}

impl ProjectList<'_> {
    fn grip_at(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: Rectangle,
    ) -> Option<ProjectId> {
        let point = cursor.position_over(viewport)?;
        self.ids
            .iter()
            .copied()
            .zip(layout.children())
            .find_map(|(id, group)| {
                // Each group starts with its header; any worktree rows follow it.
                let mut grip = group.children().next()?.bounds();
                grip.width = GRIP_WIDTH as f32;
                grip.contains(point).then_some(id)
            })
    }

    /// Find the insertion gap using whole project groups, including their worktrees.
    /// The scrollable supplies the cursor and viewport in the same coordinates as the content.
    fn drop_slot(
        &self,
        source: ProjectId,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: Rectangle,
    ) -> Option<(Option<ProjectId>, f32)> {
        let point = cursor.position_over(viewport)?;
        let bounds = layout.bounds();
        if point.x < bounds.x || point.x >= bounds.x + bounds.width {
            return None;
        }
        let source_index = self.ids.iter().position(|&id| id == source)?;
        let (index, line_y) = layout
            .children()
            .enumerate()
            .map(|(i, group)| (i, group.bounds()))
            .find(|(_, group)| point.y < group.center_y())
            .map_or(
                (self.ids.len(), bounds.y + bounds.height - LINE_HEIGHT),
                |(i, group)| (i, group.y - LINE_HEIGHT),
            );
        if index == source_index || index == source_index + 1 {
            return None;
        }
        Some((self.ids.get(index).copied(), line_y))
    }
}

impl Widget<Message, Theme, Renderer> for ProjectList<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            ids: self.ids.clone(),
            grabbed: None,
        })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(
            &self.column as &dyn Widget<Message, Theme, Renderer>,
        )]
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        let column = &mut tree.children[0];
        if state.ids != self.ids {
            // Columns diff by position. Keep focus and other local widget state with
            // the project that moved, rather than with the old slot in the list.
            let mut old = state
                .ids
                .iter()
                .copied()
                .zip(std::mem::take(&mut column.children))
                .collect::<BTreeMap<_, _>>();
            column.children = self
                .ids
                .iter()
                .map(|id| old.remove(id).unwrap_or_else(Tree::empty))
                .collect();
            state.ids.clone_from(&self.ids);
            if state.grabbed.is_some_and(|id| !self.ids.contains(&id)) {
                state.grabbed = None;
            }
        }
        column.diff(&self.column as &dyn Widget<Message, Theme, Renderer>);
    }

    fn size(&self) -> Size<Length> {
        self.column.size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.column.layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.column
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.ids.len() > 1 && !shell.is_event_captured() =>
            {
                state.grabbed = self.grip_at(layout, cursor, *viewport);
                if state.grabbed.is_some() {
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some(source) = state.grabbed.take() {
                    if let Some((before, _)) = self.drop_slot(source, layout, cursor, *viewport) {
                        shell.publish(Message::ReorderProject { source, before });
                    }
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) if state.grabbed.take().is_some() => {
                shell.capture_event();
                shell.request_redraw();
                return;
            }
            Event::Window(window::Event::Unfocused) | Event::Mouse(mouse::Event::CursorLeft) => {
                if state.grabbed.take().is_some() {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.grabbed.is_some() => {
                shell.request_redraw();
            }
            _ => {}
        }
        // Wheel events still reach the scrollable. Other project controls don't
        // hover under the drag, and never see the grip's press/release.
        let cursor = if state.grabbed.is_some() {
            cursor.levitate()
        } else {
            cursor
        };
        self.column.update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<State>().grabbed.is_some() && cursor.is_over(*viewport) {
            mouse::Interaction::Grabbing
        } else if self.ids.len() > 1 && self.grip_at(layout, cursor, *viewport).is_some() {
            mouse::Interaction::Grab
        } else {
            self.column
                .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;

        let grabbed = tree.state.downcast_ref::<State>().grabbed;
        self.column.draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            if grabbed.is_some() {
                cursor.levitate()
            } else {
                cursor
            },
            viewport,
        );
        if let Some((_, line_y)) =
            grabbed.and_then(|source| self.drop_slot(source, layout, cursor, *viewport))
        {
            let bounds = layout.bounds();
            // A scroll position may clip part of an expanded group. In that case
            // show the insertion line at the visible edge of the viewport.
            if viewport.height >= LINE_HEIGHT {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: bounds.x,
                            y: line_y.clamp(viewport.y, viewport.y + viewport.height - LINE_HEIGHT),
                            width: bounds.width,
                            height: LINE_HEIGHT,
                        },
                        border: iced::border::rounded(1),
                        ..Default::default()
                    },
                    self.accent,
                );
            }
        }
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.column.overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::renderer::Headless;
    use iced::widget::{Space, button, column, scrollable};
    use iced::{Font, Point};

    #[test]
    fn dragging_a_project_only_reorders_on_a_valid_drop() {
        let renderer = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(Renderer::new(Font::DEFAULT, 13.into(), Some("tiny-skia")))
            .unwrap();
        let [first, second, third, last] = [ProjectId(1), ProjectId(2), ProjectId(3), ProjectId(4)];
        let header = |id| {
            button(Space::new())
                .height(24)
                .width(Fill)
                .on_press(Message::ToggleProject(id))
        };
        let projects = vec![
            (
                first,
                column![header(first), Space::new().height(64)].into(),
            ),
            (second, column![header(second)].into()),
            (third, column![header(third)].into()),
            (last, column![header(last), Space::new().height(90)].into()),
        ];
        let mut list: Element<'_, Message> = scrollable(reorderable(projects, Color::WHITE))
            .width(Fill)
            .height(Fill)
            .into();
        let viewport = Rectangle::with_size(Size::new(192.0, 180.0));
        let mut tree = Tree::new(list.as_widget());
        let node = list.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, viewport.size()),
        );
        let mut send = |event: Event, x, y| {
            let mut messages = Vec::new();
            list.as_widget_mut().update(
                &mut tree,
                &event,
                Layout::new(&node),
                mouse::Cursor::Available(Point::new(x, y)),
                &renderer,
                &mut iced::advanced::clipboard::Null,
                &mut Shell::new(&mut messages),
                &viewport,
            );
            messages
        };
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));

        // A click on the header still toggles; a click on the grip does neither.
        assert!(send(press.clone(), 50.0, 100.0).is_empty());
        assert!(
            matches!(&send(release.clone(), 50.0, 100.0)[..], [Message::ToggleProject(id)] if *id == second)
        );
        assert!(send(press.clone(), 10.0, 125.0).is_empty());
        assert!(send(release.clone(), 10.0, 125.0).is_empty());

        // The upper half of an expanded group's worktree area belongs before
        // that group, not before or after one of its individual worktrees.
        assert!(send(press.clone(), 10.0, 125.0).is_empty());
        assert!(
            matches!(&send(release.clone(), 50.0, 34.0)[..], [Message::ReorderProject { source, before: Some(before) }] if *source == third && *before == first)
        );
        assert!(send(press.clone(), 10.0, 100.0).is_empty());
        assert!(send(release.clone(), 210.0, 30.0).is_empty());

        let escape = keyboard::Key::Named(keyboard::key::Named::Escape);
        assert!(send(press.clone(), 10.0, 100.0).is_empty());
        assert!(
            send(
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: escape.clone(),
                    modified_key: escape,
                    physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Escape),
                    location: keyboard::Location::Standard,
                    modifiers: keyboard::Modifiers::empty(),
                    text: None,
                    repeat: false,
                }),
                10.0,
                32.0
            )
            .is_empty()
        );
        assert!(send(release.clone(), 10.0, 32.0).is_empty());

        // Scroll while holding the grip to reach the end of a list that
        // doesn't fit on screen. The drop uses the new scroll position.
        assert!(send(press, 10.0, 100.0).is_empty());
        assert!(
            send(
                Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -100.0 },
                }),
                50.0,
                169.0,
            )
            .is_empty()
        );
        assert!(
            matches!(&send(release, 50.0, 169.0)[..], [Message::ReorderProject { source, before: None }] if *source == second)
        );
    }
}
