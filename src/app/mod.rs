//! The application: its state, messages, `update` and `view`.
//!
//! iced apps follow the Elm architecture. Mapped to React: [`App`] is the
//! state, [`Message`] is an action, `App::update` is the reducer and returns a
//! `Task` (an effect), and `App::view` renders.
//!
//! This module holds the state types and helpers every part shares. Each
//! submodule adds an `impl App` block for one area:
//!
//! | Module | Area |
//! | --- | --- |
//! | `update` | `App::update`, which hands each message to a handler |
//! | `auto_refresh` | Refreshing on window focus and when Git metadata changes |
//! | `projects` | Bookmarks, refreshing worktrees from Git, display names |
//! | `pull_requests` | PR badge lookups (one `gh pr list` per project) |
//! | `sessions` | tmux session activity badges |
//! | `terminal_pane` | The embedded terminal's connection state |
//! | `creation` | The New Project / New Worktree form (a nested component) |
//! | `sidebar` | Sidebar state and view |
//! | `view` | The window layout, workspace and worktree header |
//! | `widgets` | Small shared view helpers |
//!
//! Rust privacy is per module: private fields of `App` are visible here and in
//! every submodule, while items defined *in* a submodule need `pub(super)` to
//! be seen here or in a sibling.

mod auto_refresh;
mod creation;
mod projects;
mod pull_requests;
mod sessions;
mod sidebar;
mod terminal_pane;
mod update;
mod view;
mod widgets;

#[cfg(test)]
mod tests;

use crate::{
    actions::DirectoryAction,
    config::{ProjectBookmark, Store},
    create::{self, ProjectChoice},
    git::{Checkout, Worktree},
    pull_request::PullRequest,
    settings,
    style::Corners,
    terminal_config::Preferences,
    theme::{self, Palette},
    tmux::SessionActivity,
    watch::Fingerprint,
};
use iced::{Subscription, Task};
use projects::RefreshKind;
use sessions::Session;
use sidebar::SidebarPreview;
pub use sidebar::SidebarZone;
use std::collections::BTreeMap;
use std::path::PathBuf;
use terminal_pane::ConsoleState;

// Brought into scope for the tests (`use super::*`), which predate the split.
#[cfg(test)]
use {
    crate::{git, pull_request},
    sidebar::WorktreeIcon,
    std::path::Path,
    widgets::menu_actions,
};

const WORKTREE_NAME_INPUT: &str = "worktree-name-input";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectId(usize);

#[derive(Clone, Copy, Debug)]
pub enum PickMode {
    Add,
    Replace(ProjectId),
    CreateParent,
}

pub struct App {
    store: Option<Store>,
    projects: Vec<Project>,
    selection: Option<Selection>,
    next_project_id: usize,
    next_refresh_id: usize,
    /// The Git metadata check in flight, if any (see `auto_refresh`).
    watch_request: Option<u64>,
    next_watch_id: u64,
    /// The tmux session poll in flight, if any (see `sessions`).
    session_request: Option<u64>,
    next_session_request: u64,
    next_console_id: u64,
    pr_request_id: u64,
    console: ConsoleState,
    terminal_preferences: Preferences,
    window_scale: f32,
    /// Rounding for panels in the window's bottom corners (macOS only).
    window_corners: Corners,
    quit_pending: bool,
    sidebar_visible: bool,
    sidebar_preview: SidebarPreview,
    picking: bool,
    creation: Option<create::Draft>,
    worktree_name_draft: Option<RenameDraft>,
    notice: Option<String>,
    /// The saved theme.
    palette: Palette,
    theme_store: Option<theme::Store>,
    /// The open Settings modal and its unsaved edits.
    settings: Option<settings::Draft>,
}

struct Project {
    id: ProjectId,
    checkout: Checkout,
    worktree_names: BTreeMap<PathBuf, String>,
    expanded: bool,
    refresh_id: Option<usize>,
    /// Why the refresh in flight runs; decides which PRs it looks up again.
    refresh_kind: RefreshKind,
    /// The last Git metadata fingerprint seen, to detect outside changes.
    fingerprint: Option<Fingerprint>,
    /// The last worktree list Git returned successfully. `None` until the
    /// first refresh succeeds (or after the anchor is replaced).
    inventory: Option<Vec<Worktree>>,
    /// Why the latest refresh failed. The last good `inventory` is kept, so a
    /// transient Git error doesn't make the selected worktree look deleted.
    refresh_error: Option<String>,
    pull_requests: crate::pull_request::Badges,
    /// tmux sessions of this project's worktrees, from the latest poll.
    sessions: BTreeMap<PathBuf, Session>,
    open_after_refresh: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    project: ProjectId,
    worktree: Option<PathBuf>,
}

#[derive(Clone)]
struct RenameDraft {
    target: Selection,
    input: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Pick(PickMode),
    FolderPicked(PickMode, Option<PathBuf>),
    Discovered(PickMode, Result<Checkout, String>),
    ProjectChoice(ProjectChoice),
    NewWorktree(ProjectId),
    /// An edit in the New Project / New Worktree form.
    Creation(create::Message),
    ProjectCreated(Result<Checkout, String>),
    WorktreeCreated(ProjectId, Result<PathBuf, String>),
    Refresh,
    /// The window gained focus: refresh automatically.
    WindowFocused,
    /// Time to check Git metadata for outside changes.
    WatchTick,
    /// Fingerprints from a `WatchTick` check, tagged with its request id.
    Watched(u64, Vec<(ProjectId, Fingerprint)>),
    /// Time to poll tmux for session activity.
    SessionPollTick,
    /// Session activity from a `SessionPollTick` poll, tagged with its request id.
    SessionsPolled(u64, Vec<((ProjectId, PathBuf), SessionActivity)>),
    ToggleSidebar,
    SidebarHover(SidebarZone, bool),
    SidebarPreviewExpired(u64),
    Refreshed(usize, Result<Vec<Worktree>, String>),
    ToggleProject(ProjectId),
    SelectWorktree(ProjectId, PathBuf),
    PullRequestsListed(ProjectId, u64, Option<Vec<PullRequest>>),
    OpenPullRequest,
    PullRequestOpened(Result<(), String>),
    Reconnect,
    Prepared(u64, Result<iced_term::settings::BackendSettings, String>),
    /// The running terminal's tmux client switched sessions (or failed to).
    Switched(u64, Result<(), String>),
    Detach,
    Terminal(iced_term::Event),
    WindowOpened(iced::window::Id),
    WindowScale(f32),
    QuitRequested(iced::window::Id),
    QuitAnswered(bool),
    Forget(ProjectId),
    MenuAction(Selection, MenuAction),
    EditWorktreeName(String),
    SaveWorktreeName,
    CancelWorktreeName,
    ActionFinished(Result<Option<String>, String>),
    DismissNotice,
    OpenSettings,
    Settings(settings::Message),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Directory(DirectoryAction),
    RenameWorktree,
    NewWorktree,
    ReplaceAnchor,
    ForgetProject,
}

impl std::fmt::Display for MenuAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Directory(action) => std::fmt::Display::fmt(action, f),
            Self::RenameWorktree => f.write_str("Rename Worktree"),
            Self::NewWorktree => f.write_str("New Worktree…"),
            Self::ReplaceAnchor => f.write_str("Replace Anchor…"),
            Self::ForgetProject => f.write_str("Forget Project"),
        }
    }
}

impl App {
    pub fn new(
        preferences: Result<Preferences, String>,
        window_radius: f32,
    ) -> (Self, Task<Message>) {
        let (mut app, task) = Self::from_bookmarks(Store::open());
        app.window_corners = Corners::window(window_radius);
        match preferences {
            Ok(preferences) => app.terminal_preferences = preferences,
            Err(error) if app.notice.is_none() => app.notice = Some(error),
            Err(_) => {}
        }
        let theme = theme::Store::open().and_then(|store| {
            let palette = store.load();
            // Keep the store even when the file is unreadable, so saving from
            // Settings can replace it.
            app.theme_store = Some(store);
            palette
        });
        match theme {
            Ok(palette) => app.palette = palette,
            Err(error) if app.notice.is_none() => app.notice = Some(error),
            Err(_) => {}
        }
        #[cfg(feature = "capture")]
        if let Some(target) = crate::capture::target() {
            app.open_after_load(target);
        }
        (app, task)
    }

    /// Screenshot builds only: open a worktree as soon as its project loads.
    /// This reuses the path a newly created worktree takes, so the refresh
    /// handler selects the worktree and connects its terminal.
    #[cfg(feature = "capture")]
    fn open_after_load(&mut self, target: crate::capture::Target) {
        if let Some(project) = self.projects.get_mut(target.project) {
            project.expanded = true;
            project.open_after_refresh = Some(target.worktree);
        }
    }

    fn from_bookmarks(
        bookmarks: Result<(Store, Vec<ProjectBookmark>), String>,
    ) -> (Self, Task<Message>) {
        let (store, projects, notice) = match bookmarks {
            Ok((store, checkouts)) => (Some(store), checkouts, None),
            Err(error) => (None, Vec::new(), Some(error)),
        };
        let mut app = Self {
            store,
            projects: Vec::new(),
            selection: None,
            next_project_id: 0,
            next_refresh_id: 0,
            watch_request: None,
            next_watch_id: 0,
            session_request: None,
            next_session_request: 0,
            next_console_id: 0,
            pr_request_id: 0,
            console: ConsoleState::default(),
            terminal_preferences: Preferences::default(),
            window_scale: 1.0,
            window_corners: Corners::NONE,
            quit_pending: false,
            sidebar_visible: true,
            sidebar_preview: SidebarPreview::default(),
            picking: false,
            creation: None,
            worktree_name_draft: None,
            notice,
            palette: Palette::default(),
            theme_store: None,
            settings: None,
        };
        for project in projects {
            app.insert_project(project);
        }
        if let Some(project) = app.projects.first_mut() {
            project.expanded = true;
            app.selection = Some(Selection {
                project: project.id,
                worktree: None,
            });
        }
        let refresh = app.refresh_all();
        (app, refresh)
    }

    fn select(&mut self, id: ProjectId, worktree: Option<PathBuf>) {
        if self.projects.iter().any(|project| project.id == id) {
            let selection = Selection {
                project: id,
                worktree,
            };
            if self.selection.as_ref() != Some(&selection) {
                self.leave_session();
                self.selection = Some(selection);
                self.on_selection_changed();
            }
        }
    }

    /// Resets what belongs to the previous selection: the terminal detaches
    /// (its tmux session keeps running) unless it can switch to the new
    /// worktree, and an open rename draft is dropped.
    /// The header's PR badge needs no reset, because it is read from the
    /// selected worktree each time (`selected_pull_request`).
    fn on_selection_changed(&mut self) {
        self.keep_or_detach_terminal();
        self.worktree_name_draft = None;
    }

    fn selected(&self) -> Option<(&Project, &Selection)> {
        let selection = self.selection.as_ref()?;
        let project = self.projects.iter().find(|p| p.id == selection.project)?;
        Some((project, selection))
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let terminal = match self.console.console() {
            Some(console) => console.subscription().map(Message::Terminal),
            None => Subscription::none(),
        };
        let settings = if self.settings.is_some() {
            iced::event::listen_with(settings_keys)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            terminal,
            settings,
            self.watch_subscription(),
            self.session_subscription(),
            iced::event::listen_with(window_events),
        ])
    }

    /// The palette to draw with: the unsaved draft while Settings is open
    /// (live preview), otherwise the saved theme.
    pub fn palette(&self) -> Palette {
        self.settings
            .as_ref()
            .map_or(self.palette, settings::Draft::palette)
    }

    pub fn theme(&self) -> iced::Theme {
        self.palette().iced_theme()
    }
}

/// Turns the window events the app cares about into messages. `listen_with`
/// needs a plain `fn` (no captured state), so this lives outside `App`.
fn window_events(
    event: iced::Event,
    _: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::Opened { .. }) => Some(Message::WindowOpened(id)),
        iced::Event::Window(iced::window::Event::CloseRequested) => {
            Some(Message::QuitRequested(id))
        }
        iced::Event::Window(iced::window::Event::Rescaled(scale)) => {
            Some(Message::WindowScale(scale))
        }
        iced::Event::Window(iced::window::Event::Focused) => Some(Message::WindowFocused),
        _ => None,
    }
}

/// Escape cancels the Settings modal (subscribed only while it is open).
fn settings_keys(
    event: iced::Event,
    _: iced::event::Status,
    _: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(Message::Settings(settings::Message::Cancel)),
        _ => None,
    }
}

/// Runs blocking `work` (git, tmux, `open`) on Tokio's blocking thread pool,
/// so the UI thread never waits, and turns its result into a message.
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
