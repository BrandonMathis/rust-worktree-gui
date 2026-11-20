use crate::{
    config::Store,
    git::{self, Checkout, Worktree},
    terminal,
};
use iced::widget::{button, column, container, pick_list, row, scrollable, text, tooltip};
use iced::{Element, Fill, Task};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectId(usize);

#[derive(Clone, Copy, Debug)]
pub enum PickMode {
    Add,
    Replace(ProjectId),
}

pub struct App {
    store: Option<Store>,
    projects: Vec<Project>,
    selection: Option<Selection>,
    next_project_id: usize,
    next_refresh_id: usize,
    picking: bool,
    session_draft: Option<Selection>,
    launching: bool,
    notice: Option<String>,
}

struct Project {
    id: ProjectId,
    checkout: Checkout,
    expanded: bool,
    refresh_id: Option<usize>,
    inventory: Option<Result<Vec<Worktree>, String>>,
    sessions: Vec<WorktreeChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeChoice {
    path: PathBuf,
    branch: String,
}

impl From<&Worktree> for WorktreeChoice {
    fn from(tree: &Worktree) -> Self {
        Self {
            path: tree.path.clone(),
            branch: tree.label().to_owned(),
        }
    }
}

impl std::fmt::Display for WorktreeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} — {}", self.branch, self.path.display())
    }
}

struct Selection {
    project: ProjectId,
    worktree: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Pick(PickMode),
    FolderPicked(PickMode, Option<PathBuf>),
    Discovered(PickMode, Result<Checkout, String>),
    Refresh,
    Refreshed(usize, Result<Vec<Worktree>, String>),
    ToggleProject(ProjectId),
    SelectProject(ProjectId),
    SelectWorktree(ProjectId, PathBuf),
    NewSession(ProjectId),
    ChooseSessionWorktree(WorktreeChoice),
    CancelSession,
    Launch(ProjectId, PathBuf),
    Launched(ProjectId, WorktreeChoice, Result<(), String>),
    Forget(ProjectId),
    DismissNotice,
}

impl Project {
    fn name(&self) -> String {
        let path = self.checkout.root.as_path();
        path.file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    fn worktrees(&self) -> &[Worktree] {
        match &self.inventory {
            Some(Ok(trees)) => trees,
            _ => &[],
        }
    }

    fn worktree(&self, path: &Path) -> Option<&Worktree> {
        self.worktrees().iter().find(|tree| tree.path == path)
    }

    fn usable_worktrees(&self) -> impl Iterator<Item = &Worktree> {
        self.worktrees()
            .iter()
            .filter(|tree| tree.unavailable_reason().is_none())
    }
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        Self::from_bookmarks(Store::open())
    }

    fn from_bookmarks(bookmarks: Result<(Store, Vec<Checkout>), String>) -> (Self, Task<Message>) {
        let (store, checkouts, notice) = match bookmarks {
            Ok((store, checkouts)) => (Some(store), checkouts, None),
            Err(error) => (None, Vec::new(), Some(error)),
        };
        let mut app = Self {
            store,
            projects: Vec::new(),
            selection: None,
            next_project_id: 0,
            next_refresh_id: 0,
            picking: false,
            session_draft: None,
            launching: false,
            notice,
        };
        for checkout in checkouts {
            app.insert_project(checkout);
        }
        if let Some(project) = app.projects.first() {
            app.select(project.id, None);
        }
        let refresh = app.refresh_all();
        (app, refresh)
    }

    fn insert_project(&mut self, checkout: Checkout) -> ProjectId {
        let id = ProjectId(self.next_project_id);
        self.next_project_id += 1;
        self.projects.push(Project {
            id,
            checkout,
            expanded: false,
            refresh_id: None,
            inventory: None,
            sessions: Vec::new(),
        });
        id
    }

    fn select(&mut self, id: ProjectId, worktree: Option<PathBuf>) {
        if self.projects.iter().any(|project| project.id == id) {
            self.selection = Some(Selection {
                project: id,
                worktree,
            });
            self.session_draft = None;
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pick(mode) if !self.picking && self.store.is_some() => {
                self.picking = true;
                self.notice = None;
                // Construct the dialog on the main thread so rfd can attach its native sheet.
                let picker = rfd::AsyncFileDialog::new()
                    .set_title("Choose a Git checkout")
                    .pick_folder();
                return Task::perform(picker, move |folder| {
                    Message::FolderPicked(mode, folder.map(|handle| handle.path().to_path_buf()))
                });
            }
            Message::Pick(_) => {}
            Message::FolderPicked(mode, Some(path)) => {
                return background(
                    move || git::discover(&path),
                    move |result| Message::Discovered(mode, result),
                );
            }
            Message::FolderPicked(_, None) => self.picking = false,
            Message::Discovered(mode, result) => {
                self.picking = false;
                match result.and_then(|checkout| self.bookmark(mode, checkout)) {
                    Ok(id) => {
                        if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                            project.expanded = true;
                        }
                        self.select(id, None);
                        return self.refresh(id);
                    }
                    Err(error) => self.notice = Some(error),
                }
            }
            Message::Refresh => return self.refresh_all(),
            Message::Refreshed(refresh_id, result) => {
                if let Some(project) = self
                    .projects
                    .iter_mut()
                    .find(|p| p.refresh_id == Some(refresh_id))
                {
                    project.refresh_id = None;
                    project.inventory = Some(result);
                    if let Some(selection) =
                        self.selection.as_mut().filter(|s| s.project == project.id)
                        && selection
                            .worktree
                            .as_deref()
                            .is_some_and(|path| project.worktree(path).is_none())
                    {
                        selection.worktree = None;
                    }
                }
            }
            Message::ToggleProject(id) => {
                if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                    project.expanded = !project.expanded;
                    if project.expanded || self.selection.as_ref().is_none_or(|s| s.project == id) {
                        self.select(id, None);
                    }
                }
            }
            Message::SelectProject(id) => self.select(id, None),
            Message::SelectWorktree(id, path) => self.select(id, Some(path)),
            Message::NewSession(id) => self.new_session(id),
            Message::ChooseSessionWorktree(choice) => {
                if let Some(draft) = &mut self.session_draft {
                    draft.worktree = Some(choice.path);
                }
            }
            Message::CancelSession => self.session_draft = None,
            Message::Launch(id, path) => return self.launch(id, path),
            Message::Launched(id, choice, result) => {
                self.launching = false;
                match result {
                    Ok(()) => {
                        if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                            project.expanded = true;
                            project.sessions.push(choice);
                        }
                        self.session_draft = None;
                    }
                    Err(error) => self.notice = Some(error),
                }
            }
            Message::Forget(id) => {
                if let Err(error) = self.forget(id) {
                    self.notice = Some(error);
                }
            }
            Message::DismissNotice => self.notice = None,
        }
        Task::none()
    }

    fn new_session(&mut self, id: ProjectId) {
        let Some(project) = self.projects.iter().find(|p| p.id == id) else {
            return;
        };
        if self.launching || project.usable_worktrees().next().is_none() {
            return;
        }
        let worktree = self
            .selection
            .as_ref()
            .filter(|s| s.project == id)
            .and_then(|s| s.worktree.as_deref())
            .and_then(|path| project.usable_worktrees().find(|tree| tree.path == path))
            .map(|tree| tree.path.clone());
        self.session_draft = Some(Selection {
            project: id,
            worktree,
        });
    }

    fn launch(&mut self, id: ProjectId, path: PathBuf) -> Task<Message> {
        if self.launching {
            return Task::none();
        }
        let choice = self
            .projects
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.usable_worktrees().find(|tree| tree.path == path))
            .map(WorktreeChoice::from);
        let Some(choice) = choice else {
            self.notice = Some(
                "That worktree is no longer available. Refresh or choose another worktree.".into(),
            );
            return Task::none();
        };
        self.launching = true;
        self.notice = None;
        Task::perform(terminal::launch(path), move |result| {
            Message::Launched(id, choice.clone(), result)
        })
    }

    fn bookmark(&mut self, mode: PickMode, checkout: Checkout) -> Result<ProjectId, String> {
        match mode {
            PickMode::Add => {
                if let Some(project) = self
                    .projects
                    .iter()
                    .find(|p| p.checkout.common_dir == checkout.common_dir)
                {
                    if project.checkout.root != checkout.root {
                        self.notice = Some("This repository is already bookmarked. To change its anchor, use Replace Anchor.".into());
                    }
                    return Ok(project.id);
                }
                self.save(
                    self.projects
                        .iter()
                        .map(|p| p.checkout.clone())
                        .chain([checkout.clone()]),
                )?;
                Ok(self.insert_project(checkout))
            }
            PickMode::Replace(id) => {
                let index = self
                    .projects
                    .iter()
                    .position(|p| p.id == id)
                    .ok_or("This project was forgotten")?;
                if self.projects[index].checkout.common_dir != checkout.common_dir {
                    return Err("That folder belongs to a different repository. Pick another checkout of this project, or use Add Project.".into());
                }
                self.save(self.projects.iter().map(|p| {
                    if p.id == id {
                        checkout.clone()
                    } else {
                        p.checkout.clone()
                    }
                }))?;
                let project = &mut self.projects[index];
                project.checkout = checkout;
                project.inventory = None;
                project.refresh_id = None; // Ignore inventory in flight for the old anchor.
                Ok(id)
            }
        }
    }

    fn forget(&mut self, id: ProjectId) -> Result<(), String> {
        self.save(
            self.projects
                .iter()
                .filter(|p| p.id != id)
                .map(|p| p.checkout.clone()),
        )?;
        self.projects.retain(|p| p.id != id);
        if self.session_draft.as_ref().is_some_and(|s| s.project == id) {
            self.session_draft = None;
        }
        if self.selection.as_ref().is_some_and(|s| s.project == id) {
            self.selection = self.projects.first().map(|p| Selection {
                project: p.id,
                worktree: None,
            });
        }
        Ok(())
    }

    fn save(&self, checkouts: impl IntoIterator<Item = Checkout>) -> Result<(), String> {
        self.store.as_ref().ok_or("Bookmarks are unavailable. Check the Application Support file and restart the app.")?.save(checkouts)
    }

    fn refresh_all(&mut self) -> Task<Message> {
        let ids: Vec<_> = self.projects.iter().map(|p| p.id).collect();
        Task::batch(ids.into_iter().map(|id| self.refresh(id)))
    }

    fn refresh(&mut self, id: ProjectId) -> Task<Message> {
        let Some(project) = self
            .projects
            .iter_mut()
            .find(|p| p.id == id && p.refresh_id.is_none())
        else {
            return Task::none();
        };
        let checkout = project.checkout.clone();
        let refresh_id = self.next_refresh_id;
        self.next_refresh_id += 1;
        project.refresh_id = Some(refresh_id);
        background(
            move || git::inventory(&checkout),
            move |result| Message::Refreshed(refresh_id, result),
        )
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mut page = column![
            row![self.sidebar(), self.details()]
                .spacing(16)
                .height(Fill)
        ]
        .spacing(12);
        if let Some(notice) = &self.notice {
            page = page.push(
                container(
                    row![
                        text(notice).width(Fill),
                        button("Dismiss")
                            .on_press(Message::DismissNotice)
                            .style(button::secondary),
                    ]
                    .spacing(12),
                )
                .padding(12)
                .width(Fill)
                .style(container::bordered_box),
            );
        }
        container(page).padding(16).width(Fill).height(Fill).into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let add_label = if self.picking {
            "Choosing…"
        } else {
            "Add Project"
        };
        let mut list = column![].spacing(8);
        if self.projects.is_empty() {
            let message = if self.store.is_some() {
                "No projects added yet."
            } else {
                "Bookmarks are unavailable."
            };
            list = list.push(text(message).size(14));
        }
        for project in &self.projects {
            let selected = self.selection.as_ref().filter(|s| s.project == project.id);
            let arrow = if project.expanded { "▾" } else { "▸" };
            let name = if matches!(project.inventory, Some(Err(_))) {
                format!("{arrow} {} (unavailable)", project.name())
            } else {
                format!("{arrow} {}", project.name())
            };
            list = list.push(
                button(text(name))
                    .width(Fill)
                    .on_press(Message::ToggleProject(project.id))
                    .style(if selected.is_some() {
                        button::secondary
                    } else {
                        button::subtle
                    }),
            );
            if project.expanded {
                let mut children = column![
                    button("Overview")
                        .style(button::subtle)
                        .on_press(Message::SelectProject(project.id)),
                    text("Worktrees").size(14),
                ]
                .spacing(6);
                if project.worktrees().is_empty() {
                    children = children.push(
                        text(if project.refresh_id.is_some() {
                            "Checking Git…"
                        } else {
                            "No worktrees to show"
                        })
                        .size(13),
                    );
                }
                for tree in project.worktrees() {
                    let label = if tree.unavailable_reason().is_some() {
                        format!("{} (unavailable)", tree.label())
                    } else {
                        tree.label().to_owned()
                    };
                    let active =
                        selected.and_then(|s| s.worktree.as_deref()) == Some(tree.path.as_path());
                    children = children.push(
                        button(text(label).size(14))
                            .width(Fill)
                            .style(if active {
                                button::secondary
                            } else {
                                button::subtle
                            })
                            .on_press(Message::SelectWorktree(project.id, tree.path.clone())),
                    );
                }
                children = children.push(text("Sessions · this run").size(14));
                if project.sessions.is_empty() {
                    children = children.push(text("No sessions launched").size(13));
                }
                for (index, session) in project.sessions.iter().enumerate() {
                    children = children.push(tooltip(
                        text(format!("#{} · {}", index + 1, session.branch)).size(14),
                        container(text(session.path.display().to_string()).size(13))
                            .padding(6)
                            .style(container::bordered_box),
                        tooltip::Position::Right,
                    ));
                }
                list = list.push(container(children).padding([2, 14]));
            }
        }
        let refreshing = self.projects.iter().any(|p| p.refresh_id.is_some());
        let actions = row![
            button(add_label).on_press_maybe(
                (!self.picking && self.store.is_some()).then_some(Message::Pick(PickMode::Add))
            ),
            button("Refresh").style(button::secondary).on_press_maybe(
                (!self.projects.is_empty() && !refreshing).then_some(Message::Refresh)
            ),
        ]
        .spacing(8);
        container(
            column![
                text("Projects").size(22),
                actions,
                scrollable(list.width(Fill)).height(Fill)
            ]
            .spacing(12),
        )
        .padding(14)
        .width(285)
        .height(Fill)
        .style(container::bordered_box)
        .into()
    }

    fn details(&self) -> Element<'_, Message> {
        let details = if let Some((project, selection)) = self.selection.as_ref().and_then(|s| {
            self.projects
                .iter()
                .find(|p| p.id == s.project)
                .map(|p| (p, s))
        }) {
            let mut details = column![
                text(project.name()).size(28),
                field(
                    "Saved checkout",
                    project.checkout.root.display().to_string()
                )
            ]
            .spacing(12);
            let actions = row![
                button("New Session").on_press_maybe(
                    (!self.launching && project.usable_worktrees().next().is_some())
                        .then_some(Message::NewSession(project.id))
                ),
                button("Replace Anchor")
                    .style(button::secondary)
                    .on_press_maybe(
                        (!self.picking).then_some(Message::Pick(PickMode::Replace(project.id)))
                    ),
                button("Forget Project")
                    .style(button::danger)
                    .on_press_maybe((!self.picking).then_some(Message::Forget(project.id))),
            ]
            .spacing(8);
            details = details.push(actions);
            if let Some(draft) = self
                .session_draft
                .as_ref()
                .filter(|draft| draft.project == project.id)
            {
                details = details.push(self.session_form(project, draft));
            } else if self.launching {
                details = details.push(text("Opening Alacritty…"));
            }
            if project.refresh_id.is_some() {
                details = details.push(text("Refreshing worktrees…"));
            }
            match &project.inventory {
                Some(Err(error)) => details.push(field("Checkout unavailable", error.clone()))
                    .push(text("Replace the anchor with a live checkout of the same repository, or forget this project.")),
                Some(Ok(trees)) => {
                    if let Some(tree) = selection.worktree.as_deref().and_then(|path| project.worktree(path)) {
                        details.push(worktree_details(project.id, tree, !self.launching && self.session_draft.is_none()))
                    } else {
                        details.push(text(format!("{} worktree(s) • Select one in the sidebar for details.", trees.len())))
                    }
                }
                None => details,
            }
        } else {
            column![text("Worktree Browser").size(30), text("Add a Git checkout to browse its worktrees. Only projects you choose are shown here.")].spacing(12)
        };
        container(scrollable(details.padding(20).width(Fill)))
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn session_form(&self, project: &Project, draft: &Selection) -> Element<'_, Message> {
        let choices: Vec<_> = project
            .usable_worktrees()
            .map(WorktreeChoice::from)
            .collect();
        let selected = choices
            .iter()
            .find(|choice| Some(&choice.path) == draft.worktree.as_ref())
            .cloned();
        let launch = selected
            .as_ref()
            .filter(|_| !self.launching)
            .map(|choice| Message::Launch(project.id, choice.path.clone()));
        let label = if self.launching {
            "Opening…"
        } else {
            "Open Shell"
        };
        container(column![
            text("New Session").size(22),
            text("Choose a worktree to open in Alacritty. Start your coding agent from that shell."),
            pick_list(choices, selected, Message::ChooseSessionWorktree)
                .placeholder("Choose a worktree")
                .width(Fill),
            row![
                button(label).on_press_maybe(launch),
                button("Cancel").style(button::secondary)
                    .on_press_maybe((!self.launching).then_some(Message::CancelSession)),
            ].spacing(8),
        ].spacing(10)).padding(14).width(Fill).style(container::bordered_box).into()
    }
}

fn background<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl Fn(Result<T, String>) -> Message + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(work)
                .await
                .map_err(|err| format!("Background task failed: {err}"))?
        },
        done,
    )
}

fn field<'a>(label: &'a str, value: String) -> Element<'a, Message> {
    column![text(label).size(14), text(value)]
        .spacing(3)
        .width(Fill)
        .into()
}

fn worktree_details(id: ProjectId, tree: &Worktree, can_launch: bool) -> Element<'_, Message> {
    let mut content = column![
        text(tree.label()).size(22),
        field("Worktree path", tree.path.display().to_string()),
        field(
            "Status",
            tree.unavailable_reason().unwrap_or("Available").into()
        ),
    ]
    .spacing(10);
    if let Some(head) = &tree.head {
        content = content.push(field("HEAD", head.clone()));
    }
    if let Some(reason) = &tree.locked {
        content = content.push(field(
            "Locked",
            if reason.is_empty() {
                "Yes".into()
            } else {
                reason.clone()
            },
        ));
    }
    if let Some(reason) = &tree.prunable {
        content = content.push(field(
            "Prunable",
            if reason.is_empty() {
                "Yes".into()
            } else {
                reason.clone()
            },
        ));
    }
    content = content.push(
        button("Open in Alacritty").on_press_maybe(
            (can_launch && tree.unavailable_reason().is_none())
                .then(|| Message::Launch(id, tree.path.clone())),
        ),
    );
    container(content)
        .padding(14)
        .width(Fill)
        .style(container::bordered_box)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_fixtures::{fixture, run_git};

    #[test]
    fn deduplicates_linked_checkouts_and_persists_anchor_replacement() {
        let (tmp, root) = fixture();
        run_git(
            &root,
            &["worktree", "add", "-qb", "topic", "../linked checkout"],
        );
        let main = git::discover(&root).unwrap();
        let linked = git::discover(&root.with_file_name("linked checkout")).unwrap();
        let config = tmp.path().join("config/projects.json");
        let (mut app, _) = App::from_bookmarks(Store::open_path(config.clone()));
        let id = app.bookmark(PickMode::Add, linked.clone()).unwrap();
        assert_eq!(app.bookmark(PickMode::Add, main.clone()).unwrap(), id);
        assert_eq!(app.projects.len(), 1);
        assert_eq!(Store::open_path(config.clone()).unwrap().1, [linked]);

        run_git(&root, &["worktree", "remove", "../linked checkout"]);
        assert!(git::inventory(&app.projects[0].checkout).is_err());
        assert_eq!(
            app.bookmark(PickMode::Replace(id), main.clone()).unwrap(),
            id
        );
        let (mut restarted, _) = App::from_bookmarks(Store::open_path(config.clone()));
        assert_eq!(restarted.projects[0].checkout, main);
        restarted.forget(restarted.projects[0].id).unwrap();
        assert!(Store::open_path(config).unwrap().1.is_empty());
        assert!(git::inventory(&main).is_ok());
    }

    #[test]
    fn sessions_preselect_the_worktree_and_record_only_successes_for_this_run() {
        let (tmp, root) = fixture();
        let config = tmp.path().join("config/projects.json");
        let (mut app, _) = App::from_bookmarks(Store::open_path(config.clone()));
        let checkout = git::discover(&root).unwrap();
        let id = app.bookmark(PickMode::Add, checkout.clone()).unwrap();
        let trees = git::inventory(&checkout).unwrap();
        let choice = WorktreeChoice::from(&trees[0]);
        app.projects[0].inventory = Some(Ok(trees));
        app.select(id, Some(root.clone()));
        let _ = app.update(Message::NewSession(id));
        assert_eq!(
            app.session_draft.as_ref().unwrap().worktree.as_ref(),
            Some(&root)
        );

        let _ = app.update(Message::Launch(id, root.clone()));
        assert!(app.launching);
        let _ = app.update(Message::Launched(
            id,
            choice.clone(),
            Err("missing binary".into()),
        ));
        assert!(app.projects[0].sessions.is_empty());
        assert!(app.session_draft.is_some());
        assert_eq!(app.notice.as_deref(), Some("missing binary"));

        for _ in 0..2 {
            let _ = app.update(Message::Launch(id, root.clone()));
            let _ = app.update(Message::Launched(id, choice.clone(), Ok(())));
        }
        let _ = app.update(Message::Refresh);
        let refresh_id = app.projects[0].refresh_id.unwrap();
        let _ = app.update(Message::Refreshed(refresh_id, git::inventory(&checkout)));
        assert_eq!(app.projects[0].sessions, [choice.clone(), choice]);
        let (restarted, _) = App::from_bookmarks(Store::open_path(config));
        assert!(restarted.projects[0].sessions.is_empty());
    }
}
