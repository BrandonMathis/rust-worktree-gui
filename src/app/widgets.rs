//! Small view helpers shared by the sidebar and the workspace.

use super::{MenuAction, Message, Selection};
use crate::actions::DirectoryAction;
use crate::style::{self, Button, Panel};
use crate::theme::Palette;
use crate::{icons, row_menu};
use iced::widget::{Svg, button, container, hover, text, tooltip};
use iced::{Element, Fill};

/// The actions offered in a sidebar row's ⋮ menu, for a worktree
/// (`worktree == true`) or a project. A project with `unavailable` worktrees
/// (hidden from the sidebar) also gets an item that lists them.
pub(super) fn menu_actions(worktree: bool, unavailable: usize) -> Vec<MenuAction> {
    let mut actions = Vec::new();
    if !worktree {
        actions.push(MenuAction::NewWorktree);
    }
    actions.extend(DirectoryAction::ALL.map(MenuAction::Directory));
    if worktree {
        actions.push(MenuAction::RenameWorktree);
    } else {
        if unavailable > 0 {
            actions.push(MenuAction::UnavailableWorktrees(unavailable));
        }
        actions.extend([MenuAction::ReplaceAnchor, MenuAction::RemoveProject]);
    }
    actions
}

/// Overlays the ⋮ row menu on the right edge of `base` while it is hovered.
/// `unavailable` counts a project's unavailable worktrees (0 for a worktree row).
pub(super) fn with_row_menu<'a>(
    base: impl Into<Element<'a, Message>>,
    target: Selection,
    unavailable: usize,
    p: Palette,
) -> Element<'a, Message> {
    let menu = row_menu::show(
        format!("{target:?}"),
        menu_actions(target.worktree.is_some(), unavailable),
        move |action| Message::MenuAction(target.clone(), action),
        p,
    );
    hover(
        base,
        container(menu)
            .padding([0, 3])
            .align_right(Fill)
            .center_y(Fill),
    )
}

/// A label that never wraps; overflow is clipped.
pub(super) fn single_line<'a>(
    label: impl iced::widget::text::IntoFragment<'a>,
    size: u32,
) -> iced::widget::Container<'a, Message> {
    container(
        text(label)
            .size(size)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .clip(true)
    .width(Fill)
}

/// An icon-only toolbar button with a tooltip. `None` disables it.
pub(super) fn icon_button(
    icon: Svg<'static>,
    label: &'static str,
    message: Option<Message>,
    p: Palette,
) -> Element<'static, Message> {
    tooltip(
        button(icon)
            .padding(4)
            .style(style::button(p, Button::Nav))
            .on_press_maybe(message),
        container(text(label).size(11))
            .padding([3, 5])
            .style(style::panel(p, Panel::Console)),
        tooltip::Position::Bottom,
    )
    .delay(std::time::Duration::from_millis(400))
    .into()
}

/// The gear in the sidebar's bottom-left corner that opens the Settings modal.
pub(super) fn settings_button(p: Palette) -> Element<'static, Message> {
    icon_button(
        icons::settings(14, p.muted()),
        "Settings",
        Some(Message::OpenSettings),
        p,
    )
}
