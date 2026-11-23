//! Automatic refresh. Git stays the source of truth: these only decide *when*
//! to run `git worktree list` again, never cache its answer.
//!
//! - When the window gains focus, every project refreshes.
//! - Every couple of seconds, a background check fingerprints each project's
//!   Git metadata (`watch::fingerprint`). A project whose fingerprint changed
//!   (a worktree added in the embedded terminal, a branch switched, a
//!   directory removed) refreshes.
//!
//! Both are `RefreshKind::Automatic`: PR badges for unchanged worktrees stay,
//! so `gh` runs only for worktrees that are new or changed branches.

use super::{App, Message, ProjectId, RefreshKind, background};
use crate::watch::{self, Fingerprint};
use iced::{Subscription, Task};
use std::time::Duration;

/// How often Git metadata is checked. Each check is a handful of `stat` calls
/// per project, run off the UI thread.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);

impl App {
    pub(super) fn refresh_all_automatically(&mut self) -> Task<Message> {
        let ids: Vec<_> = self.projects.iter().map(|p| p.id).collect();
        Task::batch(
            ids.into_iter()
                .map(|id| self.start_refresh(id, RefreshKind::Automatic)),
        )
    }

    /// A timer that drives `watch_tick`, active while there are projects.
    pub(super) fn watch_subscription(&self) -> Subscription<Message> {
        if self.projects.is_empty() {
            Subscription::none()
        } else {
            iced::time::every(WATCH_INTERVAL).map(|_| Message::WatchTick)
        }
    }

    /// Starts one background fingerprint check for all projects, unless one is
    /// still running.
    pub(super) fn watch_tick(&mut self) -> Task<Message> {
        if self.watch_request.is_some() || self.projects.is_empty() {
            return Task::none();
        }
        self.next_watch_id += 1;
        let request = self.next_watch_id;
        self.watch_request = Some(request);
        let repositories: Vec<_> = self
            .projects
            .iter()
            .map(|p| (p.id, p.checkout.common_dir.clone()))
            .collect();
        background(
            move || {
                Ok(repositories
                    .into_iter()
                    .map(|(id, common_dir)| (id, watch::fingerprint(&common_dir)))
                    .collect())
            },
            // A failed check (the worker thread panicked) is just skipped.
            move |result| Message::Watched(request, result.unwrap_or_default()),
        )
    }

    /// Records each project's fingerprint and refreshes those that changed.
    /// The first fingerprint of a project is only a baseline.
    pub(super) fn watched(
        &mut self,
        request: u64,
        fingerprints: Vec<(ProjectId, Fingerprint)>,
    ) -> Task<Message> {
        if self.watch_request != Some(request) {
            return Task::none();
        }
        self.watch_request = None;
        let mut changed = Vec::new();
        for (id, fingerprint) in fingerprints {
            // A project forgotten while the check ran is skipped.
            if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                let previous = project.fingerprint.replace(fingerprint);
                if previous.is_some_and(|previous| previous != fingerprint) {
                    changed.push(id);
                }
            }
        }
        Task::batch(
            changed
                .into_iter()
                .map(|id| self.start_refresh(id, RefreshKind::Automatic)),
        )
    }
}
