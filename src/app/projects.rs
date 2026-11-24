//! Project bookmarks, refreshing worktrees from Git, and display names.

use super::pull_requests::Lookup;
use super::{App, ConsoleState, Message, PickMode, Project, ProjectId, Selection, background};
use crate::config::ProjectBookmark;
use crate::git::{self, Checkout, Worktree};
use crate::pull_request::PullRequest;
use iced::Task;
use std::collections::BTreeMap;
use std::path::Path;

/// Why a refresh runs. A manual Refresh looks up every PR again; an
/// automatic one (window focus, Git metadata change) keeps the PR answers of
/// worktrees whose branch didn't change, so it doesn't run `gh` for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RefreshKind {
    #[default]
    Manual,
    Automatic,
}

impl Project {
    pub(super) fn bookmark(&self) -> ProjectBookmark {
        ProjectBookmark {
            checkout: self.checkout.clone(),
            worktree_names: self.worktree_names.clone(),
            managed: self.managed.clone(),
        }
    }

    pub(super) fn worktree_name<'a>(&'a self, tree: &'a Worktree) -> &'a str {
        self.worktree_names
            .get(&tree.path)
            .map_or_else(|| tree.label(), String::as_str)
    }

    pub(super) fn name(&self) -> std::borrow::Cow<'_, str> {
        self.checkout.repository_name().to_string_lossy()
    }

    pub(super) fn worktrees(&self) -> &[Worktree] {
        self.inventory.as_deref().unwrap_or_default()
    }

    pub(super) fn worktree(&self, path: &Path) -> Option<&Worktree> {
        self.worktrees().iter().find(|tree| tree.path == path)
    }

    pub(super) fn pull_request(&self, path: &Path) -> Option<&PullRequest> {
        self.pull_requests.get(path)
    }
}

impl App {
    pub(super) fn insert_project(&mut self, bookmark: ProjectBookmark) -> ProjectId {
        let id = ProjectId(self.next_project_id);
        self.next_project_id += 1;
        self.projects.push(Project {
            id,
            checkout: bookmark.checkout,
            worktree_names: bookmark.worktree_names,
            managed: bookmark.managed,
            expanded: false,
            refresh_id: None,
            refresh_kind: RefreshKind::default(),
            fingerprint: None,
            inventory: None,
            refresh_error: None,
            pull_requests: Default::default(),
            sessions: BTreeMap::new(),
            open_after_refresh: None,
        });
        id
    }

    pub(super) fn toggle_project(&mut self, id: ProjectId) {
        let Some(project) = self.projects.iter_mut().find(|p| p.id == id) else {
            return;
        };
        project.expanded = !project.expanded;
        if !project.expanded
            && self.worktree_name_draft.as_ref().is_some_and(|draft| {
                draft.target.project == id && self.selection.as_ref() != Some(&draft.target)
            })
        {
            self.worktree_name_draft = None;
        }
        // Browsing the sidebar must not change what the workspace shows:
        // only select the project when nothing is selected at all.
        if project.expanded && self.selection.is_none() {
            self.select(id, None);
        }
    }

    pub(super) fn finish_bookmark(
        &mut self,
        mode: PickMode,
        result: Result<Checkout, String>,
    ) -> Task<Message> {
        match result.and_then(|checkout| self.bookmark(mode, checkout)) {
            Ok(id) => {
                if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                    project.expanded = true;
                }
                self.select(id, None);
                self.refresh(id)
            }
            Err(error) => {
                self.notice = Some(error);
                Task::none()
            }
        }
    }

    pub(super) fn resolve_action_target(
        &self,
        target: &Selection,
    ) -> Option<(&Project, Option<&Worktree>)> {
        let project = self.projects.iter().find(|p| p.id == target.project)?;
        let tree = match target.worktree.as_deref() {
            Some(path) => Some(project.worktree(path)?),
            None => None,
        };
        Some((project, tree))
    }

    pub(super) fn rename_worktree(
        &mut self,
        target: &Selection,
        label: &str,
    ) -> Result<(), String> {
        let label = label.trim();
        if label.chars().count() > 80
            || label
                .chars()
                .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
        {
            return Err("Use a single-line worktree name of at most 80 characters.".into());
        }
        let Some((project, Some(tree))) = self.resolve_action_target(target) else {
            return Err("The worktree is no longer in this project.".into());
        };
        let path = &tree.path;
        let id = project.id;
        let mut updated = project.bookmark();
        if label.is_empty() {
            updated.worktree_names.remove(path);
        } else {
            updated
                .worktree_names
                .insert(path.to_path_buf(), label.to_owned());
        }
        self.save(self.projects.iter().map(|p| {
            if p.id == id {
                updated.clone()
            } else {
                p.bookmark()
            }
        }))?;
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
            project.worktree_names = updated.worktree_names;
        }
        self.notice = None;
        Ok(())
    }

    pub(super) fn bookmark(
        &mut self,
        mode: PickMode,
        checkout: Checkout,
    ) -> Result<ProjectId, String> {
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
                let bookmark = ProjectBookmark::from(checkout);
                self.save(
                    self.projects
                        .iter()
                        .map(Project::bookmark)
                        .chain([bookmark.clone()]),
                )?;
                Ok(self.insert_project(bookmark))
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
                    let mut saved = p.bookmark();
                    if p.id == id {
                        saved.checkout = checkout.clone();
                    }
                    saved
                }))?;
                let project = &mut self.projects[index];
                project.checkout = checkout;
                project.inventory = None;
                project.refresh_error = None;
                project.pull_requests.clear();
                project.refresh_id = None; // Ignore inventory in flight for the old anchor.
                Ok(id)
            }
            PickMode::CreateParent => Err("A parent folder is not a project bookmark.".into()),
        }
    }

    /// Moves a bookmark before another project, or to the end when `before` is `None`.
    pub(super) fn reorder_project(
        &mut self,
        source: ProjectId,
        before: Option<ProjectId>,
    ) -> Result<(), String> {
        let index = |id| self.projects.iter().position(|p| p.id == id);
        let Some(from) = index(source) else {
            return Ok(());
        };
        let slot = match before {
            Some(id) => match index(id) {
                Some(slot) => slot,
                None => return Ok(()),
            },
            None => self.projects.len(),
        };
        // Removing the source shifts insertion slots below it up by one.
        let to = slot - usize::from(slot > from);
        if from == to {
            return Ok(());
        }

        let mut bookmarks: Vec<_> = self.projects.iter().map(Project::bookmark).collect();
        let bookmark = bookmarks.remove(from);
        bookmarks.insert(to, bookmark);
        self.save(bookmarks)?;

        // Move the live project, not just its bookmark: all its worktree state
        // stays with it, and selection and async results refer to its stable ID.
        let project = self.projects.remove(from);
        self.projects.insert(to, project);
        Ok(())
    }

    pub(super) fn remove_bookmark(&mut self, id: ProjectId) -> Result<(), String> {
        self.save(
            self.projects
                .iter()
                .filter(|p| p.id != id)
                .map(Project::bookmark),
        )?;
        self.projects.retain(|p| p.id != id);
        if self
            .worktree_name_draft
            .as_ref()
            .is_some_and(|draft| draft.target.project == id)
        {
            self.worktree_name_draft = None;
        }
        if self.selection.as_ref().is_some_and(|s| s.project == id) {
            self.selection = self.projects.first().map(|p| Selection {
                project: p.id,
                worktree: None,
            });
            self.on_selection_changed();
        }
        Ok(())
    }

    pub(super) fn save(
        &self,
        projects: impl IntoIterator<Item = ProjectBookmark>,
    ) -> Result<(), String> {
        self.store.as_ref().ok_or("Bookmarks are unavailable. Check the Application Support file and restart the app.")?.save(projects)
    }

    pub(super) fn refresh_all(&mut self) -> Task<Message> {
        let ids: Vec<_> = self.projects.iter().map(|p| p.id).collect();
        Task::batch(ids.into_iter().map(|id| self.refresh(id)))
    }

    /// Re-reads one project's worktrees from Git and looks up all its PRs again.
    pub(super) fn refresh(&mut self, id: ProjectId) -> Task<Message> {
        self.start_refresh(id, RefreshKind::Manual)
    }

    /// Re-reads one project's worktrees from Git in the background. At most one
    /// refresh per project runs at a time; `refresh_id` identifies it, so a
    /// result for a refresh that was superseded (Replace Anchor) is ignored.
    pub(super) fn start_refresh(&mut self, id: ProjectId, kind: RefreshKind) -> Task<Message> {
        let Some(project) = self.projects.iter_mut().find(|p| p.id == id) else {
            return Task::none();
        };
        if project.refresh_id.is_some() {
            // A manual Refresh pressed during an automatic one still gets its
            // PR lookups when that refresh finishes.
            if kind == RefreshKind::Manual {
                project.refresh_kind = kind;
            }
            return Task::none();
        }
        project.refresh_kind = kind;
        let checkout = project.checkout.clone();
        let refresh_id = self.next_refresh_id;
        self.next_refresh_id += 1;
        project.refresh_id = Some(refresh_id);
        background(
            move || git::inventory(&checkout),
            move |result| Message::Refreshed(refresh_id, result),
        )
    }

    pub(super) fn refreshed(
        &mut self,
        refresh_id: usize,
        result: Result<Vec<Worktree>, String>,
    ) -> Task<Message> {
        let Some(project) = self
            .projects
            .iter_mut()
            .find(|p| p.refresh_id == Some(refresh_id))
        else {
            return Task::none();
        };
        project.refresh_id = None;
        let id = project.id;
        let created = project.open_after_refresh.take();
        let trees = match result {
            Ok(trees) => trees,
            // Keep the last good inventory, selection, terminal and PR
            // badges: a failed refresh says nothing about whether the
            // worktrees still exist. The error is shown next to the project.
            Err(error) => {
                project.refresh_error = Some(error);
                if let Some(path) = created {
                    self.notice = Some(format!(
                        "Worktree created at {}, but Git could not be read. Refresh to try again.",
                        path.display()
                    ));
                }
                return Task::none();
            }
        };
        project.inventory = Some(trees);
        project.refresh_error = None;
        // Keep badges until the next lookup answers (no flicker), but drop any
        // whose worktree disappeared or changed branch.
        let worktrees = project.inventory.as_deref().unwrap_or_default();
        project.pull_requests.retain_listed(worktrees);
        let scope = match project.refresh_kind {
            RefreshKind::Manual => Lookup::All,
            RefreshKind::Automatic => Lookup::New,
        };
        self.reconcile_selection(id);
        let lookup = self.lookup_pull_requests(id, scope);
        if let Some(path) = created {
            if self
                .projects
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| p.worktree(&path))
                .is_some()
            {
                self.select(id, Some(path));
                return Task::batch([lookup, self.connect()]);
            }
            self.notice = Some(format!(
                "Worktree created at {}, but Git did not list it. Refresh to try again.",
                path.display()
            ));
        }
        lookup
    }

    /// After a successful refresh of project `id`, lets go of a selection or
    /// rename draft that points at a worktree Git no longer lists.
    fn reconcile_selection(&mut self, id: ProjectId) {
        let Some(project) = self.projects.iter().find(|p| p.id == id) else {
            return;
        };
        if self.worktree_name_draft.as_ref().is_some_and(|draft| {
            draft.target.project == id
                && draft
                    .target
                    .worktree
                    .as_deref()
                    .is_some_and(|path| project.worktree(path).is_none())
        }) {
            self.worktree_name_draft = None;
        }
        let selected = self
            .selection
            .as_ref()
            .filter(|s| s.project == id)
            .and_then(|s| s.worktree.as_deref());
        // `Some(None)`: a worktree was selected and Git no longer lists it.
        match selected.map(|path| project.worktree(path)) {
            Some(None) => {
                if let Some(selection) = &mut self.selection {
                    selection.worktree = None;
                }
                self.on_selection_changed();
            }
            // Its directory is gone: keep the row selected, but detach.
            Some(Some(tree)) if tree.unavailable_reason().is_some() => {
                self.console = ConsoleState::Detached;
            }
            _ => {}
        }
    }
}
