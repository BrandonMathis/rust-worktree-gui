//! Schedules the idle tmux session cleanup (TRM-22 to TRM-25). The policy and
//! the tmux calls live in `crate::session_cleanup`; this file only decides
//! when a sweep runs and makes sure two never overlap.

use super::{App, Message, background};
use crate::config::Store;
use crate::session_cleanup;
use crate::tmux;
use iced::{Subscription, Task};
use std::time::Duration;

/// How often idle sessions are cleaned up while the app is open. The first
/// sweep runs at startup (`App::new`).
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

impl App {
    pub(super) fn sweep_subscription(&self) -> Subscription<Message> {
        iced::time::every(SWEEP_INTERVAL).map(|_| Message::SweepSessions)
    }

    /// Starts a background cleanup of idle tmux sessions, unless one is still
    /// running.
    pub(super) fn sweep_sessions(&mut self) -> Task<Message> {
        // Without the bookmark store the app knows no projects, so every
        // session would look orphaned. Skip cleanup entirely (TRM-24).
        let Some(log) = self.store.as_ref().map(Store::closed_sessions_path) else {
            return Task::none();
        };
        if self.sweeping_sessions {
            return Task::none();
        }
        self.sweeping_sessions = true;
        let checkouts: Vec<_> = self.projects.iter().map(|p| p.checkout.clone()).collect();
        background(
            move || {
                // No tmux means no sessions to clean up; connecting reports the problem.
                let Ok(tmux) = tmux::app_server() else {
                    return Ok(0);
                };
                session_cleanup::sweep(tmux, &checkouts, &log, session_cleanup::unix_now())
            },
            Message::SessionsSwept,
        )
    }

    pub(super) fn sessions_swept(&mut self, result: Result<usize, String>) {
        self.sweeping_sessions = false;
        if let Err(error) = result {
            self.notice = Some(error);
        }
    }
}
