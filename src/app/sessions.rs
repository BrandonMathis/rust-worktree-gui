//! tmux session badges in the sidebar: which worktrees have a live session,
//! and which have output or a bell the user hasn't seen (for example, an agent
//! finished or is waiting for input).
//!
//! A background poll asks the app's tmux server about all sessions in one
//! `list-windows -a` call every few seconds. It only reads: it never kills a
//! session or changes an option. "Unseen" is tracked here, not in tmux, so the
//! user's tmux config (`monitor-activity` and so on) doesn't matter.

use super::{App, ConsoleState, Message, Project, ProjectId, background};
use crate::icons;
use crate::theme::Palette;
use crate::tmux::{self, SessionActivity};
use iced::widget::Svg;
use iced::{Subscription, Task};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// The latest tmux report for one worktree's session, plus how much of its
/// output the user has seen.
#[derive(Clone, Debug)]
pub(super) struct Session {
    activity: SessionActivity,
    /// Output up to this time (seconds since the Unix epoch, like tmux's
    /// `window_activity`) counts as seen.
    seen_until: u64,
}

/// The badge on the right of a sidebar worktree row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionBadge {
    /// The session is running and nothing new happened.
    Live,
    /// New output since the user last looked.
    Activity,
    /// A bell rang in a window nobody has looked at since.
    Bell,
}

impl Session {
    fn badge(&self) -> SessionBadge {
        if self.activity.bell {
            SessionBadge::Bell
        } else if self.activity.last_activity > self.seen_until {
            SessionBadge::Activity
        } else {
            SessionBadge::Live
        }
    }
}

impl SessionBadge {
    pub(super) fn view<'a>(self, size: u32, p: Palette) -> Svg<'a> {
        match self {
            Self::Live => icons::session_live(size, p.muted()),
            Self::Activity => icons::session_activity(size, p.cyan),
            Self::Bell => icons::bell(size, p.yellow),
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Live => "tmux session running",
            Self::Activity => "tmux session has new output",
            Self::Bell => "tmux session rang the bell",
        }
    }
}

impl Project {
    pub(super) fn session_badge(&self, path: &Path) -> Option<SessionBadge> {
        self.sessions.get(path).map(Session::badge)
    }
}

impl App {
    /// A timer that drives `poll_sessions`, active while there are projects.
    pub(super) fn session_subscription(&self) -> Subscription<Message> {
        if self.projects.is_empty() {
            Subscription::none()
        } else {
            iced::time::every(POLL_INTERVAL).map(|_| Message::SessionPollTick)
        }
    }

    /// Starts one background `tmux list-windows -a`, unless one is still running.
    pub(super) fn poll_sessions(&mut self) -> Task<Message> {
        if self.session_request.is_some() || self.projects.is_empty() {
            return Task::none();
        }
        self.next_session_request += 1;
        let request = self.next_session_request;
        self.session_request = Some(request);
        let worktrees: Vec<_> = self
            .projects
            .iter()
            .flat_map(|project| {
                project
                    .worktrees()
                    .iter()
                    .filter(|tree| tree.unavailable_reason().is_none())
                    .map(|tree| {
                        let key = (project.id, tree.path.clone());
                        (key, project.checkout.common_dir.clone(), tree.path.clone())
                    })
            })
            .collect();
        background(
            move || Ok(tmux::session_activity(worktrees)),
            move |result| Message::SessionsPolled(request, result.unwrap_or_default()),
        )
    }

    /// Replaces every project's sessions with the poll's report. Sessions that
    /// tmux no longer lists lose their badge.
    pub(super) fn sessions_polled(
        &mut self,
        request: u64,
        reports: Vec<((ProjectId, PathBuf), SessionActivity)>,
    ) {
        if self.session_request != Some(request) {
            return;
        }
        self.session_request = None;
        let viewing = self.attached_worktree();
        let mut sessions: BTreeMap<ProjectId, BTreeMap<PathBuf, Session>> = BTreeMap::new();
        for ((id, path), activity) in reports {
            let Some(project) = self.projects.iter().find(|p| p.id == id) else {
                continue;
            };
            // The first report is a baseline: output from before the app saw
            // the session isn't news. The session the user is watching is seen.
            let mut seen_until = project
                .sessions
                .get(&path)
                .map_or(activity.last_activity, |session| session.seen_until);
            if viewing.as_ref() == Some(&(id, path.clone())) {
                seen_until = seen_until.max(activity.last_activity);
            }
            let session = Session {
                activity,
                seen_until,
            };
            sessions.entry(id).or_default().insert(path, session);
        }
        for project in &mut self.projects {
            project.sessions = sessions.remove(&project.id).unwrap_or_default();
        }
    }

    /// The user is looking at, or just stopped looking at, this worktree's
    /// session: everything it printed so far counts as seen.
    pub(super) fn mark_session_seen(&mut self, id: ProjectId, path: &Path) {
        let Some(session) = self
            .projects
            .iter_mut()
            .find(|p| p.id == id)
            .and_then(|project| project.sessions.get_mut(path))
        else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        session.seen_until = session
            .seen_until
            .max(session.activity.last_activity)
            .max(now);
        // tmux clears the flag once the window is viewed; don't wait a poll.
        session.activity.bell = false;
    }

    /// Marks the session the terminal is showing as seen, before the view
    /// moves away from it (another selection, or Detach).
    pub(super) fn leave_session(&mut self) {
        if let Some((id, path)) = self.attached_worktree() {
            self.mark_session_seen(id, &path);
        }
    }

    /// The worktree whose session the terminal is showing, if any.
    fn attached_worktree(&self) -> Option<(ProjectId, PathBuf)> {
        match &self.console {
            ConsoleState::Attached(console) if console.is_open() => {
                let selection = self.selection.as_ref()?;
                Some((selection.project, selection.worktree.clone()?))
            }
            _ => None,
        }
    }
}
