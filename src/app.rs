use crate::{
    actions::DirectoryAction,
    config::{ProjectBookmark, Store},
    create::{self, ProjectChoice},
    git::{self, Checkout, Worktree},
    icons,
    pull_request::{self, PullRequest},
    quit, row_menu, style,
    terminal::Console,
    terminal_config::Preferences,
    tmux,
};
use iced::futures::{StreamExt, stream};
use iced::widget::{
    Space, Svg, button, column, container, hover, mouse_area, opaque, operation, pick_list, row,
    scrollable, stack, text, text_input, tooltip,
};
use iced::{Center, Element, Fill, Subscription, Task};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const WORKTREE_NAME_INPUT: &str = "worktree-name-input";
const CREATE_NAME_INPUT: &str = "create-name-input";
const CREATE_BRANCH_INPUT: &str = "create-branch-input";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    next_console_id: u64,
    pr_request_id: u64,
    console: ConsoleState,
    terminal_preferences: Preferences,
    window_scale: f32,
    quit_pending: bool,
    sidebar_visible: bool,
    sidebar_preview: SidebarPreview,
    picking: bool,
    creation: Option<create::Draft>,
    worktree_name_draft: Option<RenameDraft>,
    notice: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum SidebarZone {
    Toggle,
    Panel,
}

#[derive(Default)]
struct SidebarPreview {
    visible: bool,
    over_toggle: bool,
    over_panel: bool,
    epoch: u64,
}

impl SidebarPreview {
    fn hover(&mut self, zone: SidebarZone, entered: bool) -> Option<u64> {
        match zone {
            SidebarZone::Toggle => self.over_toggle = entered,
            SidebarZone::Panel => self.over_panel = entered,
        }
        self.epoch += 1;
        if self.over_toggle || self.over_panel {
            self.visible = true;
            None
        } else {
            self.visible.then_some(self.epoch)
        }
    }

    fn hide(&mut self) {
        self.epoch += 1;
        self.visible = false;
        self.over_toggle = false;
        self.over_panel = false;
    }
}

#[derive(Default)]
enum ConsoleState {
    #[default]
    Detached,
    Connecting(u64),
    Attached(Box<Console>),
    Failed(String),
}

struct Project {
    id: ProjectId,
    checkout: Checkout,
    worktree_names: BTreeMap<PathBuf, String>,
    expanded: bool,
    refresh_id: Option<usize>,
    inventory: Option<Result<Vec<Worktree>, String>>,
    pull_requests: BTreeMap<PathBuf, PullRequestCheck>,
    open_after_refresh: Option<PathBuf>,
}

#[derive(Default)]
struct PullRequestCheck {
    request_id: Option<u64>,
    pr: Option<PullRequest>,
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
    CreateName(String),
    CreateBranch(String),
    CreateBase(String),
    ConfirmCreation,
    CancelCreation,
    ProjectCreated(Result<Checkout, String>),
    WorktreeCreated(ProjectId, Result<PathBuf, String>),
    Refresh,
    ToggleSidebar,
    SidebarHover(SidebarZone, bool),
    SidebarPreviewExpired(u64),
    Refreshed(usize, Result<Vec<Worktree>, String>),
    ToggleProject(ProjectId),
    SelectWorktree(ProjectId, PathBuf),
    PullRequestFound(ProjectId, PathBuf, u64, Option<PullRequest>),
    OpenPullRequest,
    PullRequestOpened(Result<(), String>),
    Reconnect,
    Prepared(u64, Result<iced_term::settings::BackendSettings, String>),
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

impl Project {
    fn bookmark(&self) -> ProjectBookmark {
        ProjectBookmark {
            checkout: self.checkout.clone(),
            worktree_names: self.worktree_names.clone(),
        }
    }

    fn worktree_name<'a>(&'a self, tree: &'a Worktree) -> &'a str {
        self.worktree_names
            .get(&tree.path)
            .map_or_else(|| tree.label(), String::as_str)
    }

    fn name(&self) -> std::borrow::Cow<'_, str> {
        let path = self.checkout.root.as_path();
        path.file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
    }

    fn worktrees(&self) -> &[Worktree] {
        match &self.inventory {
            Some(Ok(trees)) => trees,
            _ => &[],
        }
    }

    fn worktree(&self, path: &Path) -> Option<&Worktree> {
        self.worktrees().iter().find(|tree| tree.path == path)
    }

    fn pull_request(&self, path: &Path) -> Option<&PullRequest> {
        self.pull_requests.get(path)?.pr.as_ref()
    }

    /// Picks the single sidebar icon for `tree`. Precedence: unavailable, then
    /// the main checkout, then the PR state, then a plain worktree.
    fn worktree_icon(&self, tree: &Worktree) -> WorktreeIcon {
        // `git worktree list` always lists the main worktree first.
        let is_main = self.worktrees().first().map(|main| &main.path) == Some(&tree.path);
        if tree.unavailable_reason().is_some() {
            WorktreeIcon::Unavailable
        } else if is_main {
            WorktreeIcon::Main
        } else if let Some(pr) = self.pull_request(&tree.path) {
            WorktreeIcon::PullRequest(pr.status)
        } else {
            WorktreeIcon::Worktree
        }
    }
}

/// The one status icon shown left of each worktree in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorktreeIcon {
    Unavailable,
    Main,
    Worktree,
    PullRequest(pull_request::Status),
}

impl WorktreeIcon {
    fn view<'a>(self, size: u32) -> Svg<'a> {
        match self {
            Self::Unavailable => icons::warning(size, style::RED),
            Self::Main => icons::home(size, style::ORANGE),
            Self::Worktree => icons::branch(size, style::CYAN),
            Self::PullRequest(status) => icons::pr_status(size, status),
        }
    }
}

impl App {
    pub fn new(preferences: Result<Preferences, String>) -> (Self, Task<Message>) {
        let (mut app, task) = Self::from_bookmarks(Store::open());
        match preferences {
            Ok(preferences) => app.terminal_preferences = preferences,
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
    /// This reuses the path a newly created worktree takes, so `Refreshed`
    /// selects the worktree and connects its terminal.
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
            next_console_id: 0,
            pr_request_id: 0,
            console: ConsoleState::default(),
            terminal_preferences: Preferences::default(),
            window_scale: 1.0,
            quit_pending: false,
            sidebar_visible: true,
            sidebar_preview: SidebarPreview::default(),
            picking: false,
            creation: None,
            worktree_name_draft: None,
            notice,
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

    fn insert_project(&mut self, bookmark: ProjectBookmark) -> ProjectId {
        let id = ProjectId(self.next_project_id);
        self.next_project_id += 1;
        self.projects.push(Project {
            id,
            checkout: bookmark.checkout,
            worktree_names: bookmark.worktree_names,
            expanded: false,
            refresh_id: None,
            inventory: None,
            pull_requests: BTreeMap::new(),
            open_after_refresh: None,
        });
        id
    }

    fn select(&mut self, id: ProjectId, worktree: Option<PathBuf>) {
        if self.projects.iter().any(|project| project.id == id) {
            let selection = Selection {
                project: id,
                worktree,
            };
            if self.selection.as_ref() != Some(&selection) {
                self.console = ConsoleState::Detached;
                self.selection = Some(selection);
                self.worktree_name_draft = None;
            }
        }
    }

    fn selected(&self) -> Option<(&Project, &Selection)> {
        let selection = self.selection.as_ref()?;
        let project = self.projects.iter().find(|p| p.id == selection.project)?;
        Some((project, selection))
    }

    fn lookup_pull_requests(&mut self, id: ProjectId, only: Option<&Path>) -> Task<Message> {
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
        Task::stream(
            stream::iter(requests)
                .map(move |(path, branch, request)| async move {
                    let pr = pull_request::lookup(&path, &branch).await;
                    Message::PullRequestFound(id, path, request, pr)
                })
                .buffer_unordered(4),
        )
    }

    fn selected_pull_request(&self) -> Option<&PullRequest> {
        let (project, selection) = self.selected()?;
        project.pull_request(selection.worktree.as_deref()?)
    }

    fn connect(&mut self) -> Task<Message> {
        if let ConsoleState::Attached(console) = &self.console
            && console.is_open()
        {
            return console.focus().map(Message::Terminal);
        }
        if matches!(self.console, ConsoleState::Connecting(_)) {
            return Task::none();
        }
        let Some((project, selection)) = self.selected() else {
            return Task::none();
        };
        let Some(tree) = selection
            .worktree
            .as_deref()
            .and_then(|path| project.worktree(path))
        else {
            return Task::none();
        };
        if tree.unavailable_reason().is_some() {
            return Task::none();
        }
        let worktree = tree.path.clone();
        let common_dir = project.checkout.common_dir.clone();
        self.next_console_id += 1;
        let id = self.next_console_id;
        self.console = ConsoleState::Connecting(id);
        background(
            move || tmux::prepare(&common_dir, &worktree),
            move |result| Message::Prepared(id, result),
        )
    }

    fn prepared(
        &mut self,
        id: u64,
        result: Result<iced_term::settings::BackendSettings, String>,
    ) -> Task<Message> {
        if !matches!(self.console, ConsoleState::Connecting(current) if current == id) {
            return Task::none();
        }
        match result.and_then(|backend| {
            Console::open(id, backend, &self.terminal_preferences, self.window_scale)
        }) {
            Ok(console) => {
                self.console = ConsoleState::Attached(Box::new(console));
                self.focus_terminal()
            }
            Err(error) => {
                self.console = ConsoleState::Failed(error);
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let terminal = if let ConsoleState::Attached(console) = &self.console {
            console.subscription().map(Message::Terminal)
        } else {
            Subscription::none()
        };
        Subscription::batch([terminal, iced::event::listen_with(window_scale)])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::QuitRequested(id) => {
                if !self.quit_pending && !self.picking {
                    self.quit_pending = true;
                    return quit::confirm(id).map(Message::QuitAnswered);
                }
            }
            Message::QuitAnswered(confirmed) => {
                if self.quit_pending {
                    self.quit_pending = false;
                    if confirmed {
                        return iced::exit();
                    }
                    return self.focus_terminal();
                }
            }
            Message::Pick(mode) if !self.picking && self.store.is_some() => {
                self.picking = true;
                self.notice = None;
                // Create the native sheet on the main thread.
                let picker = rfd::AsyncFileDialog::new()
                    .set_title(match mode {
                        PickMode::CreateParent => "Choose a parent folder",
                        _ => "Choose an existing Git checkout",
                    })
                    .pick_folder();
                return Task::perform(picker, move |folder| {
                    Message::FolderPicked(mode, folder.map(|handle| handle.path().to_path_buf()))
                });
            }
            Message::Pick(_) => {}
            Message::FolderPicked(PickMode::CreateParent, path) => {
                self.picking = false;
                if let Some(draft) = self.creation.as_mut()
                    && let Some(path) = path
                {
                    draft.parent = path;
                }
            }
            Message::FolderPicked(mode, Some(path)) => {
                return background(
                    move || git::discover(&path),
                    move |result| Message::Discovered(mode, result),
                );
            }
            Message::FolderPicked(_, None) => self.picking = false,
            Message::Discovered(mode, result) => {
                self.picking = false;
                return self.finish_bookmark(mode, result);
            }
            Message::ProjectChoice(ProjectChoice::Existing) if self.can_start_creation() => {
                return self.update(Message::Pick(PickMode::Add));
            }
            Message::ProjectChoice(ProjectChoice::New) if self.can_start_creation() => {
                let parent = self
                    .selected()
                    .and_then(|(project, _)| project.checkout.root.parent().map(Path::to_path_buf))
                    .or_else(|| {
                        directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
                    })
                    .unwrap_or_default();
                self.creation = Some(create::Draft::new(create::Kind::Project, parent));
                self.notice = None;
                return operation::focus(CREATE_NAME_INPUT);
            }
            Message::ProjectChoice(_) => {}
            Message::NewWorktree(id) if self.can_start_creation() => {
                if let Some(project) = self.projects.iter().find(|p| p.id == id)
                    && matches!(project.inventory, Some(Ok(_)))
                {
                    let source = self
                        .selection
                        .as_ref()
                        .filter(|selection| selection.project == id)
                        .and_then(|selection| project.worktree(selection.worktree.as_deref()?))
                        .filter(|tree| tree.unavailable_reason().is_none())
                        .map_or_else(|| project.checkout.root.clone(), |tree| tree.path.clone());
                    let parent = project
                        .checkout
                        .root
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                    self.creation = Some(create::Draft::new(
                        create::Kind::Worktree {
                            project: id,
                            anchor: project.checkout.clone(),
                            source,
                        },
                        parent,
                    ));
                    self.notice = None;
                    return operation::focus(CREATE_BRANCH_INPUT);
                }
                self.notice = Some("A live checkout is needed to create a worktree. Refresh or replace the project anchor.".into());
            }
            Message::NewWorktree(_) => {}
            Message::CreateName(name) => {
                if let Some(draft) = &mut self.creation {
                    draft.name = name;
                }
            }
            Message::CreateBranch(branch) => {
                if let Some(draft) = &mut self.creation {
                    draft.branch = branch;
                }
            }
            Message::CreateBase(base) => {
                if let Some(draft) = &mut self.creation {
                    draft.base = base;
                }
            }
            Message::ConfirmCreation => return self.confirm_creation(),
            Message::CancelCreation => {
                if !self.picking
                    && self
                        .creation
                        .as_ref()
                        .is_some_and(|draft| !draft.submitting)
                {
                    self.creation = None;
                    self.notice = None;
                    return self.focus_terminal();
                }
            }
            Message::ProjectCreated(result) => match result {
                Ok(checkout) => {
                    self.creation = None;
                    return self.finish_bookmark(PickMode::Add, Ok(checkout));
                }
                Err(error) => self.creation_error(error),
            },
            Message::WorktreeCreated(id, result) => match result {
                Ok(path) => {
                    self.creation = None;
                    if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                        project.expanded = true;
                        project.open_after_refresh = Some(path);
                        project.refresh_id = None; // Discard a snapshot taken before creation.
                        return self.refresh(id);
                    }
                    self.notice = Some(format!(
                        "Worktree created at {}, but its project is no longer bookmarked.",
                        path.display()
                    ));
                }
                Err(error) => self.creation_error(error),
            },
            Message::Refresh => return self.refresh_all(),
            Message::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                self.sidebar_preview.hide();
                if !self.sidebar_visible {
                    self.cancel_sidebar_rename();
                }
                return self.focus_terminal();
            }
            Message::SidebarHover(zone, entered) => {
                if !self.sidebar_visible
                    && let Some(epoch) = self.sidebar_preview.hover(zone, entered)
                {
                    // Give the pointer time to cross the gap from the toggle to the panel.
                    return Task::perform(
                        async { tokio::time::sleep(Duration::from_millis(220)).await },
                        move |_| Message::SidebarPreviewExpired(epoch),
                    );
                }
            }
            Message::SidebarPreviewExpired(epoch) => {
                if !self.sidebar_visible && self.sidebar_preview.epoch == epoch {
                    self.sidebar_preview.hide();
                    if self.cancel_sidebar_rename() {
                        return self.focus_terminal();
                    }
                }
            }
            Message::Refreshed(refresh_id, result) => {
                if let Some(project) = self
                    .projects
                    .iter_mut()
                    .find(|p| p.refresh_id == Some(refresh_id))
                {
                    project.refresh_id = None;
                    project.inventory = Some(result);
                    // Old results can't be applied to a worktree that changed branches.
                    project.pull_requests.clear();
                    let id = project.id;
                    let created = project.open_after_refresh.take();
                    if self.worktree_name_draft.as_ref().is_some_and(|draft| {
                        draft.target.project == project.id
                            && draft
                                .target
                                .worktree
                                .as_deref()
                                .is_some_and(|path| project.worktree(path).is_none())
                    }) {
                        self.worktree_name_draft = None;
                    }
                    if let Some(selection) =
                        self.selection.as_mut().filter(|s| s.project == project.id)
                        && let Some(path) = &selection.worktree
                    {
                        match project.worktree(path) {
                            None => {
                                selection.worktree = None;
                                self.console = ConsoleState::Detached;
                                self.worktree_name_draft = None;
                            }
                            Some(tree) if tree.unavailable_reason().is_some() => {
                                self.console = ConsoleState::Detached
                            }
                            _ => {}
                        }
                    }
                    let lookup = self.lookup_pull_requests(id, None);
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
                    return lookup;
                }
            }
            Message::ToggleProject(id) => {
                if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                    project.expanded = !project.expanded;
                    if !project.expanded
                        && self.worktree_name_draft.as_ref().is_some_and(|draft| {
                            draft.target.project == id
                                && self.selection.as_ref() != Some(&draft.target)
                        })
                    {
                        self.worktree_name_draft = None;
                    }
                    if project.expanded && self.selection.as_ref().is_none_or(|s| s.project != id) {
                        self.select(id, None);
                    }
                }
            }
            Message::SelectWorktree(id, path) => {
                self.select(id, Some(path.clone()));
                return Task::batch([self.connect(), self.lookup_pull_requests(id, Some(&path))]);
            }
            Message::PullRequestFound(id, path, request, pr) => {
                if let Some(project) = self.projects.iter_mut().find(|p| p.id == id)
                    && let Some(check) = project.pull_requests.get_mut(&path)
                    && check.request_id == Some(request)
                {
                    check.request_id = None;
                    check.pr = pr;
                }
            }
            Message::OpenPullRequest => {
                if let Some(pr) = self.selected_pull_request().cloned() {
                    return background(move || pr.open(), Message::PullRequestOpened);
                }
            }
            Message::PullRequestOpened(Err(error)) => self.notice = Some(error),
            Message::PullRequestOpened(Ok(())) => {}
            Message::Reconnect => return self.connect(),
            Message::Prepared(id, result) => return self.prepared(id, result),
            Message::Detach => self.console = ConsoleState::Detached,
            Message::WindowOpened(id) => {
                let scale = iced::window::scale_factor(id).map(Message::WindowScale);
                #[cfg(target_os = "macos")]
                return Task::batch([
                    scale,
                    iced::window::run(id, quit::install_shortcut).discard(),
                ]);
                #[cfg(not(target_os = "macos"))]
                return scale;
            }
            Message::WindowScale(scale) => {
                if self.window_scale != scale {
                    self.window_scale = scale;
                    if let ConsoleState::Attached(console) = &mut self.console {
                        console.rescale(&self.terminal_preferences, scale);
                    }
                }
            }
            Message::Terminal(event) => {
                if let ConsoleState::Attached(console) = &mut self.console {
                    console.update(event);
                }
            }
            Message::Forget(id) => {
                if let Err(error) = self.forget(id) {
                    self.notice = Some(error);
                }
            }
            Message::MenuAction(target, MenuAction::RenameWorktree) => {
                if let Some((project, Some(tree))) = self.resolve_action_target(&target) {
                    let input = project
                        .worktree_names
                        .get(&tree.path)
                        .cloned()
                        .unwrap_or_default();
                    self.worktree_name_draft = Some(RenameDraft { target, input });
                    return operation::focus(WORKTREE_NAME_INPUT);
                }
            }
            Message::MenuAction(
                target,
                action @ (MenuAction::NewWorktree
                | MenuAction::ReplaceAnchor
                | MenuAction::ForgetProject),
            ) => {
                if self.can_start_creation()
                    && target.worktree.is_none()
                    && self.resolve_action_target(&target).is_some()
                {
                    let message = match action {
                        MenuAction::NewWorktree => Message::NewWorktree(target.project),
                        MenuAction::ReplaceAnchor => {
                            Message::Pick(PickMode::Replace(target.project))
                        }
                        MenuAction::ForgetProject => Message::Forget(target.project),
                        _ => unreachable!(),
                    };
                    return self.update(message);
                }
            }
            Message::EditWorktreeName(name) => {
                if let Some(draft) = &mut self.worktree_name_draft {
                    draft.input = name;
                }
            }
            Message::SaveWorktreeName => {
                if let Some(draft) = self.worktree_name_draft.clone() {
                    match self.rename_worktree(&draft.target, &draft.input) {
                        Ok(()) => {
                            self.worktree_name_draft = None;
                            return self.focus_terminal();
                        }
                        Err(error) => self.notice = Some(error),
                    }
                }
            }
            Message::CancelWorktreeName => {
                self.worktree_name_draft = None;
                return self.focus_terminal();
            }
            Message::MenuAction(target, MenuAction::Directory(action)) => {
                if let Some((project, tree)) = self.resolve_action_target(&target) {
                    let (root, repo) = match tree {
                        Some(tree) => {
                            (tree.path.clone(), Some(project.checkout.common_dir.clone()))
                        }
                        None => (project.checkout.root.clone(), None),
                    };
                    self.notice = None;
                    return background(move || action.run(root, repo), Message::ActionFinished);
                }
            }
            Message::ActionFinished(Ok(path)) => {
                let copy = path.map_or_else(Task::none, iced::clipboard::write);
                return Task::batch([copy, self.focus_terminal()]);
            }
            Message::ActionFinished(Err(error)) => self.notice = Some(error),
            Message::DismissNotice => self.notice = None,
        }
        Task::none()
    }

    fn can_start_creation(&self) -> bool {
        self.store.is_some() && !self.picking && self.creation.is_none()
    }

    fn finish_bookmark(
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

    fn confirm_creation(&mut self) -> Task<Message> {
        let Some(draft) = self
            .creation
            .as_mut()
            .filter(|draft| !draft.submitting && !self.picking)
        else {
            return Task::none();
        };
        let path = match draft.destination() {
            Ok(path) => path,
            Err(error) => {
                self.notice = Some(error.into());
                return Task::none();
            }
        };
        draft.submitting = true;
        self.notice = None;
        match &draft.kind {
            create::Kind::Project => {
                background(move || git::new_project(&path), Message::ProjectCreated)
            }
            create::Kind::Worktree {
                project,
                anchor,
                source,
            } => {
                let (project, anchor, source) = (*project, anchor.clone(), source.clone());
                let branch = draft.branch.trim().to_owned();
                let base = if draft.base.trim().is_empty() {
                    "HEAD"
                } else {
                    draft.base.trim()
                }
                .to_owned();
                background(
                    move || git::add_worktree(&anchor, &source, &branch, &path, &base),
                    move |result| Message::WorktreeCreated(project, result),
                )
            }
        }
    }

    fn creation_error(&mut self, error: String) {
        if let Some(draft) = &mut self.creation {
            draft.submitting = false;
        }
        self.notice = Some(error);
    }

    fn cancel_sidebar_rename(&mut self) -> bool {
        if self
            .worktree_name_draft
            .as_ref()
            .is_some_and(|draft| self.selection.as_ref() != Some(&draft.target))
        {
            self.worktree_name_draft = None;
            true
        } else {
            false
        }
    }

    fn focus_terminal(&self) -> Task<Message> {
        match &self.console {
            ConsoleState::Attached(console)
                if console.is_open()
                    && self.worktree_name_draft.is_none()
                    && self.creation.is_none() =>
            {
                console.focus().map(Message::Terminal)
            }
            _ => Task::none(),
        }
    }

    fn resolve_action_target(&self, target: &Selection) -> Option<(&Project, Option<&Worktree>)> {
        let project = self.projects.iter().find(|p| p.id == target.project)?;
        let tree = match target.worktree.as_deref() {
            Some(path) => Some(project.worktree(path)?),
            None => None,
        };
        Some((project, tree))
    }

    fn rename_worktree(&mut self, target: &Selection, label: &str) -> Result<(), String> {
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

    fn bookmark(&mut self, mode: PickMode, checkout: Checkout) -> Result<ProjectId, String> {
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
                project.pull_requests.clear();
                project.refresh_id = None; // Ignore inventory in flight for the old anchor.
                Ok(id)
            }
            PickMode::CreateParent => Err("A parent folder is not a project bookmark.".into()),
        }
    }

    fn forget(&mut self, id: ProjectId) -> Result<(), String> {
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
            self.console = ConsoleState::Detached;
            self.worktree_name_draft = None;
            self.selection = self.projects.first().map(|p| Selection {
                project: p.id,
                worktree: None,
            });
        }
        Ok(())
    }

    fn save(&self, projects: impl IntoIterator<Item = ProjectBookmark>) -> Result<(), String> {
        self.store.as_ref().ok_or("Bookmarks are unavailable. Check the Application Support file and restart the app.")?.save(projects)
    }

    fn refresh_all(&mut self) -> Task<Message> {
        let ids: Vec<_> = self.projects.iter().map(|p| p.id).collect();
        Task::batch(ids.into_iter().map(|id| self.refresh(id)))
    }

    fn refresh(&mut self, id: ProjectId) -> Task<Message> {
        let Some(project) = self
            .projects
            .iter_mut()
            .find(|p| p.id == id && p.refresh_id.is_none())
        else {
            return Task::none();
        };
        let checkout = project.checkout.clone();
        let refresh_id = self.next_refresh_id;
        self.next_refresh_id += 1;
        project.refresh_id = Some(refresh_id);
        background(
            move || git::inventory(&checkout),
            move |result| Message::Refreshed(refresh_id, result),
        )
    }

    pub fn view(&self) -> Element<'_, Message> {
        // Keep the workspace in the same widget slot when toggling the sidebar.
        let sidebar = if self.sidebar_visible {
            self.sidebar()
        } else {
            Space::new().into()
        };
        let workspace = row![sidebar, self.workspace()]
            .spacing(if self.sidebar_visible { 4 } else { 0 })
            .height(Fill);
        let mut workspace = stack![workspace];
        if !self.sidebar_visible && self.sidebar_preview.visible {
            let panel = opaque(
                mouse_area(self.sidebar())
                    .on_enter(Message::SidebarHover(SidebarZone::Panel, true))
                    .on_exit(Message::SidebarHover(SidebarZone::Panel, false)),
            );
            // The stack keeps its base layout; the top layer only paints over the terminal.
            workspace = workspace.push(column![Space::new().height(36), panel]);
        }
        let mut page = column![workspace].spacing(4);
        if let Some(notice) = &self.notice {
            page = page.push(
                container(
                    row![
                        text(notice).size(13).width(Fill),
                        button("Dismiss")
                            .on_press(Message::DismissNotice)
                            .style(style::secondary),
                    ]
                    .spacing(12)
                    .align_y(Center),
                )
                .padding(10)
                .width(Fill)
                .style(style::glass),
            );
        }
        container(page).padding(4).width(Fill).height(Fill).into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let refreshing = self.projects.iter().any(|p| p.refresh_id.is_some());
        let project_menu = if self.can_start_creation() {
            row_menu::add(
                vec![ProjectChoice::New, ProjectChoice::Existing],
                Message::ProjectChoice,
            )
        } else {
            icon_button(icons::add(14, style::MUTED), "Add Project", None)
        };
        let toolbar = row![
            single_line("Projects", 13),
            project_menu,
            icon_button(
                icons::connecting(14, style::MUTED),
                if refreshing {
                    "Refreshing…"
                } else {
                    "Refresh worktrees and PR"
                },
                (!self.projects.is_empty() && !refreshing).then_some(Message::Refresh),
            ),
            icon_button(
                icons::sidebar(14, style::MUTED),
                if self.sidebar_visible {
                    "Hide sidebar"
                } else {
                    "Pin sidebar"
                },
                Some(Message::ToggleSidebar),
            ),
        ]
        .spacing(2)
        .align_y(Center);
        let mut list = column![].spacing(2);
        if self.projects.is_empty() {
            list = list.push(
                text(if self.store.is_some() {
                    "No bookmarked projects"
                } else {
                    "Bookmarks are unavailable"
                })
                .size(13)
                .color(style::MUTED),
            );
        }
        // Struct update syntax (`..base`) copies the user's font family and only
        // changes the weight, like `{ ...font, weight: 'bold' }` in JS.
        let bold = iced::Font {
            weight: iced::font::Weight::Bold,
            ..self.terminal_preferences.font.font_type
        };
        for project in &self.projects {
            let selection = self.selection.as_ref().filter(|s| s.project == project.id);
            let caret = if project.expanded {
                icons::chevron_down(15, style::TEXT)
            } else {
                icons::chevron_right(15, style::TEXT)
            };
            let name = project.name().replace(['\n', '\r'], " ");
            let project_button = button(
                row![
                    caret,
                    // Like `single_line`, but bold so projects stand apart from worktrees.
                    container(
                        text(name)
                            .size(13)
                            .font(bold)
                            .wrapping(iced::widget::text::Wrapping::None)
                    )
                    .clip(true)
                    .width(Fill),
                    text(if matches!(project.inventory, Some(Err(_))) {
                        "!"
                    } else {
                        ""
                    })
                    .color(style::RED),
                    Space::new().width(22),
                ]
                .spacing(4)
                .align_y(Center),
            )
            .width(Fill)
            .padding([5, 4])
            .on_press(Message::ToggleProject(project.id))
            .style(if selection.is_some() {
                style::secondary
            } else {
                style::project
            });
            list = list.push(with_row_menu(
                project_button,
                Selection {
                    project: project.id,
                    worktree: None,
                },
            ));
            if !project.expanded {
                continue;
            }
            let mut children =
                column![container(text("Worktrees").size(10).color(style::MUTED)).padding([1, 4]),]
                    .spacing(1);
            if project.worktrees().is_empty() {
                children = children.push(
                    text(if project.refresh_id.is_some() {
                        "Reading Git…"
                    } else {
                        "No worktrees to show"
                    })
                    .size(12)
                    .color(style::MUTED),
                );
            }
            for tree in project.worktrees() {
                let active =
                    selection.and_then(|s| s.worktree.as_deref()) == Some(tree.path.as_path());
                let pr = project.pull_request(&tree.path);
                let label = row![
                    project.worktree_icon(tree).view(13),
                    single_line(project.worktree_name(tree), 12),
                ]
                .spacing(5)
                .align_y(Center);
                let mut details = column![
                    text(project.worktree_name(tree)).size(12),
                    text(format!("{}  ·  {}", tree.label(), tree.path.display()))
                        .size(11)
                        .color(style::MUTED),
                ]
                .spacing(3);
                if let Some(pr) = pr {
                    details = details.push(text(pr.label()).size(11));
                }
                let label = tooltip(
                    label,
                    container(details)
                        .max_width(500)
                        .padding(7)
                        .style(style::console),
                    tooltip::Position::Right,
                )
                .delay(std::time::Duration::from_millis(600));
                let base = button(row![label, Space::new().width(22)])
                    .width(Fill)
                    .padding([3, 5])
                    .style(if active { style::selected } else { style::nav })
                    .on_press(Message::SelectWorktree(project.id, tree.path.clone()));
                let target = Selection {
                    project: project.id,
                    worktree: Some(tree.path.clone()),
                };
                let editor = if active {
                    None
                } else {
                    self.rename_editor(&target, tree.label(), true)
                };
                children = children.push(with_row_menu(base, target));
                if let Some(editor) = editor {
                    children = children.push(editor);
                }
            }
            // Indent worktrees under their project so the hierarchy reads at a glance.
            list = list.push(
                container(children)
                    .padding(iced::padding::top(1).bottom(4).left(12))
                    .width(Fill),
            );
        }
        container(column![toolbar, scrollable(list.width(Fill)).height(Fill),].spacing(5))
            .padding(6)
            .width(202)
            .height(Fill)
            .style(if self.sidebar_visible {
                style::glass
            } else {
                style::sidebar_preview
            })
            .into()
    }

    fn show_sidebar_button(&self) -> Element<'static, Message> {
        if self.sidebar_visible {
            Space::new().into()
        } else {
            mouse_area(
                button(icons::sidebar(15, style::PURPLE))
                    .padding(4)
                    .style(style::nav)
                    .on_press(Message::ToggleSidebar),
            )
            .on_enter(Message::SidebarHover(SidebarZone::Toggle, true))
            .on_exit(Message::SidebarHover(SidebarZone::Toggle, false))
            .into()
        }
    }

    fn workspace(&self) -> Element<'_, Message> {
        if let Some(draft) = &self.creation {
            return self.creation_form(draft);
        }
        match self.selected() {
            Some((project, selection)) => match selection
                .worktree
                .as_deref()
                .and_then(|p| project.worktree(p))
            {
                Some(tree) => self.worktree_workspace(project, tree),
                None => self.project_status(project),
            },
            None => container(
                column![
                    row![self.show_sidebar_button(), text("Open a project").size(20)]
                        .spacing(if self.sidebar_visible { 0 } else { 5 })
                        .align_y(Center),
                    text("Create a project or add an existing Git repository.").color(style::MUTED),
                    row![
                        button(text("New Project…").size(13))
                            .style(style::primary)
                            .on_press_maybe(
                                self.can_start_creation()
                                    .then_some(Message::ProjectChoice(ProjectChoice::New))
                            ),
                        button(text("Add Existing Repository…").size(13))
                            .style(style::secondary)
                            .on_press_maybe(
                                self.can_start_creation()
                                    .then_some(Message::ProjectChoice(ProjectChoice::Existing))
                            ),
                    ]
                    .spacing(6),
                ]
                .spacing(7),
            )
            .padding(12)
            .width(Fill)
            .height(Fill)
            .style(style::glass)
            .into(),
        }
    }

    fn creation_form<'a>(&'a self, draft: &'a create::Draft) -> Element<'a, Message> {
        let worktree = matches!(draft.kind, create::Kind::Worktree { .. });
        let title = if worktree {
            "New Worktree"
        } else {
            "New Project"
        };
        let ready = !draft.submitting && !self.picking;
        let field = |label: &'static str, input: Element<'a, Message>| {
            column![text(label).size(12).color(style::MUTED), input].spacing(3)
        };
        let mut form = column![
            row![self.show_sidebar_button(), text(title).size(18)]
                .spacing(4)
                .align_y(Center),
            text(if worktree {
                "Create a linked checkout on a new local branch."
            } else {
                "Create a folder, initialize Git, and add it to Projects."
            })
            .size(13)
            .color(style::MUTED),
        ]
        .spacing(8);
        if let create::Kind::Worktree { source, .. } = &draft.kind {
            form = form
                .push(field(
                    "Branch name",
                    text_input("feature/my-change", &draft.branch)
                        .id(CREATE_BRANCH_INPUT)
                        .on_input_maybe(ready.then_some(Message::CreateBranch))
                        .into(),
                ))
                .push(field(
                    "Base ref",
                    text_input("HEAD", &draft.base)
                        .on_input_maybe(ready.then_some(Message::CreateBase))
                        .into(),
                ))
                .push(
                    text(format!("Base is resolved from {}", source.display()))
                        .size(11)
                        .color(style::MUTED),
                );
        }
        let placeholder = draft.suggested_name();
        form = form.push(field(
            "Folder name",
            text_input(
                if placeholder.is_empty() {
                    "my-project"
                } else {
                    &placeholder
                },
                &draft.name,
            )
            .id(CREATE_NAME_INPUT)
            .on_input_maybe(ready.then_some(Message::CreateName))
            .into(),
        ));
        form = form.push(
            row![
                text(if draft.parent.as_os_str().is_empty() {
                    "Choose a location".into()
                } else {
                    draft.parent.display().to_string()
                })
                .width(Fill)
                .size(13),
                button(text("Choose Location…").size(12))
                    .style(style::secondary)
                    .on_press_maybe(ready.then_some(Message::Pick(PickMode::CreateParent))),
            ]
            .spacing(6)
            .align_y(Center),
        );
        let destination = draft.destination();
        if let Ok(path) = &destination {
            form = form.push(
                text(format!("Create at {}", path.display()))
                    .size(12)
                    .color(style::MUTED),
            );
        }
        form = form.push(
            row![
                button(
                    text(if draft.submitting {
                        "Creating…"
                    } else if worktree {
                        "Create Worktree"
                    } else {
                        "Create Project"
                    })
                    .size(13)
                )
                .style(style::primary)
                .on_press_maybe((ready && destination.is_ok()).then_some(Message::ConfirmCreation)),
                button(text("Cancel").size(13))
                    .style(style::secondary)
                    .on_press_maybe(ready.then_some(Message::CancelCreation)),
            ]
            .spacing(6),
        );
        container(scrollable(
            container(form).padding(14).max_width(620).width(Fill),
        ))
        .width(Fill)
        .height(Fill)
        .style(style::glass)
        .into()
    }

    fn project_status<'a>(&self, project: &'a Project) -> Element<'a, Message> {
        let message = match &project.inventory {
            Some(Err(error)) => column![
                text("Checkout unavailable").color(style::RED),
                text(error).size(14),
                text("Replace the anchor with a live checkout of the same repository.")
                    .size(14)
                    .color(style::MUTED)
            ]
            .spacing(9),
            Some(Ok(trees)) => column![
                text(format!(
                    "{} worktrees  ·  Select one to open its terminal.",
                    trees.len()
                ))
                .color(style::MUTED)
            ],
            None => column![text("Loading worktrees…").color(style::MUTED)],
        };
        container(scrollable(
            column![
                row![
                    if self.sidebar_visible {
                        icons::folder(16, style::PURPLE).into()
                    } else {
                        self.show_sidebar_button()
                    },
                    single_line(project.name(), 15),
                ]
                .spacing(7)
                .align_y(Center),
                text(project.checkout.root.display().to_string())
                    .size(13)
                    .color(style::MUTED),
                message,
            ]
            .spacing(8)
            .padding(10)
            .width(Fill),
        ))
        .width(Fill)
        .height(Fill)
        .style(style::glass)
        .into()
    }

    fn rename_editor<'a>(
        &'a self,
        target: &Selection,
        branch: &'a str,
        compact: bool,
    ) -> Option<Element<'a, Message>> {
        let draft = self
            .worktree_name_draft
            .as_ref()
            .filter(|draft| draft.target == *target)?;
        let input = text_input(branch, &draft.input)
            .id(WORKTREE_NAME_INPUT)
            .size(13)
            .padding([4, 7])
            .on_input(Message::EditWorktreeName)
            .on_submit(Message::SaveWorktreeName);
        let controls = row![
            button(text("Save").size(12))
                .style(style::primary)
                .on_press(Message::SaveWorktreeName),
            button(text("Cancel").size(12))
                .style(style::secondary)
                .on_press(Message::CancelWorktreeName),
        ]
        .spacing(4);
        Some(if compact {
            container(column![input, controls].spacing(4))
                .padding([2, 4])
                .into()
        } else {
            column![
                row![input, controls].spacing(6).align_y(Center),
                text("Display name only. Clear it to use the Git branch name.")
                    .size(11)
                    .color(style::MUTED),
            ]
            .spacing(2)
            .into()
        })
    }

    fn worktree_workspace<'a>(
        &'a self,
        project: &'a Project,
        tree: &'a Worktree,
    ) -> Element<'a, Message> {
        let name = project.worktree_name(tree);
        let mut actions = row![].spacing(6).align_y(Center);
        if let Some(pr) = project.pull_request(&tree.path) {
            actions = actions.push(tooltip(
                button(
                    row![
                        icons::pr_status(14, pr.status),
                        text(format!("PR #{}", pr.number)).size(12),
                    ]
                    .spacing(5)
                    .align_y(Center),
                )
                .padding([4, 7])
                .style(style::secondary)
                .on_press(Message::OpenPullRequest),
                container(text(pr.label()).size(11))
                    .padding([3, 5])
                    .style(style::console),
                tooltip::Position::Bottom,
            ));
        }
        let title = tooltip(
            row![icons::branch(14, style::PURPLE), single_line(name, 14)]
                .spacing(5)
                .width(Fill)
                .align_y(Center),
            container(
                column![
                    text(project.name()).size(12),
                    text(format!(
                        "{}  {}",
                        tree.label(),
                        tree.head
                            .as_deref()
                            .and_then(|head| head.get(..8))
                            .unwrap_or("")
                    ))
                    .size(11)
                    .color(style::MUTED),
                    text(tree.path.display().to_string())
                        .size(11)
                        .color(style::MUTED),
                ]
                .spacing(2),
            )
            .padding(6)
            .max_width(500)
            .style(style::console),
            tooltip::Position::Bottom,
        )
        .delay(std::time::Duration::from_millis(500));
        let target = Selection {
            project: project.id,
            worktree: Some(tree.path.clone()),
        };
        let editor = self.rename_editor(&target, tree.label(), false);
        let (badge, badge_color, action, body) = if let Some(reason) = tree.unavailable_reason() {
            let detail = tree
                .prunable
                .as_ref()
                .filter(|detail| !detail.is_empty())
                .map(String::as_str);
            ("Unavailable", style::RED, None, placeholder(reason, detail))
        } else {
            match &self.console {
                ConsoleState::Attached(console) => {
                    let failure = console.failure();
                    let open = console.is_open();
                    let mut content = column![console.view().map(Message::Terminal)]
                        .height(Fill)
                        .spacing(6);
                    if let Some(error) = failure {
                        content = content.push(text(error).size(13).color(style::RED));
                    }
                    (
                        if open { "Attached" } else { "Detached" },
                        if open { style::GREEN } else { style::MUTED },
                        Some(if open {
                            ("Detach", Message::Detach)
                        } else {
                            ("Reconnect", Message::Reconnect)
                        }),
                        content.into(),
                    )
                }
                ConsoleState::Failed(error) => (
                    "Could not connect",
                    style::RED,
                    Some(("Retry", Message::Reconnect)),
                    placeholder("Terminal not started", Some(error)),
                ),
                ConsoleState::Connecting(_) => (
                    "Connecting…",
                    style::MUTED,
                    Some(("Cancel", Message::Detach)),
                    placeholder(
                        "Opening tmux",
                        Some("Connecting to this worktree’s session."),
                    ),
                ),
                ConsoleState::Detached => (
                    "Detached",
                    style::MUTED,
                    Some(("Connect", Message::Reconnect)),
                    placeholder(
                        "No terminal attached",
                        Some("Connect to start or resume the tmux session for this worktree."),
                    ),
                ),
            }
        };
        let status = match badge {
            "Attached" => icons::connected(13, badge_color),
            "Connecting…" => icons::connecting(13, style::PURPLE),
            "Detached" => icons::empty(13, badge_color),
            _ => icons::warning(13, badge_color),
        };
        let terminal_status: Element<'_, Message> = if let Some((label, message)) = action {
            tooltip(
                button(
                    row![status, text(label).size(11)]
                        .spacing(3)
                        .align_y(Center),
                )
                .padding([2, 5])
                .style(style::secondary)
                .on_press(message),
                container(text(format!("Terminal: {badge}")).size(11))
                    .padding([3, 5])
                    .style(style::console),
                tooltip::Position::Bottom,
            )
            .delay(std::time::Duration::from_millis(500))
            .into()
        } else {
            row![status, text(badge).size(11).color(badge_color)]
                .spacing(3)
                .align_y(Center)
                .into()
        };
        let heading = row![
            self.show_sidebar_button(),
            title,
            terminal_status,
            actions.push(actions_menu(target)),
        ]
        .spacing(4)
        .align_y(Center);
        let mut header = column![heading].spacing(3);
        if let Some(editor) = editor {
            header = header.push(editor);
        }
        column![
            container(header)
                .padding([2, 4])
                .width(Fill)
                .style(style::glass),
            container(body)
                .width(Fill)
                .height(Fill)
                .style(style::console),
        ]
        .spacing(2)
        .into()
    }
}

fn window_scale(
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
        _ => None,
    }
}

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

fn menu_actions(worktree: bool) -> Vec<MenuAction> {
    let mut actions = Vec::new();
    if !worktree {
        actions.push(MenuAction::NewWorktree);
    }
    actions.extend(DirectoryAction::ALL.map(MenuAction::Directory));
    if worktree {
        actions.push(MenuAction::RenameWorktree);
    } else {
        actions.extend([MenuAction::ReplaceAnchor, MenuAction::ForgetProject]);
    }
    actions
}

fn with_row_menu<'a>(
    base: impl Into<Element<'a, Message>>,
    target: Selection,
) -> Element<'a, Message> {
    let menu = row_menu::show(
        format!("{target:?}"),
        menu_actions(target.worktree.is_some()),
        move |action| Message::MenuAction(target.clone(), action),
    );
    hover(
        base,
        container(menu)
            .padding([0, 3])
            .align_right(Fill)
            .center_y(Fill),
    )
}

fn actions_menu(target: Selection) -> Element<'static, Message> {
    pick_list(
        menu_actions(target.worktree.is_some()),
        None::<MenuAction>,
        move |action| Message::MenuAction(target.clone(), action),
    )
    .placeholder("Actions")
    .width(164)
    .text_size(12)
    .padding([4, 7])
    .style(style::action_picker)
    .menu_style(style::action_menu)
    .into()
}

fn single_line<'a>(
    label: impl iced::widget::text::IntoFragment<'a>,
    size: u32,
) -> iced::widget::Container<'a, Message> {
    container(
        text(label)
            .size(size)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .clip(true)
    .width(Fill)
}

fn icon_button(
    icon: Svg<'static>,
    label: &'static str,
    message: Option<Message>,
) -> Element<'static, Message> {
    tooltip(
        button(icon)
            .padding(4)
            .style(style::nav)
            .on_press_maybe(message),
        container(text(label).size(11))
            .padding([3, 5])
            .style(style::console),
        tooltip::Position::Bottom,
    )
    .delay(std::time::Duration::from_millis(400))
    .into()
}

fn placeholder<'a>(title: &'a str, detail: Option<&'a str>) -> Element<'a, Message> {
    let mut body = column![text(title).size(19)].spacing(6);
    if let Some(detail) = detail {
        body = body.push(text(detail).size(14).color(style::MUTED));
    }
    container(body).padding(6).width(Fill).height(Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_fixtures::{fixture, run_git};

    #[test]
    fn collapsed_sidebar_previews_until_the_pointer_leaves_both_zones() {
        let (mut app, _) = App::from_bookmarks(Err("test store".into()));
        let _ = app.update(Message::SidebarHover(SidebarZone::Toggle, true));
        assert!(!app.sidebar_preview.visible);
        let _ = app.update(Message::ToggleSidebar);
        let _ = app.update(Message::SidebarHover(SidebarZone::Toggle, true));
        assert!(app.sidebar_preview.visible);
        assert!(!app.sidebar_visible);

        let _ = app.update(Message::SidebarHover(SidebarZone::Toggle, false));
        let stale = app.sidebar_preview.epoch;
        let _ = app.update(Message::SidebarHover(SidebarZone::Panel, true));
        let _ = app.update(Message::SidebarPreviewExpired(stale));
        assert!(app.sidebar_preview.visible);

        let _ = app.update(Message::SidebarHover(SidebarZone::Panel, false));
        let _ = app.update(Message::SidebarPreviewExpired(app.sidebar_preview.epoch));
        assert!(!app.sidebar_preview.visible);

        let _ = app.update(Message::SidebarHover(SidebarZone::Toggle, true));
        let _ = app.update(Message::ToggleSidebar);
        assert!(app.sidebar_visible);
        assert!(!app.sidebar_preview.visible);
    }

    #[test]
    fn collapsing_sidebar_keeps_the_selected_worktree_and_connection() {
        let (_tmp, root) = fixture();
        let checkout = git::discover(&root).unwrap();
        let (mut app, _) = App::from_bookmarks(Err("test store".into()));
        let id = app.insert_project(checkout.into());
        app.select(id, Some(root.clone()));
        app.console = ConsoleState::Connecting(1);

        for visible in [false, true] {
            let _ = app.update(Message::ToggleSidebar);
            assert_eq!(app.sidebar_visible, visible);
            assert_eq!(
                app.selected().unwrap().1.worktree.as_deref(),
                Some(root.as_path())
            );
            assert!(matches!(app.console, ConsoleState::Connecting(1)));
        }
    }

    #[test]
    fn switching_worktrees_ignores_a_stale_connection_result() {
        let (_tmp, root) = fixture();
        let checkout = git::discover(&root).unwrap();
        let (mut app, _) = App::from_bookmarks(Err("test store".into()));
        let id = app.insert_project(checkout.into());
        app.select(id, Some(root.clone()));
        app.console = ConsoleState::Connecting(1);
        app.select(id, Some(root.join("new worktree")));
        app.console = ConsoleState::Connecting(2);
        let _ = app.update(Message::Prepared(1, Err("old error".into())));
        assert!(matches!(app.console, ConsoleState::Connecting(2)));
        let _ = app.update(Message::Prepared(2, Err("current error".into())));
        assert!(matches!(app.console, ConsoleState::Failed(ref error) if error == "current error"));
    }

    #[test]
    fn sidebar_actions_target_their_worktree_without_switching_the_console() {
        let (tmp, root) = fixture();
        run_git(
            &root,
            &["worktree", "add", "-qb", "topic", "../linked checkout"],
        );
        let checkout = git::discover(&root).unwrap();
        let linked = root.with_file_name("linked checkout");
        let config = tmp.path().join("config/projects.json");
        let (mut app, _) = App::from_bookmarks(Store::open_path(config.clone()));
        let id = app.bookmark(PickMode::Add, checkout.clone()).unwrap();
        app.projects[0].inventory = Some(git::inventory(&checkout));

        app.select(id, Some(root.clone()));
        app.console = ConsoleState::Connecting(9);
        let target = Selection {
            project: id,
            worktree: Some(linked.clone()),
        };
        assert_eq!(
            app.resolve_action_target(&target).unwrap().1.unwrap().path,
            linked
        );
        let _ = app.update(Message::MenuAction(target, MenuAction::RenameWorktree));
        let _ = app.update(Message::EditWorktreeName("Review".into()));
        let _ = app.update(Message::SaveWorktreeName);

        assert_eq!(
            app.selection.as_ref().unwrap().worktree.as_deref(),
            Some(root.as_path())
        );
        assert!(matches!(app.console, ConsoleState::Connecting(9)));
        assert_eq!(
            Store::open_path(config).unwrap().1[0].worktree_names[&linked],
            "Review"
        );
    }

    #[test]
    fn worktree_display_names_survive_restart_without_changing_git() {
        let (tmp, root) = fixture();
        run_git(
            &root,
            &[
                "worktree",
                "add",
                "-qb",
                "topic/long-branch",
                "../linked checkout",
            ],
        );
        let checkout = git::discover(&root).unwrap();
        let linked = root.with_file_name("linked checkout");
        let config = tmp.path().join("config/projects.json");
        let (mut app, _) = App::from_bookmarks(Store::open_path(config.clone()));
        let id = app.bookmark(PickMode::Add, checkout.clone()).unwrap();
        app.projects[0].inventory = Some(git::inventory(&checkout));
        app.select(id, Some(linked.clone()));
        let target = app.selection.clone().unwrap();
        app.rename_worktree(&target, "  Feature review  ").unwrap();

        let tree = app.projects[0].worktree(&linked).unwrap();
        assert_eq!(app.projects[0].worktree_name(tree), "Feature review");
        assert_eq!(tree.label(), "topic/long-branch");
        assert!(app.rename_worktree(&target, "Not\na single line").is_err());
        let next_anchor = git::discover(&linked).unwrap();
        assert_eq!(
            app.bookmark(PickMode::Replace(id), next_anchor.clone())
                .unwrap(),
            id
        );

        let (mut restarted, _) = App::from_bookmarks(Store::open_path(config.clone()));
        restarted.projects[0].inventory = Some(git::inventory(&next_anchor));
        let id = restarted.projects[0].id;
        restarted.select(id, Some(linked.clone()));
        let project = &restarted.projects[0];
        let tree = project.worktree(&linked).unwrap();
        assert_eq!(project.worktree_name(tree), "Feature review");
        assert_eq!(tree.label(), "topic/long-branch");
        let target = restarted.selection.clone().unwrap();
        restarted.rename_worktree(&target, " ").unwrap();
        assert!(
            Store::open_path(config).unwrap().1[0]
                .worktree_names
                .is_empty()
        );
        let project = &restarted.projects[0];
        assert_eq!(
            project.worktree_name(project.worktree(&linked).unwrap()),
            "topic/long-branch"
        );
    }

    /// A bookmarked fixture repository with a linked `topic` checkout and a loaded inventory.
    fn app_with_linked_checkout() -> (tempfile::TempDir, App, ProjectId, PathBuf, PathBuf) {
        let (tmp, root) = fixture();
        run_git(
            &root,
            &["worktree", "add", "-qb", "topic", "../linked checkout"],
        );
        let linked = root.with_file_name("linked checkout");
        let checkout = git::discover(&root).unwrap();
        let config = tmp.path().join("config/projects.json");
        let (mut app, _) = App::from_bookmarks(Store::open_path(config));
        let id = app.bookmark(PickMode::Add, checkout.clone()).unwrap();
        app.projects[0].inventory = Some(git::inventory(&checkout));
        (tmp, app, id, root, linked)
    }

    fn refresh_now(app: &mut App, id: ProjectId) {
        let _ = app.refresh(id);
        let project = app.projects.iter().find(|p| p.id == id).unwrap();
        let refresh_id = project.refresh_id.unwrap();
        let inventory = git::inventory(&project.checkout);
        let _ = app.update(Message::Refreshed(refresh_id, inventory));
    }

    #[test]
    fn startup_restores_bookmarks_and_refreshes_each_project() {
        let (tmp, first) = fixture();
        let second = tmp.path().join("second");
        std::fs::create_dir(&second).unwrap();
        run_git(&second, &["init", "-q", "-b", "main"]);
        let config = tmp.path().join("config/projects.json");
        let (store, _) = Store::open_path(config.clone()).unwrap();
        store
            .save([
                git::discover(&first).unwrap().into(),
                git::discover(&second).unwrap().into(),
            ])
            .unwrap();

        let (app, refresh) = App::from_bookmarks(Store::open_path(config));
        assert_eq!(app.projects.len(), 2);
        assert!(app.projects[0].expanded);
        assert!(!app.projects[1].expanded);
        assert_eq!(
            app.selection,
            Some(Selection {
                project: app.projects[0].id,
                worktree: None
            })
        );
        assert_eq!(refresh.units(), 2);
        assert!(matches!(app.console, ConsoleState::Detached));
    }

    #[test]
    fn unreadable_bookmarks_disable_adding_projects_and_show_the_error() {
        let (mut app, _) = App::from_bookmarks(Err("Could not read projects.json".into()));
        assert_eq!(app.notice.as_deref(), Some("Could not read projects.json"));
        assert_eq!(app.update(Message::Pick(PickMode::Add)).units(), 0);
        assert!(!app.picking);
    }

    #[test]
    fn selecting_a_worktree_connects_its_terminal_and_looks_up_its_pr() {
        let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
        let task = app.update(Message::SelectWorktree(id, linked.clone()));
        assert!(matches!(app.console, ConsoleState::Connecting(_)));
        assert_eq!(task.units(), 2, "tmux connection and PR lookup");

        // Selecting it again while connecting does not start a second connection.
        let ConsoleState::Connecting(first) = app.console else {
            unreachable!()
        };
        let _ = app.update(Message::Reconnect);
        assert!(matches!(app.console, ConsoleState::Connecting(id) if id == first));

        // Detach keeps the selection; Connect starts a new attempt.
        let _ = app.update(Message::Detach);
        assert!(matches!(app.console, ConsoleState::Detached));
        assert_eq!(app.update(Message::Reconnect).units(), 1);
        assert!(matches!(app.console, ConsoleState::Connecting(n) if n > first));

        // A detached HEAD checkout has no branch to look up.
        run_git(&root, &["checkout", "-q", "--detach"]);
        refresh_now(&mut app, id);
        let task = app.update(Message::SelectWorktree(id, root));
        assert_eq!(task.units(), 1, "tmux connection only");
    }

    #[test]
    fn unavailable_worktrees_never_start_a_terminal() {
        let (_tmp, mut app, id, _root, linked) = app_with_linked_checkout();
        std::fs::remove_dir_all(&linked).unwrap();
        refresh_now(&mut app, id);
        let task = app.update(Message::SelectWorktree(id, linked.clone()));
        assert_eq!(task.units(), 0);
        assert!(matches!(app.console, ConsoleState::Detached));
        assert_eq!(
            app.selection.as_ref().unwrap().worktree.as_deref(),
            Some(linked.as_path())
        );
    }

    #[test]
    fn refresh_detaches_or_clears_a_selection_that_changed_outside_the_app() {
        let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
        app.select(id, Some(linked.clone()));
        app.console = ConsoleState::Connecting(1);

        // The directory disappears: keep the row, but detach the terminal.
        std::fs::rename(&linked, root.with_file_name("moved")).unwrap();
        refresh_now(&mut app, id);
        assert!(matches!(app.console, ConsoleState::Detached));
        assert_eq!(
            app.selection.as_ref().unwrap().worktree.as_deref(),
            Some(linked.as_path())
        );

        // Git forgets it: fall back to the project overview.
        run_git(&root, &["worktree", "prune"]);
        app.console = ConsoleState::Connecting(2);
        refresh_now(&mut app, id);
        assert_eq!(app.selection.as_ref().unwrap().worktree, None);
        assert!(matches!(app.console, ConsoleState::Detached));
    }

    #[test]
    fn stale_refresh_results_are_ignored_after_replacing_the_anchor() {
        let (_tmp, mut app, id, _root, linked) = app_with_linked_checkout();
        let _ = app.refresh(id);
        let stale = app.projects[0].refresh_id.unwrap();
        assert_eq!(
            app.refresh(id).units(),
            0,
            "one refresh per project at a time"
        );
        app.bookmark(PickMode::Replace(id), git::discover(&linked).unwrap())
            .unwrap();
        let _ = app.update(Message::Refreshed(stale, Err("old anchor".into())));
        assert!(app.projects[0].inventory.is_none());
    }

    #[test]
    fn replacing_an_anchor_with_another_repository_is_rejected() {
        let (tmp, mut app, id, root, _linked) = app_with_linked_checkout();
        let other = tmp.path().join("other repo");
        std::fs::create_dir(&other).unwrap();
        run_git(&other, &["init", "-q"]);
        let error = app
            .bookmark(PickMode::Replace(id), git::discover(&other).unwrap())
            .unwrap_err();
        assert!(error.contains("different repository"), "{error}");
        assert_eq!(app.projects[0].checkout.root, root);
    }

    #[test]
    fn adding_another_checkout_of_a_bookmarked_repository_explains_replace_anchor() {
        let (_tmp, mut app, id, _root, linked) = app_with_linked_checkout();
        let linked = git::discover(&linked).unwrap();
        assert_eq!(app.bookmark(PickMode::Add, linked).unwrap(), id);
        assert!(app.notice.as_deref().unwrap().contains("Replace Anchor"));
    }

    #[test]
    fn forgetting_the_selected_project_detaches_and_selects_the_next() {
        let (tmp, mut app, id, _root, linked) = app_with_linked_checkout();
        let other = tmp.path().join("other repo");
        std::fs::create_dir(&other).unwrap();
        run_git(&other, &["init", "-q"]);
        let next = app
            .bookmark(PickMode::Add, git::discover(&other).unwrap())
            .unwrap();
        let _ = app.update(Message::SelectWorktree(id, linked.clone()));
        let request = app.pr_request_id;
        let _ = app.update(Message::PullRequestFound(
            id,
            linked,
            request,
            Some(PullRequest::for_test(5)),
        ));
        app.console = ConsoleState::Connecting(3);
        assert!(app.selected_pull_request().is_some());

        let _ = app.update(Message::Forget(id));
        assert_eq!(app.projects.len(), 1);
        assert_eq!(
            app.selection,
            Some(Selection {
                project: next,
                worktree: None
            })
        );
        assert!(matches!(app.console, ConsoleState::Detached));
        assert!(app.selected_pull_request().is_none());
    }

    #[test]
    fn refreshing_checks_all_worktrees_and_invalidates_in_flight_pr_results() {
        let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
        let _ = app.refresh(id);
        let git_request = app.projects[0].refresh_id.unwrap();
        let inventory = git::inventory(&app.projects[0].checkout);
        let _ = app.update(Message::Refreshed(git_request, inventory));
        let prior = app.projects[0].pull_requests[&linked].request_id.unwrap();
        assert!(app.projects[0].pull_requests[&root].request_id.is_some());

        let _ = app.refresh(id);
        let git_request = app.projects[0].refresh_id.unwrap();
        let inventory = git::inventory(&app.projects[0].checkout);
        let _ = app.update(Message::Refreshed(git_request, inventory));
        let current = app.projects[0].pull_requests[&linked].request_id.unwrap();
        assert_ne!(prior, current);
        let _ = app.update(Message::PullRequestFound(
            id,
            linked.clone(),
            prior,
            Some(PullRequest::for_test(8)),
        ));
        assert!(app.projects[0].pull_request(&linked).is_none());
        let _ = app.update(Message::PullRequestFound(
            id,
            linked.clone(),
            current,
            Some(PullRequest::for_test(9)),
        ));
        assert_eq!(app.projects[0].pull_request(&linked).unwrap().number, 9);
    }

    #[test]
    fn sidebar_icon_marks_the_main_checkout_pr_state_and_unavailable_worktrees() {
        let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
        let icon = |app: &App, path: &Path| {
            let project = &app.projects[0];
            project.worktree_icon(project.worktree(path).unwrap())
        };
        assert_eq!(icon(&app, &root), WorktreeIcon::Main);
        assert_eq!(icon(&app, &linked), WorktreeIcon::Worktree);

        // A PR on the main checkout doesn't replace its unique icon.
        let _ = app.update(Message::SelectWorktree(id, root.clone()));
        let request = app.pr_request_id;
        let _ = app.update(Message::PullRequestFound(
            id,
            root.clone(),
            request,
            Some(PullRequest::for_test(3)),
        ));
        assert_eq!(icon(&app, &root), WorktreeIcon::Main);

        let _ = app.update(Message::SelectWorktree(id, linked.clone()));
        let request = app.pr_request_id;
        let _ = app.update(Message::PullRequestFound(
            id,
            linked.clone(),
            request,
            Some(PullRequest::for_test(4)),
        ));
        assert_eq!(
            icon(&app, &linked),
            WorktreeIcon::PullRequest(pull_request::Status::Open)
        );

        if let Some(Ok(trees)) = &mut app.projects[0].inventory {
            trees[1].directory_exists = false;
        }
        assert_eq!(icon(&app, &linked), WorktreeIcon::Unavailable);
    }

    #[test]
    fn pr_badges_are_kept_for_unselected_worktrees_and_ignore_stale_results() {
        let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
        let _ = app.update(Message::SelectWorktree(id, linked.clone()));
        let request = app.pr_request_id;
        let _ = app.update(Message::SelectWorktree(id, root));
        let _ = app.update(Message::PullRequestFound(
            id,
            linked.clone(),
            request,
            Some(PullRequest::for_test(12)),
        ));
        assert_eq!(app.projects[0].pull_request(&linked).unwrap().number, 12);
        assert!(app.selected_pull_request().is_none());

        let _ = app.update(Message::SelectWorktree(id, linked.clone()));
        let current = app.pr_request_id;
        assert!(current > request);
        let _ = app.update(Message::PullRequestFound(
            id,
            linked.clone(),
            request,
            Some(PullRequest::for_test(15)),
        ));
        assert_eq!(app.selected_pull_request().unwrap().number, 12);
        let _ = app.update(Message::PullRequestFound(id, linked, current, None));
        assert!(app.selected_pull_request().is_none());
    }

    #[test]
    fn rename_validates_input_and_cancel_discards_the_draft() {
        let (_tmp, mut app, id, _root, linked) = app_with_linked_checkout();
        let target = Selection {
            project: id,
            worktree: Some(linked.clone()),
        };
        assert!(app.rename_worktree(&target, &"x".repeat(81)).is_err());
        assert!(app.rename_worktree(&target, "tab\there").is_err());
        assert!(app.rename_worktree(&target, &"x".repeat(80)).is_ok());

        let _ = app.update(Message::MenuAction(
            target.clone(),
            MenuAction::RenameWorktree,
        ));
        assert_eq!(
            app.worktree_name_draft.as_ref().unwrap().input,
            "x".repeat(80),
            "editor starts with the current display name"
        );
        let _ = app.update(Message::EditWorktreeName("Discard me".into()));
        let _ = app.update(Message::CancelWorktreeName);
        assert!(app.worktree_name_draft.is_none());
        assert_eq!(app.projects[0].worktree_names[&linked], "x".repeat(80));

        // A project is not a worktree.
        let project = Selection {
            project: id,
            worktree: None,
        };
        let _ = app.update(Message::MenuAction(project, MenuAction::RenameWorktree));
        assert!(app.worktree_name_draft.is_none());
    }

    #[test]
    fn row_menu_mirrors_the_header_actions() {
        use DirectoryAction::*;
        assert_eq!(
            menu_actions(true),
            [
                MenuAction::Directory(Finder),
                MenuAction::Directory(VsCode),
                MenuAction::Directory(CopyPath),
                MenuAction::RenameWorktree,
            ]
        );
        assert_eq!(
            menu_actions(false),
            [
                MenuAction::NewWorktree,
                MenuAction::Directory(Finder),
                MenuAction::Directory(VsCode),
                MenuAction::Directory(CopyPath),
                MenuAction::ReplaceAnchor,
                MenuAction::ForgetProject,
            ]
        );
        assert_eq!(MenuAction::RenameWorktree.to_string(), "Rename Worktree");
    }

    #[test]
    fn deduplicates_linked_checkouts_and_persists_anchor_replacement() {
        let (tmp, root) = fixture();
        run_git(
            &root,
            &["worktree", "add", "-qb", "topic", "../linked checkout"],
        );
        let main = git::discover(&root).unwrap();
        let linked = git::discover(&root.with_file_name("linked checkout")).unwrap();
        let config = tmp.path().join("config/projects.json");
        let (mut app, _) = App::from_bookmarks(Store::open_path(config.clone()));
        let id = app.bookmark(PickMode::Add, linked.clone()).unwrap();
        assert_eq!(app.bookmark(PickMode::Add, main.clone()).unwrap(), id);
        assert_eq!(app.projects.len(), 1);
        assert_eq!(Store::open_path(config.clone()).unwrap().1, [linked.into()]);

        run_git(&root, &["worktree", "remove", "../linked checkout"]);
        assert!(git::inventory(&app.projects[0].checkout).is_err());
        assert_eq!(
            app.bookmark(PickMode::Replace(id), main.clone()).unwrap(),
            id
        );
        let (mut restarted, _) = App::from_bookmarks(Store::open_path(config.clone()));
        assert_eq!(restarted.projects[0].checkout, main);
        restarted.forget(restarted.projects[0].id).unwrap();
        assert!(Store::open_path(config).unwrap().1.is_empty());
        assert!(git::inventory(&main).is_ok());
    }
}
