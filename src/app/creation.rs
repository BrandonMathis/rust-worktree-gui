//! The New Project / New Worktree flow. The form's own state and edits live in
//! the nested component `create::Draft`; this module opens it, acts on its
//! `Outcome`s, and draws it.

use super::{App, Message, PickMode, ProjectId, background};
use crate::create::{self, ProjectChoice};
use crate::git::{self, Checkout};
use crate::style::{self, Button, Corners, Panel};
use iced::widget::{button, column, container, operation, row, scrollable, text, text_input};
use iced::{Center, Element, Fill, Task};
use std::path::{Path, PathBuf};

const CREATE_NAME_INPUT: &str = "create-name-input";
const CREATE_BRANCH_INPUT: &str = "create-branch-input";

impl App {
    pub(super) fn can_start_creation(&self) -> bool {
        self.store.is_some()
            && !self.picking
            && self.creation.is_none()
            && self.pending_confirmation.is_none()
    }

    pub(super) fn project_choice(&mut self, choice: ProjectChoice) -> Task<Message> {
        if !self.can_start_creation() {
            return Task::none();
        }
        match choice {
            ProjectChoice::Existing => self.update(Message::Pick(PickMode::Add)),
            ProjectChoice::New => {
                let parent = self
                    .selected()
                    .and_then(|(project, _)| project.checkout.root.parent().map(Path::to_path_buf))
                    .or_else(|| {
                        directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
                    })
                    .unwrap_or_default();
                self.creation = Some(create::Draft::new(create::Kind::Project, parent));
                self.notice = None;
                operation::focus(CREATE_NAME_INPUT)
            }
        }
    }

    pub(super) fn new_worktree(&mut self, id: ProjectId) -> Task<Message> {
        if !self.can_start_creation() {
            return Task::none();
        }
        if let Some(project) = self.projects.iter().find(|p| p.id == id)
            && project.inventory.is_some()
            && project.refresh_error.is_none()
        {
            let source = self
                .selection
                .as_ref()
                .filter(|selection| selection.project == id)
                .and_then(|selection| project.worktree(selection.worktree.as_deref()?))
                .filter(|tree| tree.unavailable_reason().is_none())
                .map_or_else(|| project.checkout.root.clone(), |tree| tree.path.clone());
            let parent = project
                .checkout
                .root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            self.creation = Some(create::Draft::new(
                create::Kind::Worktree {
                    project: id,
                    anchor: project.checkout.clone(),
                    source,
                },
                parent,
            ));
            self.notice = None;
            return operation::focus(CREATE_BRANCH_INPUT);
        }
        self.notice = Some(
            "A live checkout is needed to create a worktree. Refresh or replace the project anchor."
                .into(),
        );
        Task::none()
    }

    pub(super) fn creation_message(&mut self, message: create::Message) -> Task<Message> {
        let Some(draft) = &mut self.creation else {
            return Task::none();
        };
        match draft.update(message) {
            None => Task::none(),
            Some(create::Outcome::Confirm) => self.confirm_creation(),
            Some(create::Outcome::Cancel) => {
                if self.picking || draft.submitting {
                    return Task::none();
                }
                self.creation = None;
                self.notice = None;
                self.focus_terminal()
            }
        }
    }

    fn confirm_creation(&mut self) -> Task<Message> {
        let Some(draft) = self
            .creation
            .as_mut()
            .filter(|draft| !draft.submitting && !self.picking)
        else {
            return Task::none();
        };
        let path = match draft.destination() {
            Ok(path) => path,
            Err(error) => {
                self.notice = Some(error.into());
                return Task::none();
            }
        };
        draft.submitting = true;
        self.notice = None;
        match &draft.kind {
            create::Kind::Project => {
                background(move || git::new_project(&path), Message::ProjectCreated)
            }
            create::Kind::Worktree {
                project,
                anchor,
                source,
            } => {
                let (project, anchor, source) = (*project, anchor.clone(), source.clone());
                let branch = draft.branch.trim().to_owned();
                let base = if draft.base.trim().is_empty() {
                    "HEAD"
                } else {
                    draft.base.trim()
                }
                .to_owned();
                background(
                    move || git::add_worktree(&anchor, &source, &branch, &path, &base),
                    move |result| Message::WorktreeCreated(project, result),
                )
            }
        }
    }

    pub(super) fn project_created(&mut self, result: Result<Checkout, String>) -> Task<Message> {
        match result {
            Ok(checkout) => {
                self.creation = None;
                self.finish_bookmark(PickMode::Add, Ok(checkout))
            }
            Err(error) => {
                self.creation_error(error);
                Task::none()
            }
        }
    }

    pub(super) fn worktree_created(
        &mut self,
        id: ProjectId,
        result: Result<PathBuf, String>,
    ) -> Task<Message> {
        let path = match result {
            Ok(path) => path,
            Err(error) => {
                self.creation_error(error);
                return Task::none();
            }
        };
        let managed = self.creation.take().is_some_and(|draft| draft.managed);
        // Remember the choice before the refresh selects the new worktree, so
        // it opens with the agent pane rather than a terminal.
        if managed && let Err(error) = self.set_managed(id, &path, true) {
            self.notice = Some(error);
        }
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
            project.expanded = true;
            project.open_after_refresh = Some(path);
            project.refresh_id = None; // Discard a snapshot taken before creation.
            return self.refresh(id);
        }
        self.notice = Some(format!(
            "Worktree created at {}, but its project is no longer bookmarked.",
            path.display()
        ));
        Task::none()
    }

    fn creation_error(&mut self, error: String) {
        if let Some(draft) = &mut self.creation {
            draft.submitting = false;
        }
        self.notice = Some(error);
    }

    pub(super) fn creation_form<'a>(
        &'a self,
        draft: &'a create::Draft,
        corners: Corners,
    ) -> Element<'a, Message> {
        let p = self.palette();
        let worktree = matches!(draft.kind, create::Kind::Worktree { .. });
        let title = if worktree {
            "New Worktree"
        } else {
            "New Project"
        };
        let ready = !draft.submitting && !self.picking;
        // Wraps a form message into the app's `Message`, like passing
        // `dispatch` down to a child component in React.
        let edit = |message: fn(String) -> create::Message| {
            ready.then_some(move |value| Message::Creation(message(value)))
        };
        let field = |label: &'static str, input: Element<'a, Message>| {
            column![text(label).size(12).color(p.muted()), input].spacing(3)
        };
        let mut form = column![
            row![self.show_sidebar_button(), text(title).size(18)]
                .spacing(4)
                .align_y(Center),
            text(if worktree {
                "Create a linked checkout on a new local branch."
            } else {
                "Create a folder, initialize Git, and add it to Projects."
            })
            .size(13)
            .color(p.muted()),
        ]
        .spacing(8);
        if let create::Kind::Worktree { source, .. } = &draft.kind {
            form = form
                .push(field(
                    "Branch name",
                    text_input("feature/my-change", &draft.branch)
                        .id(CREATE_BRANCH_INPUT)
                        .on_input_maybe(edit(create::Message::Branch))
                        .into(),
                ))
                .push(field(
                    "Base ref",
                    text_input("HEAD", &draft.base)
                        .on_input_maybe(edit(create::Message::Base))
                        .into(),
                ))
                .push(
                    text(format!("Base is resolved from {}", source.display()))
                        .size(11)
                        .color(p.muted()),
                );
        }
        if worktree {
            // Two buttons acting as a radio group: the chosen one is highlighted.
            let choice = |label: &'static str, managed: bool| {
                button(text(label).size(12))
                    .padding([4, 9])
                    .style(style::button(
                        p,
                        if draft.managed == managed {
                            Button::Selected
                        } else {
                            Button::Secondary
                        },
                    ))
                    .on_press_maybe(
                        ready.then_some(Message::Creation(create::Message::Managed(managed))),
                    )
            };
            form = form.push(field(
                "Open with",
                row![choice("Terminal", false), choice("Managed Agent", true)]
                    .spacing(6)
                    .into(),
            ));
            form = form.push(
                text(if draft.managed {
                    "A managed worktree shows a prompt for Claude, Codex or Pi instead of a terminal."
                } else {
                    "A terminal attached to this worktree's tmux session."
                })
                .size(11)
                .color(p.muted()),
            );
        }
        let placeholder = draft.suggested_name();
        form = form.push(field(
            "Folder name",
            text_input(
                if placeholder.is_empty() {
                    "my-project"
                } else {
                    &placeholder
                },
                &draft.name,
            )
            .id(CREATE_NAME_INPUT)
            .on_input_maybe(edit(create::Message::Name))
            .into(),
        ));
        form = form.push(
            row![
                text(if draft.parent.as_os_str().is_empty() {
                    "Choose a location".into()
                } else {
                    draft.parent.display().to_string()
                })
                .width(Fill)
                .size(13),
                button(text("Choose Location…").size(12))
                    .style(style::button(p, Button::Secondary))
                    .on_press_maybe(ready.then_some(Message::Pick(PickMode::CreateParent))),
            ]
            .spacing(6)
            .align_y(Center),
        );
        let destination = draft.destination();
        if let Ok(path) = &destination {
            form = form.push(
                text(format!("Create at {}", path.display()))
                    .size(12)
                    .color(p.muted()),
            );
        }
        form = form.push(
            row![
                button(
                    text(if draft.submitting {
                        "Creating…"
                    } else if worktree {
                        "Create Worktree"
                    } else {
                        "Create Project"
                    })
                    .size(13)
                )
                .style(style::button(p, Button::Primary))
                .on_press_maybe(
                    (ready && destination.is_ok())
                        .then_some(Message::Creation(create::Message::Confirm))
                ),
                button(text("Cancel").size(13))
                    .style(style::button(p, Button::Secondary))
                    .on_press_maybe(ready.then_some(Message::Creation(create::Message::Cancel))),
            ]
            .spacing(6),
        );
        container(scrollable(
            container(form).padding(14).max_width(620).width(Fill),
        ))
        .width(Fill)
        .height(Fill)
        .style(style::panel_in(p, Panel::Glass, corners))
        .into()
    }
}
