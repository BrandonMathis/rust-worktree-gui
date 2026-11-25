//! Startup hooks for the PR screenshot script (`script/capture/capture.rb`).
//!
//! This module is compiled only with `cargo build --features capture`, so the
//! normal app never reads these variables. They put the app into a known state
//! without synthesizing clicks, which would break whenever the layout changes:
//!
//! - `WORKTREE_BROWSER_CAPTURE_PROJECT` (a bookmark index) and
//!   `WORKTREE_BROWSER_CAPTURE_WORKTREE` (a canonical worktree path): once that
//!   project's inventory loads, select the worktree and connect its terminal,
//!   exactly as a click on its sidebar row would.
//! - `WORKTREE_BROWSER_CAPTURE_SIZE`: the initial window size, as `WIDTHxHEIGHT`.
//! - `WORKTREE_BROWSER_CAPTURE_PROMPT`: a prompt to send in the opened worktree,
//!   if it is a managed one, as soon as its harness's models have loaded. The
//!   screenshot then shows a conversation instead of the empty composer.
//! - `WORKTREE_BROWSER_CAPTURE_OPEN`: a view to show at startup. Only
//!   `settings` (the Settings window), `unavailable` (the first project's
//!   unavailable worktrees) and `earlier-chats` (a managed worktree's New Chat
//!   form, with the `PROMPT` conversation set aside under Earlier chats) exist so far; add a variant here when a
//!   screenshot state needs another view.

use std::env;
use std::path::PathBuf;

/// The worktree to open once the bookmarks load.
pub struct Target {
    pub project: usize,
    pub worktree: PathBuf,
}

pub fn target() -> Option<Target> {
    let project = env::var("WORKTREE_BROWSER_CAPTURE_PROJECT")
        .ok()?
        .parse()
        .ok()?;
    // `var_os` keeps non-UTF-8 paths intact.
    let worktree = env::var_os("WORKTREE_BROWSER_CAPTURE_WORKTREE")?.into();
    Some(Target { project, worktree })
}

pub fn window_size() -> Option<iced::Size> {
    let value = env::var("WORKTREE_BROWSER_CAPTURE_SIZE").ok()?;
    let (width, height) = value.split_once('x')?;
    Some(iced::Size::new(width.parse().ok()?, height.parse().ok()?))
}

/// The prompt to send in the opened managed worktree. See the module docs.
pub fn prompt() -> Option<String> {
    env::var("WORKTREE_BROWSER_CAPTURE_PROMPT")
        .ok()
        .filter(|prompt| !prompt.trim().is_empty())
}

/// A view to show at startup instead of the plain workspace.
pub enum Open {
    Settings,
    /// The first project's unavailable worktrees modal.
    Unavailable,
    /// After sending the scripted prompt, start a new chat, so the form lists
    /// that conversation under Earlier chats.
    EarlierChats,
}

pub fn open() -> Option<Open> {
    match env::var("WORKTREE_BROWSER_CAPTURE_OPEN").ok()?.as_str() {
        "settings" => Some(Open::Settings),
        "unavailable" => Some(Open::Unavailable),
        "earlier-chats" => Some(Open::EarlierChats),
        // The capture script saves stderr next to each PNG, so a typo in a
        // state shows up there instead of as a silently wrong screenshot.
        other => {
            eprintln!("WORKTREE_BROWSER_CAPTURE_OPEN: unknown view {other:?}");
            None
        }
    }
}
