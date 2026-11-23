//! `App::update`: the reducer. Each arm hands its message to a named handler,
//! most of which live in the module for their area.

use super::pull_requests::Lookup;
use super::{
    App, MenuAction, Message, PickMode, RenameDraft, Selection, WORKTREE_NAME_INPUT, background,
};
use crate::{git, quit, settings};
use iced::Task;
use iced::widget::operation;

impl App {
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::QuitRequested(id) => self.quit_requested(id),
            Message::QuitAnswered(confirmed) => self.quit_answered(confirmed),
            Message::Pick(mode) => self.pick(mode),
            Message::FolderPicked(mode, path) => self.folder_picked(mode, path),
            Message::Discovered(mode, result) => {
                self.picking = false;
                self.finish_bookmark(mode, result)
            }
            Message::ProjectChoice(choice) => self.project_choice(choice),
            Message::NewWorktree(id) => self.new_worktree(id),
            Message::Creation(message) => self.creation_message(message),
            Message::ProjectCreated(result) => self.project_created(result),
            Message::WorktreeCreated(id, result) => self.worktree_created(id, result),
            Message::Refresh => self.refresh_all(),
            Message::WindowFocused => self.refresh_all_automatically(),
            Message::WatchTick => self.watch_tick(),
            Message::Watched(request, fingerprints) => self.watched(request, fingerprints),
            Message::ToggleSidebar => self.toggle_sidebar(),
            Message::SidebarHover(zone, entered) => self.sidebar_hover(zone, entered),
            Message::SidebarPreviewExpired(epoch) => self.sidebar_preview_expired(epoch),
            Message::Refreshed(refresh_id, result) => self.refreshed(refresh_id, result),
            Message::ToggleProject(id) => {
                self.toggle_project(id);
                Task::none()
            }
            Message::SelectWorktree(id, path) => {
                self.select(id, Some(path.clone()));
                Task::batch([
                    self.connect(),
                    self.lookup_pull_requests(id, Lookup::Worktree(&path)),
                ])
            }
            Message::PullRequestFound(id, path, request, pr) => {
                self.pull_request_found(id, path, request, pr);
                Task::none()
            }
            Message::OpenPullRequest => self.open_pull_request(),
            Message::PullRequestOpened(result) => {
                if let Err(error) = result {
                    self.notice = Some(error);
                }
                Task::none()
            }
            Message::Reconnect => self.connect(),
            Message::Prepared(id, result) => self.prepared(id, result),
            Message::Detach => {
                self.detach();
                Task::none()
            }
            Message::WindowOpened(id) => window_opened(id),
            Message::WindowScale(scale) => {
                self.window_rescaled(scale);
                Task::none()
            }
            Message::Terminal(event) => {
                self.terminal_event(event);
                Task::none()
            }
            Message::Forget(id) => {
                if let Err(error) = self.forget(id) {
                    self.notice = Some(error);
                }
                Task::none()
            }
            Message::MenuAction(target, action) => self.menu_action(target, action),
            Message::EditWorktreeName(name) => {
                if let Some(draft) = &mut self.worktree_name_draft {
                    draft.input = name;
                }
                Task::none()
            }
            Message::SaveWorktreeName => self.save_worktree_name(),
            Message::CancelWorktreeName => {
                self.worktree_name_draft = None;
                self.focus_terminal()
            }
            Message::ActionFinished(Ok(path)) => {
                let copy = path.map_or_else(Task::none, iced::clipboard::write);
                Task::batch([copy, self.focus_terminal()])
            }
            Message::ActionFinished(Err(error)) => {
                self.notice = Some(error);
                Task::none()
            }
            Message::DismissNotice => {
                self.notice = None;
                Task::none()
            }
            Message::OpenSettings => {
                if self.settings.is_none() {
                    self.settings = Some(settings::Draft::new(self.palette));
                }
                Task::none()
            }
            Message::Settings(message) => self.settings(message),
        }
    }

    fn quit_requested(&mut self, id: iced::window::Id) -> Task<Message> {
        if self.quit_pending || self.picking {
            return Task::none();
        }
        self.quit_pending = true;
        quit::confirm(id).map(Message::QuitAnswered)
    }

    fn quit_answered(&mut self, confirmed: bool) -> Task<Message> {
        if !self.quit_pending {
            return Task::none();
        }
        self.quit_pending = false;
        if confirmed {
            return iced::exit();
        }
        self.focus_terminal()
    }

    fn pick(&mut self, mode: PickMode) -> Task<Message> {
        if self.picking || self.store.is_none() {
            return Task::none();
        }
        self.picking = true;
        self.notice = None;
        // An app-modal folder panel, created on the main thread.
        let picker = rfd::AsyncFileDialog::new()
            .set_title(match mode {
                PickMode::CreateParent => "Choose a parent folder",
                _ => "Choose an existing Git checkout",
            })
            .pick_folder();
        Task::perform(picker, move |folder| {
            Message::FolderPicked(mode, folder.map(|handle| handle.path().to_path_buf()))
        })
    }

    fn folder_picked(&mut self, mode: PickMode, path: Option<std::path::PathBuf>) -> Task<Message> {
        match (mode, path) {
            (PickMode::CreateParent, path) => {
                self.picking = false;
                if let Some(draft) = self.creation.as_mut()
                    && let Some(path) = path
                {
                    draft.parent = path;
                }
                Task::none()
            }
            // `picking` stays set until discovery finishes.
            (mode, Some(path)) => background(
                move || git::discover(&path),
                move |result| Message::Discovered(mode, result),
            ),
            (_, None) => {
                self.picking = false;
                Task::none()
            }
        }
    }

    fn menu_action(&mut self, target: Selection, action: MenuAction) -> Task<Message> {
        match action {
            MenuAction::RenameWorktree => {
                if let Some((project, Some(tree))) = self.resolve_action_target(&target) {
                    let input = project
                        .worktree_names
                        .get(&tree.path)
                        .cloned()
                        .unwrap_or_default();
                    self.worktree_name_draft = Some(RenameDraft { target, input });
                    return operation::focus(WORKTREE_NAME_INPUT);
                }
                Task::none()
            }
            MenuAction::NewWorktree | MenuAction::ReplaceAnchor | MenuAction::ForgetProject => {
                if !(self.can_start_creation()
                    && target.worktree.is_none()
                    && self.resolve_action_target(&target).is_some())
                {
                    return Task::none();
                }
                let message = match action {
                    MenuAction::ReplaceAnchor => Message::Pick(PickMode::Replace(target.project)),
                    MenuAction::ForgetProject => Message::Forget(target.project),
                    _ => Message::NewWorktree(target.project),
                };
                self.update(message)
            }
            MenuAction::Directory(action) => {
                let Some((project, tree)) = self.resolve_action_target(&target) else {
                    return Task::none();
                };
                let (root, repo) = match tree {
                    Some(tree) => (tree.path.clone(), Some(project.checkout.common_dir.clone())),
                    None => (project.checkout.root.clone(), None),
                };
                self.notice = None;
                background(move || action.run(root, repo), Message::ActionFinished)
            }
        }
    }

    fn save_worktree_name(&mut self) -> Task<Message> {
        let Some(draft) = self.worktree_name_draft.clone() else {
            return Task::none();
        };
        match self.rename_worktree(&draft.target, &draft.input) {
            Ok(()) => {
                self.worktree_name_draft = None;
                self.focus_terminal()
            }
            Err(error) => {
                self.notice = Some(error);
                Task::none()
            }
        }
    }

    fn settings(&mut self, message: settings::Message) -> Task<Message> {
        let Some(draft) = &mut self.settings else {
            return Task::none();
        };
        match draft.update(message) {
            None => Task::none(),
            Some(settings::Outcome::Cancel) => {
                self.settings = None;
                self.focus_terminal()
            }
            Some(settings::Outcome::Save(palette)) => {
                let saved = self
                    .theme_store
                    .as_ref()
                    .ok_or_else(|| "Theme settings are unavailable: could not locate the Application Support directory.".to_owned())
                    .and_then(|store| store.save(&palette));
                match saved {
                    Ok(()) => {
                        self.palette = palette;
                        self.settings = None;
                        self.notice = None;
                        self.focus_terminal()
                    }
                    // Keep the modal open so the edits aren't lost.
                    Err(error) => {
                        self.notice = Some(error);
                        Task::none()
                    }
                }
            }
        }
    }
}

fn window_opened(id: iced::window::Id) -> Task<Message> {
    let scale = iced::window::scale_factor(id).map(Message::WindowScale);
    // `cfg!`-style attributes compile only one of these statements, so the
    // quit shortcut (AppKit) is installed on macOS alone.
    #[cfg(target_os = "macos")]
    return Task::batch([
        scale,
        iced::window::run(id, quit::install_shortcut).discard(),
    ]);
    #[cfg(not(target_os = "macos"))]
    return scale;
}
