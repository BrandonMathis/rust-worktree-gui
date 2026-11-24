//! The modal that lists a project's unavailable worktrees. The sidebar leaves
//! them out (see `Project::sidebar_rows`); the project's ⋮ menu opens this
//! list instead. It only shows them: removing or pruning worktrees is left to
//! Git, because the app never destroys user state.

use super::widgets::single_line;
use super::{App, Message};
use crate::icons;
use crate::style::{self, Button, Panel};
use iced::widget::{button, center, column, container, mouse_area, opaque, row, scrollable, text};
use iced::{Center, Element, Fill};

impl App {
    /// The modal over the window, while one is open for a bookmarked project.
    pub(super) fn unavailable_modal(&self) -> Option<Element<'_, Message>> {
        let id = self.unavailable_list?;
        let project = self.projects.iter().find(|project| project.id == id)?;
        let p = self.palette();
        let trees = project.unavailable_worktrees();
        let mut list = column![].spacing(8);
        if trees.is_empty() {
            list = list.push(
                text("All worktrees are available.")
                    .size(12)
                    .color(p.muted()),
            );
        }
        for tree in trees {
            list = list.push(
                column![
                    row![
                        icons::warning(13, p.red),
                        single_line(project.worktree_name(tree), 12),
                    ]
                    .spacing(5)
                    .align_y(Center),
                    container(
                        column![
                            text(tree.unavailable_reason().unwrap_or_default())
                                .size(11)
                                .color(p.red),
                            text(tree.path.display().to_string())
                                .size(11)
                                .color(p.muted()),
                        ]
                        .spacing(2),
                    )
                    .padding(iced::padding::left(18)),
                ]
                .spacing(2),
            );
        }
        let title = format!("Unavailable worktrees in {}", project.name());
        let panel = container(
            column![
                single_line(title.replace(['\n', '\r'], " "), 14),
                text("Git still lists these, but they can't open a terminal.")
                    .size(11)
                    .color(p.muted()),
                // `scrollable` keeps a long list inside the panel.
                container(scrollable(list).width(Fill)).max_height(360),
                container(
                    button(text("Close").size(12))
                        .style(style::button(p, Button::Secondary))
                        .on_press(Message::CloseUnavailableWorktrees),
                )
                .align_right(Fill),
            ]
            .spacing(10),
        )
        .padding(14)
        .max_width(460)
        .style(style::panel(p, Panel::Floating));
        // `opaque` stops clicks on the panel from reaching the backdrop (or the
        // app under it); a click on the dimmed backdrop closes the modal.
        let backdrop = mouse_area(center(opaque(panel)).padding(16).style(style::scrim(p)))
            .on_press(Message::CloseUnavailableWorktrees);
        Some(opaque(backdrop))
    }
}

/// Escape closes the modal (subscribed only while it is open).
pub(super) fn modal_keys(
    event: iced::Event,
    _: iced::event::Status,
    _: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(Message::CloseUnavailableWorktrees),
        _ => None,
    }
}
