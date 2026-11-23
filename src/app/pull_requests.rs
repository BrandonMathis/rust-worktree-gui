//! PR badges: one `gh pr list` per project, matched to worktrees by branch
//! (`pull_request::Badges`), and the header badge.

use super::{App, Message, ProjectId, background};
use crate::icons;
use crate::pull_request::{self, PullRequest};
use crate::style::{self, Button, Panel};
use crate::theme::Palette;
use iced::widget::{button, container, row, text, tooltip};
use iced::{Center, Element, Task};

/// When a project's PRs should be listed again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lookup {
    /// Always (manual Refresh, selecting another worktree).
    All,
    /// Only if a worktree is new or changed branch since the last answer
    /// (automatic refresh), so `gh` doesn't run when nothing changed.
    New,
}

impl App {
    /// Starts one `gh pr list` for project `id`, unless one is already running:
    /// its answer is matched against the worktrees listed when it arrives, so it
    /// is still good for a newer inventory.
    pub(super) fn lookup_pull_requests(&mut self, id: ProjectId, scope: Lookup) -> Task<Message> {
        let Some(project) = self.projects.iter_mut().find(|project| project.id == id) else {
            return Task::none();
        };
        let worktrees = project.inventory.as_deref().unwrap_or_default();
        let wanted = match scope {
            Lookup::All => worktrees
                .iter()
                .any(|tree| pull_request::branch_to_match(tree).is_some()),
            Lookup::New => project.pull_requests.needs_lookup(worktrees),
        };
        if !wanted || project.pull_requests.in_flight() {
            return Task::none();
        }
        self.pr_request_id += 1;
        let request = self.pr_request_id;
        project.pull_requests.start(request);
        let checkout = project.checkout.root.clone();
        background(
            move || Ok(pull_request::list(&checkout)),
            // A failed background task counts as "no answer" and hides the badges.
            move |prs| Message::PullRequestsListed(id, request, prs.ok().flatten()),
        )
    }

    pub(super) fn pull_requests_listed(
        &mut self,
        id: ProjectId,
        request: u64,
        prs: Option<Vec<PullRequest>>,
    ) {
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
            // `finish` ignores an answer to an older or cancelled lookup.
            let worktrees = project.inventory.as_deref().unwrap_or_default();
            project.pull_requests.finish(request, prs, worktrees);
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
