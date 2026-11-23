//! PR badges: `gh pr view` lookups per worktree, and the header badge.

use super::{App, Message, ProjectId, background};
use crate::icons;
use crate::pull_request::{self, PullRequest};
use crate::style::{self, Button, Panel};
use crate::theme::Palette;
use iced::futures::{StreamExt, stream};
use iced::widget::{button, container, row, text, tooltip};
use iced::{Center, Element, Task};
use std::path::{Path, PathBuf};

/// The PR lookup state of one worktree. `request_id` is set while a lookup
/// is in flight, so a late answer to an older lookup is ignored.
#[derive(Default)]
pub(super) struct PullRequestCheck {
    pub(super) request_id: Option<u64>,
    pub(super) pr: Option<PullRequest>,
}

impl App {
    /// Starts a lookup for each worktree of project `id` (or only the one at
    /// `only`) that has a branch and no lookup in flight. At most four `gh`
    /// processes run at once.
    pub(super) fn lookup_pull_requests(
        &mut self,
        id: ProjectId,
        only: Option<&Path>,
    ) -> Task<Message> {
        let Some(project) = self.projects.iter_mut().find(|project| project.id == id) else {
            return Task::none();
        };
        let targets: Vec<_> = project
            .worktrees()
            .iter()
            .filter(|tree| {
                only.is_none_or(|path| tree.path == path)
                    && tree.unavailable_reason().is_none()
                    && !tree.detached
            })
            .filter_map(|tree| Some((tree.path.clone(), tree.branch.clone()?)))
            .collect();
        let mut requests = Vec::new();
        for (path, branch) in targets {
            let check = project.pull_requests.entry(path.clone()).or_default();
            if check.request_id.is_none() {
                self.pr_request_id += 1;
                check.request_id = Some(self.pr_request_id);
                requests.push((path, branch, self.pr_request_id));
            }
        }
        if requests.is_empty() {
            return Task::none();
        }
        // `Task::stream` turns an async stream into messages as items arrive;
        // `buffer_unordered(4)` runs up to four lookups concurrently.
        Task::stream(
            stream::iter(requests)
                .map(move |(path, branch, request)| async move {
                    let pr = pull_request::lookup(&path, &branch).await;
                    Message::PullRequestFound(id, path, request, pr)
                })
                .buffer_unordered(4),
        )
    }

    pub(super) fn pull_request_found(
        &mut self,
        id: ProjectId,
        path: PathBuf,
        request: u64,
        pr: Option<PullRequest>,
    ) {
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == id)
            && let Some(check) = project.pull_requests.get_mut(&path)
            && check.request_id == Some(request)
        {
            check.request_id = None;
            check.pr = pr;
        }
    }

    pub(super) fn selected_pull_request(&self) -> Option<&PullRequest> {
        let (project, selection) = self.selected()?;
        project.pull_request(selection.worktree.as_deref()?)
    }

    pub(super) fn open_pull_request(&self) -> Task<Message> {
        match self.selected_pull_request().cloned() {
            Some(pr) => background(move || pr.open(), Message::PullRequestOpened),
            None => Task::none(),
        }
    }
}

/// The header button that shows the selected worktree's PR and opens it.
pub(super) fn badge(pr: &PullRequest, p: Palette) -> Element<'_, Message> {
    tooltip(
        button(
            row![
                icons::pr_status(14, pr.status, p),
                text(format!("PR #{}", pr.number)).size(12),
            ]
            .spacing(5)
            .align_y(Center),
        )
        .padding([4, 7])
        .style(style::button(p, Button::Secondary))
        .on_press(Message::OpenPullRequest),
        container(text(pr.label()).size(11))
            .padding([3, 5])
            .style(style::panel(p, Panel::Console)),
        tooltip::Position::Bottom,
    )
    .into()
}
