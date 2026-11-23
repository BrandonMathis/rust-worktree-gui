//! The sidebar: project and worktree rows, its toolbar, and the hover preview
//! shown while it is hidden.

use super::widgets::{icon_button, settings_button, single_line, with_row_menu};
use super::{App, Message, Project, Selection};
use crate::create::ProjectChoice;
use crate::git::Worktree;
use crate::style::{self, Button, Panel};
use crate::theme::Palette;
use crate::{icons, pull_request, row_menu};
use iced::widget::{
    Space, Svg, button, column, container, mouse_area, row, scrollable, text, tooltip,
};
use iced::{Center, Element, Fill, Task};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub enum SidebarZone {
    Toggle,
    Panel,
}

/// The overlay shown while the pointer is over the show-sidebar toggle or the
/// overlay itself. `epoch` changes on every hover event, so a delayed "hide"
/// from an earlier event can tell it is stale.
#[derive(Default)]
pub(super) struct SidebarPreview {
    pub(super) visible: bool,
    over_toggle: bool,
    over_panel: bool,
    pub(super) epoch: u64,
}

impl SidebarPreview {
    /// Records the pointer entering or leaving `zone`. Returns the epoch to
    /// hide at, after a delay, once the pointer has left both zones.
    fn hover(&mut self, zone: SidebarZone, entered: bool) -> Option<u64> {
        match zone {
            SidebarZone::Toggle => self.over_toggle = entered,
            SidebarZone::Panel => self.over_panel = entered,
        }
        self.epoch += 1;
        if self.over_toggle || self.over_panel {
            self.visible = true;
            None
        } else {
            self.visible.then_some(self.epoch)
        }
    }

    fn hide(&mut self) {
        self.epoch += 1;
        self.visible = false;
        self.over_toggle = false;
        self.over_panel = false;
    }
}

/// The one status icon shown left of each worktree in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorktreeIcon {
    Unavailable,
    Main,
    Worktree,
    PullRequest(pull_request::Status),
}

impl WorktreeIcon {
    fn view<'a>(self, size: u32, p: Palette) -> Svg<'a> {
        match self {
            Self::Unavailable => icons::warning(size, p.red),
            Self::Main => icons::home(size, p.orange),
            Self::Worktree => icons::branch(size, p.cyan),
            Self::PullRequest(status) => icons::pr_status(size, status, p),
        }
    }
}

impl Project {
    /// Picks the single sidebar icon for `tree`. Precedence: unavailable, then
    /// the main checkout, then the PR state, then a plain worktree.
    pub(super) fn worktree_icon(&self, tree: &Worktree) -> WorktreeIcon {
        // `git worktree list` always lists the main worktree first.
        let is_main = self.worktrees().first().map(|main| &main.path) == Some(&tree.path);
        if tree.unavailable_reason().is_some() {
            WorktreeIcon::Unavailable
        } else if is_main {
            WorktreeIcon::Main
        } else if let Some(pr) = self.pull_request(&tree.path) {
            WorktreeIcon::PullRequest(pr.status)
        } else {
            WorktreeIcon::Worktree
        }
    }
}

impl App {
    pub(super) fn toggle_sidebar(&mut self) -> Task<Message> {
        self.sidebar_visible = !self.sidebar_visible;
        self.sidebar_preview.hide();
        if !self.sidebar_visible {
            self.cancel_sidebar_rename();
        }
        self.focus_terminal()
    }

    pub(super) fn sidebar_hover(&mut self, zone: SidebarZone, entered: bool) -> Task<Message> {
        if !self.sidebar_visible
            && let Some(epoch) = self.sidebar_preview.hover(zone, entered)
        {
            // Give the pointer time to cross the gap from the toggle to the panel.
            return Task::perform(
                async { tokio::time::sleep(Duration::from_millis(220)).await },
                move |_| Message::SidebarPreviewExpired(epoch),
            );
        }
        Task::none()
    }

    pub(super) fn sidebar_preview_expired(&mut self, epoch: u64) -> Task<Message> {
        if !self.sidebar_visible && self.sidebar_preview.epoch == epoch {
            self.sidebar_preview.hide();
            if self.cancel_sidebar_rename() {
                return self.focus_terminal();
            }
        }
        Task::none()
    }

    /// Drops a rename draft opened from a sidebar row (not the header's).
    /// Returns whether one was dropped.
    fn cancel_sidebar_rename(&mut self) -> bool {
        if self
            .worktree_name_draft
            .as_ref()
            .is_some_and(|draft| self.selection.as_ref() != Some(&draft.target))
        {
            self.worktree_name_draft = None;
            true
        } else {
            false
        }
    }

    pub(super) fn sidebar(&self) -> Element<'_, Message> {
        let p = self.palette();
        let refreshing = self.projects.iter().any(|p| p.refresh_id.is_some());
        let project_menu = if self.can_start_creation() {
            row_menu::add(
                vec![ProjectChoice::New, ProjectChoice::Existing],
                Message::ProjectChoice,
                p,
            )
        } else {
            icon_button(icons::add(14, p.muted()), "Add Project", None, p)
        };
        let toolbar = row![
            single_line("Projects", 13),
            project_menu,
            icon_button(
                icons::connecting(14, p.muted()),
                if refreshing {
                    "Refreshing…"
                } else {
                    "Refresh worktrees and PR"
                },
                (!self.projects.is_empty() && !refreshing).then_some(Message::Refresh),
                p,
            ),
            icon_button(
                icons::sidebar(14, p.muted()),
                if self.sidebar_visible {
                    "Hide sidebar"
                } else {
                    "Pin sidebar"
                },
                Some(Message::ToggleSidebar),
                p,
            ),
        ]
        .spacing(2)
        .align_y(Center);
        let mut list = column![].spacing(2);
        if self.projects.is_empty() {
            list = list.push(
                text(if self.store.is_some() {
                    "No bookmarked projects"
                } else {
                    "Bookmarks are unavailable"
                })
                .size(13)
                .color(p.muted()),
            );
        }
        // Struct update syntax (`..base`) copies the user's font family and only
        // changes the weight, like `{ ...font, weight: 'bold' }` in JS.
        let bold = iced::Font {
            weight: iced::font::Weight::Bold,
            ..self.terminal_preferences.font.font_type
        };
        for project in &self.projects {
            let selection = self.selection.as_ref().filter(|s| s.project == project.id);
            let caret = if project.expanded {
                icons::chevron_down(15, p.foreground)
            } else {
                icons::chevron_right(15, p.foreground)
            };
            let name = project.name().replace(['\n', '\r'], " ");
            let project_button = button(
                row![
                    caret,
                    // Like `single_line`, but bold so projects stand apart from worktrees.
                    container(
                        text(name)
                            .size(13)
                            .font(bold)
                            .wrapping(iced::widget::text::Wrapping::None)
                    )
                    .clip(true)
                    .width(Fill),
                    text(if project.refresh_error.is_some() {
                        "!"
                    } else {
                        ""
                    })
                    .color(p.red),
                    Space::new().width(22),
                ]
                .spacing(4)
                .align_y(Center),
            )
            .width(Fill)
            .padding([5, 4])
            .on_press(Message::ToggleProject(project.id))
            .style(if selection.is_some() {
                style::button(p, Button::Secondary)
            } else {
                style::button(p, Button::Project)
            });
            list = list.push(with_row_menu(
                project_button,
                Selection {
                    project: project.id,
                    worktree: None,
                },
                p,
            ));
            if !project.expanded {
                continue;
            }
            let mut children =
                column![container(text("Worktrees").size(10).color(p.muted())).padding([1, 4]),]
                    .spacing(1);
            if project.worktrees().is_empty() {
                children = children.push(
                    text(if project.refresh_id.is_some() {
                        "Reading Git…"
                    } else {
                        "No worktrees to show"
                    })
                    .size(12)
                    .color(p.muted()),
                );
            }
            for tree in project.worktrees() {
                let active =
                    selection.and_then(|s| s.worktree.as_deref()) == Some(tree.path.as_path());
                let pr = project.pull_request(&tree.path);
                let label = row![
                    project.worktree_icon(tree).view(13, p),
                    single_line(project.worktree_name(tree), 12),
                ]
                .spacing(5)
                .align_y(Center);
                let mut details = column![
                    text(project.worktree_name(tree)).size(12),
                    text(format!("{}  ·  {}", tree.label(), tree.path.display()))
                        .size(11)
                        .color(p.muted()),
                ]
                .spacing(3);
                if let Some(pr) = pr {
                    details = details.push(text(pr.label()).size(11));
                }
                let label = tooltip(
                    label,
                    container(details)
                        .max_width(500)
                        .padding(7)
                        .style(style::panel(p, Panel::Console)),
                    tooltip::Position::Right,
                )
                .delay(std::time::Duration::from_millis(600));
                let base = button(row![label, Space::new().width(22)])
                    .width(Fill)
                    .padding([3, 5])
                    .style(if active {
                        style::button(p, Button::Selected)
                    } else {
                        style::button(p, Button::Nav)
                    })
                    .on_press(Message::SelectWorktree(project.id, tree.path.clone()));
                let target = Selection {
                    project: project.id,
                    worktree: Some(tree.path.clone()),
                };
                let editor = if active {
                    None
                } else {
                    self.rename_editor(&target, tree.label(), true)
                };
                children = children.push(with_row_menu(base, target, p));
                if let Some(editor) = editor {
                    children = children.push(editor);
                }
            }
            // Indent worktrees under their project so the hierarchy reads at a glance.
            list = list.push(
                container(children)
                    .padding(iced::padding::top(1).bottom(4).left(12))
                    .width(Fill),
            );
        }
        // The Settings gear sits in the bottom-left corner, below the list.
        container(
            column![
                toolbar,
                scrollable(list.width(Fill)).height(Fill),
                settings_button(p),
            ]
            .spacing(5),
        )
        .padding(6)
        .width(202)
        .height(Fill)
        .style(if self.sidebar_visible {
            style::panel(p, Panel::Glass)
        } else {
            style::panel(p, Panel::Floating)
        })
        .into()
    }

    /// The header button that shows the hidden sidebar, and reveals it as an
    /// overlay on hover. Empty while the sidebar is visible.
    pub(super) fn show_sidebar_button(&self) -> Element<'static, Message> {
        let p = self.palette();
        if self.sidebar_visible {
            Space::new().into()
        } else {
            mouse_area(
                button(icons::sidebar(15, p.purple))
                    .padding(4)
                    .style(style::button(p, Button::Nav))
                    .on_press(Message::ToggleSidebar),
            )
            .on_enter(Message::SidebarHover(SidebarZone::Toggle, true))
            .on_exit(Message::SidebarHover(SidebarZone::Toggle, false))
            .into()
        }
    }
}
