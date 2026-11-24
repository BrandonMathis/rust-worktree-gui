//! `App::update`: the reducer. Each arm hands its message to a named handler,
//! most of which live in the module for their area.

use super::pull_requests::Lookup;
use super::{
    App, Confirmation, MenuAction, Message, PickMode, ProjectId, RenameDraft, Selection,
    WORKTREE_NAME_INPUT, background,
};
use crate::{confirmation, git, settings};
use iced::widget::operation;
use iced::{Task, window};

/// How long quitting waits for running agent bridges to stop their agents.
const AGENT_SHUTDOWN: std::time::Duration = std::time::Duration::from_millis(600);

impl App {
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::QuitRequested(id) => self.quit_requested(id),
            Message::ConfirmationAnswered(action, confirmed) => {
                self.confirmation_answered(action, confirmed)
            }
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
            Message::SessionPollTick => self.poll_sessions(),
            Message::SessionsPolled(request, reports) => {
                self.sessions_polled(request, reports);
                Task::none()
            }
            Message::ToggleSidebar => self.toggle_sidebar(),
            Message::SidebarHover(zone, entered) => self.sidebar_hover(zone, entered),
            Message::SidebarPreviewExpired(epoch) => self.sidebar_preview_expired(epoch),
            Message::Refreshed(refresh_id, result) => self.refreshed(refresh_id, result),
            Message::ToggleProject(id) => {
                self.toggle_project(id);
                Task::none()
            }
            Message::ShowAllWorktrees(id, show) => {
                if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                    project.show_all_worktrees = show;
                }
                Task::none()
            }
            Message::CloseUnavailableWorktrees => {
                self.unavailable_list = None;
                self.focus_terminal()
            }
            Message::ReorderProject { source, before } => {
                if let Err(error) = self.reorder_project(source, before) {
                    self.notice = Some(error);
                }
                Task::none()
            }
            Message::SelectWorktree(id, path) => {
                // Clicking the selected worktree again must not re-run `gh`.
                let changed = self.selection.as_ref().is_none_or(|selection| {
                    selection.project != id || selection.worktree.as_ref() != Some(&path)
                });
                let has_branch = self
                    .projects
                    .iter()
                    .find(|p| p.id == id)
                    .and_then(|p| p.worktree(&path))
                    .and_then(crate::pull_request::branch_to_match)
                    .is_some();
                self.select(id, Some(path.clone()));
                // The user is about to look at this session: its news is seen.
                self.mark_session_seen(id, &path);
                let lookup = if changed && has_branch {
                    self.lookup_pull_requests(id, Lookup::All)
                } else {
                    Task::none()
                };
                Task::batch([self.connect(), lookup])
            }
            Message::PullRequestsListed(id, request, prs) => {
                self.pull_requests_listed(id, request, prs);
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
            Message::Switched(id, result) => self.switched(id, result),
            Message::Detach => {
                self.detach();
                Task::none()
            }
            Message::WindowOpened(id) => window_opened(id),
            Message::WindowScale(scale) => {
                self.window_rescaled(scale);
                Task::none()
            }
            Message::FileDropped(path) => {
                self.paste_dropped_path(&path);
                Task::none()
            }
            Message::Terminal(event) => {
                self.terminal_event(event);
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
            Message::Managed(message) => self.managed_message(message),
        }
    }

    fn quit_requested(&mut self, id: window::Id) -> Task<Message> {
        if self.pending_confirmation.is_some() || self.picking {
            return Task::none();
        }
        self.pending_confirmation = Some(Confirmation::Quit);
        // tmux sessions outlive the app; a managed agent's turn does not.
        let detail = if self.agents_working() {
            "Agents that are working will be stopped. Your tmux sessions will keep running."
        } else {
            "Your tmux sessions will keep running."
        };
        confirmation::show(id, "Quit Worktree Browser?", detail.into(), "Quit")
            .map(|confirmed| Message::ConfirmationAnswered(Confirmation::Quit, confirmed))
    }

    fn request_project_removal(&mut self, id: ProjectId, name: String) -> Task<Message> {
        let action = Confirmation::RemoveProject(id);
        self.pending_confirmation = Some(action);
        window::latest()
            .then(move |id| match id {
                Some(id) => confirmation::show(
                    id,
                    "Remove Project?",
                    format!("Remove “{name}” from Worktree Browser? Repository files, worktrees, and tmux sessions will be left unchanged."),
                    "Remove Project",
                ),
                None => Task::done(false),
            })
            .map(move |confirmed| Message::ConfirmationAnswered(action, confirmed))
    }

    fn confirmation_answered(&mut self, action: Confirmation, confirmed: bool) -> Task<Message> {
        if self.pending_confirmation != Some(action) {
            return Task::none();
        }
        self.pending_confirmation = None;
        if confirmed {
            match action {
                Confirmation::Quit => {
                    if self.stop_agents() {
                        // Give the bridges a moment to stop their agents;
                        // exiting at once would only kill the bridges.
                        return Task::future(async { tokio::time::sleep(AGENT_SHUTDOWN).await })
                            .then(|()| iced::exit());
                    }
                    return iced::exit();
                }
                Confirmation::RemoveProject(id) => {
                    if let Err(error) = self.remove_bookmark(id) {
                        self.notice = Some(error);
                    }
                }
            }
        }
        self.focus_terminal()
    }

    fn pick(&mut self, mode: PickMode) -> Task<Message> {
        if self.picking || self.pending_confirmation.is_some() || self.store.is_none() {
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
            MenuAction::UseAgent | MenuAction::UseTerminal => {
                let Some((_, Some(tree))) = self.resolve_action_target(&target) else {
                    return Task::none();
                };
                let path = tree.path.clone();
                self.switch_workspace(target, &path, action == MenuAction::UseAgent)
            }
            MenuAction::NewWorktree | MenuAction::ReplaceAnchor | MenuAction::RemoveProject => {
                if !self.can_start_creation() {
                    return Task::none();
                }
                let Some((project, None)) = self.resolve_action_target(&target) else {
                    return Task::none();
                };
                match action {
                    MenuAction::ReplaceAnchor => {
                        self.update(Message::Pick(PickMode::Replace(target.project)))
                    }
                    MenuAction::RemoveProject => {
                        let name = project.name().replace(['\n', '\r'], " ");
                        self.request_project_removal(target.project, name)
                    }
                    _ => self.update(Message::NewWorktree(target.project)),
                }
            }
            MenuAction::UnavailableWorktrees(_) => {
                if let Some((project, None)) = self.resolve_action_target(&target) {
                    self.unavailable_list = Some(project.id);
                }
                Task::none()
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

    /// Switches a worktree between the terminal and a managed agent. Nothing
    /// is stopped: its tmux session and its agent conversation both keep
    /// running, and switching back shows them again.
    fn switch_workspace(
        &mut self,
        target: Selection,
        path: &std::path::Path,
        managed: bool,
    ) -> Task<Message> {
        if let Err(error) = self.set_managed(target.project, path, managed) {
            self.notice = Some(error);
            return Task::none();
        }
        if self.selection.as_ref() != Some(&target) {
            return Task::none();
        }
        // The selected worktree changed kind: swap the pane it shows.
        self.leave_session();
        self.keep_or_detach_terminal();
        self.connect()
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
        iced::window::run(id, crate::quit::install_shortcut).discard(),
        iced::window::run(id, crate::backdrop::install).discard(),
    ]);
    #[cfg(not(target_os = "macos"))]
    return scale;
}
