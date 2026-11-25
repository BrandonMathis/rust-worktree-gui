//! The sidebar: project and worktree rows, its toolbar, and the hover preview
//! shown while it is hidden.

use super::project_list::{GRIP_WIDTH, reorderable};
use super::widgets::{MenuTarget, icon_button, settings_button, single_line, with_row_menu};
use super::{App, Message, Project, ProjectId, Selection};
use crate::create::ProjectChoice;
use crate::git::Worktree;
use crate::style::{self, Button, Corners, Panel};
use crate::theme::Palette;
use crate::{icons, pull_request, row_menu};
use iced::widget::{
    Space, Svg, button, column, container, mouse_area, row, scrollable, text, tooltip,
};
use iced::{Center, Element, Fill, Task};
use std::time::Duration;

// Sidebar width limits in logical pixels. The default is the old fixed width.
pub(super) const SIDEBAR_DEFAULT_WIDTH: f32 = 202.0;
pub(super) const SIDEBAR_MIN_WIDTH: f32 = 160.0;
pub(super) const SIDEBAR_MAX_WIDTH: f32 = 480.0;
/// The window margin left of the sidebar plus half the 4px drag handle, so
/// the edge tracks the cursor instead of jumping when a drag starts.
pub(super) const SIDEBAR_EDGE_OFFSET: f32 = style::WINDOW_GAP + 2.0;

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

/// The red `!` on a project whose last refresh failed. Its tooltip carries
/// the error and what to do, because the project overview (which also shows
/// it) can't be reached while a worktree is selected.
fn refresh_error_marker(error: Option<&str>, p: Palette) -> Element<'_, Message> {
    let Some(error) = error else {
        return text("").into();
    };
    let details = column![
        text("Could not read Git").size(12).color(p.red),
        text(error).size(11),
        text("Refresh to try again, or use ⋮ → Replace Anchor… to pick a live checkout.")
            .size(11)
            .color(p.muted()),
    ]
    .spacing(3);
    tooltip(
        text("!").color(p.red),
        container(details)
            .max_width(420)
            .padding(7)
            .style(style::panel(p, Panel::Console)),
        tooltip::Position::Right,
    )
    .into()
}

/// The row under a long worktree list: "Show N more" while some are hidden,
/// "Show fewer" once the whole list is shown.
fn show_more_button(project: ProjectId, hidden: usize, p: Palette) -> Element<'static, Message> {
    let (icon, label, show_all) = if hidden > 0 {
        (
            icons::chevron_down(12, p.muted()),
            format!("Show {hidden} more"),
            true,
        )
    } else {
        (
            icons::chevron_up(12, p.muted()),
            "Show fewer".to_owned(),
            false,
        )
    };
    button(
        row![icon, text(label).size(11).color(p.muted())]
            .spacing(5)
            .align_y(Center),
    )
    .width(Fill)
    .padding([2, 5])
    .style(style::button(p, Button::Nav))
    .on_press(Message::ShowAllWorktrees(project, show_all))
    .into()
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

    /// Starts dragging the sidebar edge. A hidden sidebar has no handle.
    pub(super) fn sidebar_resize_started(&mut self) {
        self.sidebar_resizing = self.sidebar_visible;
    }

    /// Follows the cursor (window x coordinate) while a drag is in progress.
    pub(super) fn sidebar_resized(&mut self, x: f32) {
        if self.sidebar_resizing {
            self.sidebar_width =
                (x - SIDEBAR_EDGE_OFFSET).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        }
    }

    pub(super) fn sidebar_resize_ended(&mut self) -> Task<Message> {
        if std::mem::take(&mut self.sidebar_resizing) {
            return self.focus_terminal();
        }
        Task::none()
    }

    pub(super) fn sidebar_resize_reset(&mut self) {
        self.sidebar_width = SIDEBAR_DEFAULT_WIDTH;
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

    pub(super) fn sidebar(&self, corners: Corners) -> Element<'_, Message> {
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
        let mut groups = Vec::with_capacity(self.projects.len());
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
            let mut header = row![].align_y(Center);
            if self.projects.len() > 1 {
                // The button's 4px left padding and the grip slot together
                // define the grab area (see project_list::GRIP_WIDTH).
                header =
                    header.push(container(icons::grip(13, p.muted())).center_x(GRIP_WIDTH - 4));
            }
            let project_button = button(
                header.push(
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
                        refresh_error_marker(project.refresh_error.as_deref(), p),
                        Space::new().width(22),
                    ]
                    .spacing(4)
                    .align_y(Center),
                ),
            )
            .width(Fill)
            .padding([5, 4])
            .on_press(Message::ToggleProject(project.id))
            .style(if selection.is_some() {
                style::button(p, Button::Secondary)
            } else {
                style::button(p, Button::Project)
            });
            let group = column![with_row_menu(
                project_button,
                Selection {
                    project: project.id,
                    worktree: None,
                },
                MenuTarget::Project {
                    unavailable: project.unavailable_worktrees().len(),
                },
                p,
            )]
            .spacing(2)
            .width(Fill);
            if !project.expanded {
                groups.push((project.id, group.into()));
                continue;
            }
            let mut children = column![].spacing(1);
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
            let selected_path = selection.and_then(|s| s.worktree.as_deref());
            let listing = project.sidebar_rows(selected_path);
            for tree in listing.rows {
                let active = selected_path == Some(tree.path.as_path());
                let pr = project.pull_request(&tree.path);
                let managed = project.is_managed(&tree.path);
                // A managed worktree shows its agent's state, not a tmux session's.
                let agent = self.agent_badge(project, &tree.path);
                let session = project
                    .session_badge(&tree.path)
                    .filter(|_| agent.is_none());
                let mut label = row![
                    project.worktree_icon(tree).view(13, p),
                    single_line(project.worktree_name(tree), 12),
                ]
                .spacing(5)
                .align_y(Center);
                // The session badge sits on the right, so the left keeps exactly one icon.
                if let Some(badge) = session {
                    label = label.push(badge.view(11, p));
                }
                if let Some(badge) = agent {
                    label = label.push(badge.view(11, p));
                }
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
                if let Some(badge) = session {
                    details = details.push(text(badge.label()).size(11).color(p.muted()));
                }
                if let Some(badge) = agent {
                    details = details.push(text(badge.label()).size(11).color(p.muted()));
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
                children = children.push(with_row_menu(
                    base,
                    target,
                    MenuTarget::Worktree { managed },
                    p,
                ));
                if let Some(editor) = editor {
                    children = children.push(editor);
                }
            }
            if listing.collapsible {
                children = children.push(show_more_button(project.id, listing.hidden, p));
            }
            // Indent worktrees under their project so the hierarchy reads at a glance.
            groups.push((
                project.id,
                group
                    .push(
                        container(children)
                            .padding(iced::padding::top(1).bottom(4).left(12))
                            .width(Fill),
                    )
                    .into(),
            ));
        }
        let list = if groups.is_empty() {
            text(if self.store.is_some() {
                "No bookmarked projects"
            } else {
                "Bookmarks are unavailable"
            })
            .size(13)
            .color(p.muted())
            .into()
        } else {
            reorderable(groups, p.purple)
        };
        // The Settings gear sits in the bottom-left corner, below the list.
        container(
            column![
                toolbar,
                // No visible scrollbar: it overlapped the row ⋮ menus. The list
                // still scrolls with the wheel and trackpad.
                scrollable(list)
                    .direction(scrollable::Direction::Vertical(
                        scrollable::Scrollbar::hidden(),
                    ))
                    .width(Fill)
                    .height(Fill),
                settings_button(p),
            ]
            .spacing(5),
        )
        .padding(6)
        .width(self.sidebar_width)
        .height(Fill)
        .style(style::panel_in(
            p,
            if self.sidebar_visible {
                Panel::Glass
            } else {
                Panel::Floating
            },
            corners,
        ))
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
