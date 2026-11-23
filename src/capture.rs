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
