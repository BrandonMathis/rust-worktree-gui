//! Closes tmux sessions nobody has used in a while, and remembers why (TRM-22 to TRM-25).
//!
//! The sweep runs off the UI thread at startup and then hourly. Every session it closes
//! is written to a small log, so that when the user opens that worktree again the app can
//! explain why they got a fresh shell instead of their old one.

use crate::config;
use crate::git::{self, Checkout};
use crate::tmux::{self, IdleSession};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{self, BufReader};
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;
/// Idle time before closing a session whose worktree isn't in any bookmarked project.
const ORPHAN_IDLE: u64 = DAY;
/// Idle time before closing a session for a worktree that still exists.
const LIVE_IDLE: u64 = 7 * DAY;
/// Log entries for sessions nobody reopened are dropped after this long.
const FORGET_AFTER: u64 = 90 * DAY;

/// The sweep and terminal connects both read-modify-write the log on background threads.
/// This lock makes those updates take turns. A `static Mutex<()>` guards a resource
/// outside Rust's ownership (the file) rather than data inside the mutex.
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Why and when a session was closed. Stored in the log, keyed by session name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closed {
    closed_at: u64,
    idle_seconds: u64,
    orphaned: bool,
}

type Log = BTreeMap<String, Closed>;

impl Closed {
    /// The notice shown when the user opens the worktree whose session was closed.
    pub fn notice(&self, now: u64) -> String {
        let idle = duration(self.idle_seconds);
        let reason = if self.orphaned {
            format!(
                "this worktree wasn't in a bookmarked project and the session had been idle for {idle}"
            )
        } else {
            format!("the session had been idle for {idle}")
        };
        format!(
            "Started a new terminal session. The previous one was closed automatically {} because {reason}.",
            ago(now.saturating_sub(self.closed_at))
        )
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Closes idle sessions and logs them. Returns how many were closed.
///
/// `checkouts` are all bookmarked projects; their worktrees decide which sessions are
/// orphans. `tmux` builds commands for the server to sweep (a closure so tests can point
/// it at a private socket).
pub fn sweep(
    tmux: impl Fn() -> Command,
    checkouts: &[Checkout],
    log_path: &Path,
    now: u64,
) -> Result<usize, String> {
    let known = known_sessions(checkouts);
    let _guard = LOG_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    // Read the log before closing anything: if it can't be read, a closed session could
    // never be explained, so nothing is closed.
    let mut log = load(log_path)?;
    let before = log.clone();
    log.retain(|_, closed| now.saturating_sub(closed.closed_at) < FORGET_AFTER);
    let sessions = tmux::idle_sessions(&tmux);
    let mut count = 0;
    for (name, closed) in select(&sessions, known.as_ref(), now) {
        if tmux::close_if_detached(&tmux, &name) {
            log.insert(name, closed);
            count += 1;
        }
    }
    if log != before {
        config::write_json(log_path, &log)?;
    }
    Ok(count)
}

/// Called when a terminal connects. Removes the worktree's log entries and returns the
/// notice to show, but only if a fresh session was started (`resumed` is false).
///
/// Log problems never block the terminal, so errors here are ignored.
pub fn acknowledge(log_path: &Path, names: &[String], resumed: bool, now: u64) -> Option<String> {
    let _guard = LOG_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let mut log = load(log_path).ok()?;
    let closed = names
        .iter()
        .filter_map(|name| log.remove(name))
        .max_by_key(|closed| closed.closed_at)?;
    let _ = config::write_json(log_path, &log);
    (!resumed).then(|| closed.notice(now))
}

/// Session names for every worktree in every bookmarked project.
///
/// Returns `None` if any project can't be read. Then the sweep can't prove a session is an
/// orphan, so every session gets the longer idle limit.
fn known_sessions(checkouts: &[Checkout]) -> Option<HashSet<String>> {
    let mut names = HashSet::new();
    for checkout in checkouts {
        for tree in git::inventory(checkout).ok()? {
            // `tmux::prepare` names sessions after the canonical path; match it.
            let path = tree.path.canonicalize().unwrap_or(tree.path);
            names.extend(tmux::session_names(&checkout.common_dir, &path));
        }
    }
    Some(names)
}

/// The cleanup policy: which sessions to close, and the log entry for each.
fn select(
    sessions: &[IdleSession],
    known: Option<&HashSet<String>>,
    now: u64,
) -> Vec<(String, Closed)> {
    sessions
        .iter()
        .filter(|session| !session.attached && tmux::is_app_session(&session.name))
        .filter_map(|session| {
            let idle_seconds = now.saturating_sub(session.last_input);
            let orphaned = known.is_some_and(|known| !known.contains(&session.name));
            let limit = if orphaned { ORPHAN_IDLE } else { LIVE_IDLE };
            (idle_seconds >= limit).then(|| {
                let closed = Closed {
                    closed_at: now,
                    idle_seconds,
                    orphaned,
                };
                (session.name.clone(), closed)
            })
        })
        .collect()
}

fn load(path: &Path) -> Result<Log, String> {
    let error = |err: &dyn std::fmt::Display| format!("Could not read {}: {err}", path.display());
    match File::open(path) {
        Ok(file) => serde_json::from_reader(BufReader::new(file)).map_err(|err| error(&err)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Log::new()),
        Err(err) => Err(error(&err)),
    }
}

fn ago(seconds: u64) -> String {
    if seconds < HOUR {
        "less than an hour ago".into()
    } else {
        format!("{} ago", duration(seconds))
    }
}

fn duration(seconds: u64) -> String {
    let (count, unit) = if seconds >= DAY {
        (seconds / DAY, "day")
    } else if seconds >= HOUR {
        (seconds / HOUR, "hour")
    } else {
        ((seconds / MINUTE).max(1), "minute")
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_fixtures::fixture;
    use crate::tmux::tests::PrivateServer;

    /// A fixed clock far enough from the epoch for any idle time used below.
    const NOW: u64 = 1_000 * DAY;

    fn session(name: &str, attached: bool, idle: u64) -> IdleSession {
        IdleSession {
            name: name.into(),
            attached,
            last_input: NOW - idle,
        }
    }

    fn selected(sessions: &[IdleSession], known: Option<&HashSet<String>>) -> Vec<String> {
        select(sessions, known, NOW)
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    #[test]
    fn orphans_close_after_a_day_and_live_sessions_after_a_week() {
        let live = "wt-aaaaaaaaaa";
        let known = HashSet::from([live.to_owned()]);
        let sessions = [
            session(live, false, 6 * DAY),
            session("wt-bbbbbbbbbb", false, DAY - 1),
            session("wt-cccccccccc", false, DAY),
        ];
        assert_eq!(selected(&sessions, Some(&known)), ["wt-cccccccccc"]);

        let sessions = [session(live, false, 7 * DAY)];
        let closed = select(&sessions, Some(&known), NOW);
        assert_eq!(closed.len(), 1);
        assert!(!closed[0].1.orphaned);
    }

    #[test]
    fn cleanup_never_touches_attached_or_foreign_sessions() {
        let sessions = [
            session("wt-aaaaaaaaaa", true, 30 * DAY),
            session("wt-unassigned-9", false, 30 * DAY),
            session("main", false, 30 * DAY),
            session("wt-ABCDEF0123", false, 30 * DAY),
            session(&format!("wt-{}", "d".repeat(32)), false, 30 * DAY),
        ];
        assert_eq!(
            selected(&sessions, Some(&HashSet::new())),
            [format!("wt-{}", "d".repeat(32))]
        );
    }

    #[test]
    fn unreadable_projects_give_every_session_the_longer_limit() {
        let sessions = [
            session("wt-aaaaaaaaaa", false, 2 * DAY),
            session("wt-bbbbbbbbbb", false, 7 * DAY),
        ];
        assert_eq!(selected(&sessions, None), ["wt-bbbbbbbbbb"]);
        let checkouts = [Checkout {
            root: "/nonexistent/repo".into(),
            common_dir: "/nonexistent/repo/.git".into(),
        }];
        assert_eq!(known_sessions(&checkouts), None);
    }

    #[test]
    fn sweep_closes_idle_sessions_logs_them_and_keeps_recent_and_foreign_ones() {
        let (tmp, root) = fixture();
        let Some(server) = PrivateServer::start() else {
            return; // tmux isn't installed.
        };
        let checkout = git::discover(&root).unwrap();
        let [live, _] = tmux::session_names(&checkout.common_dir, &root.canonicalize().unwrap());
        let orphan = "wt-0123456789";
        for name in [live.as_str(), orphan, "notes"] {
            server.new_session(name, &["/bin/sleep", "600"]);
        }
        let names = || -> Vec<String> {
            let mut names: Vec<_> = tmux::idle_sessions(|| server.command())
                .into_iter()
                .map(|session| session.name)
                .collect();
            names.sort();
            names
        };
        let log = tmp.path().join("config/closed-sessions.json");
        let now = unix_now();
        let checkouts = [checkout];

        let swept = sweep(|| server.command(), &checkouts, &log, now + 2 * DAY);
        assert_eq!(swept, Ok(1));
        assert_eq!(names(), ["notes".to_owned(), live.clone()]);

        assert_eq!(
            sweep(|| server.command(), &checkouts, &log, now + 8 * DAY),
            Ok(1)
        );
        assert_eq!(names(), ["notes"]);

        let stored = load(&log).unwrap();
        assert!(stored[orphan].orphaned);
        assert!(!stored[&live].orphaned);
        assert!(stored[&live].idle_seconds >= 8 * DAY - 5);
    }

    #[test]
    fn reopening_a_closed_session_explains_it_once() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("closed-sessions.json");
        let closed = Closed {
            closed_at: 10 * DAY,
            idle_seconds: 8 * DAY,
            orphaned: false,
        };
        let names = ["wt-aaaaaaaaaa".to_owned(), format!("wt-{}", "a".repeat(32))];
        let log = Log::from([(names[0].clone(), closed), ("wt-bbbbbbbbbb".into(), closed)]);
        config::write_json(&log_path, &log).unwrap();

        let notice = acknowledge(&log_path, &names, false, 12 * DAY).unwrap();
        assert_eq!(
            notice,
            "Started a new terminal session. The previous one was closed automatically 2 days ago because the session had been idle for 8 days."
        );
        assert_eq!(acknowledge(&log_path, &names, false, 12 * DAY), None);
        assert!(load(&log_path).unwrap().contains_key("wt-bbbbbbbbbb"));

        // Someone else recreated the session meanwhile: clear the entry without a notice.
        let other = ["wt-bbbbbbbbbb".to_owned()];
        assert_eq!(acknowledge(&log_path, &other, true, 12 * DAY), None);
        assert!(load(&log_path).unwrap().is_empty());
    }

    #[test]
    fn notices_say_when_and_why_a_session_was_closed() {
        let orphan = Closed {
            closed_at: 0,
            idle_seconds: DAY + 3 * HOUR,
            orphaned: true,
        };
        assert_eq!(
            orphan.notice(30 * MINUTE),
            "Started a new terminal session. The previous one was closed automatically less than an hour ago because this worktree wasn't in a bookmarked project and the session had been idle for 1 day."
        );
        assert_eq!(duration(2 * HOUR + 59 * MINUTE), "2 hours");
        assert_eq!(duration(5), "1 minute");
        assert_eq!(ago(HOUR), "1 hour ago");
    }
}
