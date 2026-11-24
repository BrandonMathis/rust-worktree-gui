//! `App::view`: the window layout, the workspace beside the sidebar, and the
//! worktree header.

use super::widgets::single_line;
use super::{App, Message, Project, Selection, SidebarZone, WORKTREE_NAME_INPUT, pull_requests};
use crate::create::ProjectChoice;
use crate::git::Worktree;
use crate::icons;
use crate::style::{self, Button, Corners, Panel};
use crate::theme::Palette;
use iced::widget::{
    Space, button, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input,
    tooltip,
};
use iced::{Center, Element, Fill};

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        let p = self.palette();
        // Whatever sits along the bottom edge takes the window's corner rounding:
        // the notice bar when shown, otherwise the content above it.
        let corners = if self.notice.is_some() {
            Corners::NONE
        } else {
            self.window_corners
        };
        let content = match &self.settings {
            // Settings replaces the whole window rather than floating over it,
            // so the terminal underneath can't receive keystrokes meant for it.
            Some(draft) => draft.view(corners).map(Message::Settings),
            None => self.main_layout(corners),
        };
        let mut page = column![content].spacing(4);
        if let Some(notice) = &self.notice {
            page = page.push(
                container(
                    row![
                        text(notice).size(13).width(Fill),
                        button("Dismiss")
                            .on_press(Message::DismissNotice)
                            .style(style::button(p, Button::Secondary)),
                    ]
                    .spacing(12)
                    .align_y(Center),
                )
                .padding(10)
                .width(Fill)
                .style(style::panel_in(p, Panel::Glass, self.window_corners)),
            );
        }
        container(page)
            .padding(style::WINDOW_GAP)
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// The sidebar and workspace, with the sidebar preview overlay.
    fn main_layout(&self, corners: Corners) -> Element<'_, Message> {
        // Keep the workspace in the same widget slot when toggling the sidebar.
        let (sidebar, workspace) = if self.sidebar_visible {
            (
                self.sidebar(corners.left()),
                self.workspace(corners.right()),
            )
        } else {
            (Space::new().into(), self.workspace(corners))
        };
        let workspace = row![sidebar, workspace]
            .spacing(if self.sidebar_visible { 4 } else { 0 })
            .height(Fill);
        let mut workspace = stack![workspace];
        if !self.sidebar_visible && self.sidebar_preview.visible {
            let panel = opaque(
                mouse_area(self.sidebar(corners.left()))
                    .on_enter(Message::SidebarHover(SidebarZone::Panel, true))
                    .on_exit(Message::SidebarHover(SidebarZone::Panel, false)),
            );
            // The stack keeps its base layout; the top layer only paints over the terminal.
            workspace = workspace.push(column![Space::new().height(36), panel]);
        }
        if let Some(modal) = self.unavailable_modal() {
            workspace = workspace.push(modal);
        }
        workspace.into()
    }

    fn workspace(&self, corners: Corners) -> Element<'_, Message> {
        let p = self.palette();
        if let Some(draft) = &self.creation {
            return self.creation_form(draft, corners);
        }
        match self.selected() {
            Some((project, selection)) => match selection
                .worktree
                .as_deref()
                .and_then(|p| project.worktree(p))
            {
                Some(tree) => self.worktree_workspace(project, tree, corners),
                None => self.project_status(project, corners),
            },
            None => container(
                column![
                    row![self.show_sidebar_button(), text("Open a project").size(20)]
                        .spacing(if self.sidebar_visible { 0 } else { 5 })
                        .align_y(Center),
                    text("Create a project or add an existing Git repository.").color(p.muted()),
                    row![
                        button(text("New Project…").size(13))
                            .style(style::button(p, Button::Primary))
                            .on_press_maybe(
                                self.can_start_creation()
                                    .then_some(Message::ProjectChoice(ProjectChoice::New))
                            ),
                        button(text("Add Existing Repository…").size(13))
                            .style(style::button(p, Button::Secondary))
                            .on_press_maybe(
                                self.can_start_creation()
                                    .then_some(Message::ProjectChoice(ProjectChoice::Existing))
                            ),
                    ]
                    .spacing(6),
                ]
                .spacing(7),
            )
            .padding(12)
            .width(Fill)
            .height(Fill)
            .style(style::panel_in(p, Panel::Glass, corners))
            .into(),
        }
    }

    fn project_status<'a>(&self, project: &'a Project, corners: Corners) -> Element<'a, Message> {
        let p = self.palette();
        // A refresh error wins over the (possibly stale) last good inventory.
        let message = match (&project.refresh_error, &project.inventory) {
            (Some(error), _) => column![
                text("Checkout unavailable").color(p.red),
                text(error).size(14),
                text("Replace the anchor with a live checkout of the same repository.")
                    .size(14)
                    .color(p.muted())
            ]
            .spacing(9),
            (None, Some(trees)) => column![
                text(format!(
                    "{} worktrees  ·  Select one to open its terminal.",
                    trees.len()
                ))
                .color(p.muted())
            ],
            (None, None) => column![text("Loading worktrees…").color(p.muted())],
        };
        container(scrollable(
            column![
                row![
                    if self.sidebar_visible {
                        icons::folder(16, p.purple).into()
                    } else {
                        self.show_sidebar_button()
                    },
                    single_line(project.name(), 15),
                ]
                .spacing(7)
                .align_y(Center),
                text(project.checkout.root.display().to_string())
                    .size(13)
                    .color(p.muted()),
                message,
            ]
            .spacing(8)
            .padding(10)
            .width(Fill),
        ))
        .width(Fill)
        .height(Fill)
        .style(style::panel_in(p, Panel::Glass, corners))
        .into()
    }

    /// The inline display-name editor for `target`, if a rename draft is open
    /// for it. `compact` stacks the buttons for the narrow sidebar.
    pub(super) fn rename_editor<'a>(
        &'a self,
        target: &Selection,
        branch: &'a str,
        compact: bool,
    ) -> Option<Element<'a, Message>> {
        let p = self.palette();
        let draft = self
            .worktree_name_draft
            .as_ref()
            .filter(|draft| draft.target == *target)?;
        let input = text_input(branch, &draft.input)
            .id(WORKTREE_NAME_INPUT)
            .size(13)
            .padding([4, 7])
            .on_input(Message::EditWorktreeName)
            .on_submit(Message::SaveWorktreeName);
        let controls = row![
            button(text("Save").size(12))
                .style(style::button(p, Button::Primary))
                .on_press(Message::SaveWorktreeName),
            button(text("Cancel").size(12))
                .style(style::button(p, Button::Secondary))
                .on_press(Message::CancelWorktreeName),
        ]
        .spacing(4);
        Some(if compact {
            container(column![input, controls].spacing(4))
                .padding([2, 4])
                .into()
        } else {
            column![
                row![input, controls].spacing(6).align_y(Center),
                text("Display name only. Clear it to use the Git branch name.")
                    .size(11)
                    .color(p.muted()),
            ]
            .spacing(2)
            .into()
        })
    }

    /// A selected worktree: its one-row header and the terminal or agent pane.
    fn worktree_workspace<'a>(
        &'a self,
        project: &'a Project,
        tree: &'a Worktree,
        corners: Corners,
    ) -> Element<'a, Message> {
        let p = self.palette();
        let name = project.worktree_name(tree);
        let mut actions = row![].spacing(6).align_y(Center);
        if let Some(pr) = project.pull_request(&tree.path) {
            actions = actions.push(pull_requests::badge(pr, p));
        }
        let title = tooltip(
            row![icons::branch(14, p.purple), single_line(name, 14)]
                .spacing(5)
                .width(Fill)
                .align_y(Center),
            container(
                column![
                    text(project.name()).size(12),
                    text(format!(
                        "{}  {}",
                        tree.label(),
                        tree.head
                            .as_deref()
                            .and_then(|head| head.get(..8))
                            .unwrap_or("")
                    ))
                    .size(11)
                    .color(p.muted()),
                    text(tree.path.display().to_string())
                        .size(11)
                        .color(p.muted()),
                ]
                .spacing(2),
            )
            .padding(6)
            .max_width(500)
            .style(style::panel(p, Panel::Console)),
            tooltip::Position::Bottom,
        )
        .delay(std::time::Duration::from_millis(500));
        let target = Selection {
            project: project.id,
            worktree: Some(tree.path.clone()),
        };
        let editor = self.rename_editor(&target, tree.label(), false);
        // A managed worktree shows its agent; an unavailable one explains
        // itself through the terminal pane either way.
        let managed = project.is_managed(&tree.path);
        let (terminal_status, body) = if managed && tree.unavailable_reason().is_none() {
            self.agent_pane(tree)
        } else {
            self.terminal_pane(tree, corners)
        };
        let heading = row![self.show_sidebar_button(), title, terminal_status, actions]
            .spacing(4)
            .align_y(Center);
        let mut header = column![heading].spacing(3);
        if let Some(editor) = editor {
            header = header.push(editor);
        }
        column![
            container(header)
                .padding([2, 4])
                .width(Fill)
                .style(style::panel(p, Panel::Glass)),
            container(body)
                .width(Fill)
                .height(Fill)
                .style(style::panel_in(p, Panel::Console, corners)),
        ]
        .spacing(2)
        .into()
    }
}

/// A title and optional detail filling the terminal pane when no terminal is shown.
pub(super) fn placeholder<'a>(
    title: &'a str,
    detail: Option<&'a str>,
    p: Palette,
) -> Element<'a, Message> {
    let mut body = column![text(title).size(19)].spacing(6);
    if let Some(detail) = detail {
        body = body.push(text(detail).size(14).color(p.muted()));
    }
    container(body).padding(6).width(Fill).height(Fill).into()
}
