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
    app.projects[0].inventory = Some(git::inventory(&checkout).unwrap());

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
    app.projects[0].inventory = Some(git::inventory(&checkout).unwrap());
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
    restarted.projects[0].inventory = Some(git::inventory(&next_anchor).unwrap());
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
    app.projects[0].inventory = Some(git::inventory(&checkout).unwrap());
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
fn expanding_another_project_keeps_the_selected_worktree_terminal_and_pr() {
    let (tmp, mut app, id, _root, linked) = app_with_linked_checkout();
    let other = tmp.path().join("other repo");
    std::fs::create_dir(&other).unwrap();
    run_git(&other, &["init", "-q"]);
    let other = app
        .bookmark(PickMode::Add, git::discover(&other).unwrap())
        .unwrap();
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    let request = app.pr_request_id;
    let _ = app.update(Message::PullRequestFound(
        id,
        linked.clone(),
        request,
        Some(PullRequest::for_test(2)),
    ));
    app.console = ConsoleState::Connecting(6);

    for expanded in [true, false] {
        let task = app.update(Message::ToggleProject(other));
        assert_eq!(task.units(), 0);
        assert_eq!(app.projects[1].expanded, expanded);
        assert_eq!(
            app.selection,
            Some(Selection {
                project: id,
                worktree: Some(linked.clone())
            })
        );
        assert!(matches!(app.console, ConsoleState::Connecting(6)));
        assert_eq!(app.selected_pull_request().unwrap().number, 2);
    }
}

#[test]
fn failed_refresh_keeps_the_selection_terminal_and_pr_badges() {
    let (_tmp, mut app, id, _root, linked) = app_with_linked_checkout();
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    let request = app.pr_request_id;
    let _ = app.update(Message::PullRequestFound(
        id,
        linked.clone(),
        request,
        Some(PullRequest::for_test(7)),
    ));
    app.console = ConsoleState::Connecting(4);

    let _ = app.refresh(id);
    let refresh_id = app.projects[0].refresh_id.unwrap();
    let task = app.update(Message::Refreshed(refresh_id, Err("git exploded".into())));
    assert_eq!(task.units(), 0, "no PR lookups for a failed refresh");
    assert_eq!(
        app.selection.as_ref().unwrap().worktree.as_deref(),
        Some(linked.as_path())
    );
    assert!(matches!(app.console, ConsoleState::Connecting(4)));
    assert_eq!(app.selected_pull_request().unwrap().number, 7);
    assert_eq!(app.projects[0].worktrees().len(), 2);
    assert_eq!(
        app.projects[0].refresh_error.as_deref(),
        Some("git exploded")
    );

    // The next successful refresh clears the error.
    refresh_now(&mut app, id);
    assert!(app.projects[0].refresh_error.is_none());
    assert!(matches!(app.console, ConsoleState::Connecting(4)));
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

    if let Some(trees) = &mut app.projects[0].inventory {
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
#[test]
fn theme_edits_preview_live_and_cancel_restores_the_saved_theme() {
    use crate::theme::{Preset, Slot};
    let (mut app, _) = App::from_bookmarks(Err("test store".into()));
    let saved = app.palette();
    let _ = app.update(Message::OpenSettings);
    let _ = app.update(Message::Settings(settings::Message::Preset(Preset::Blade)));
    let _ = app.update(Message::Settings(settings::Message::Hex(
        Slot::Purple,
        "#00FF00".into(),
    )));
    // The whole app (window style and iced theme) draws with the draft.
    assert_eq!(app.palette().background, Preset::Blade.palette().background);
    assert_eq!(app.theme().palette().primary, app.palette().purple);
    assert_eq!(theme::to_hex(app.palette().purple), "#00FF00");

    let _ = app.update(Message::Settings(settings::Message::Cancel));
    assert!(app.settings.is_none());
    assert_eq!(app.palette(), saved);
}

#[test]
fn saved_theme_applies_and_survives_a_restart() {
    use crate::theme::Preset;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("theme.json");
    let (mut app, _) = App::from_bookmarks(Err("test store".into()));
    app.theme_store = Some(theme::Store::at(path.clone()));
    let _ = app.update(Message::OpenSettings);
    let _ = app.update(Message::Settings(settings::Message::Preset(
        Preset::Morbius,
    )));
    let _ = app.update(Message::Settings(settings::Message::Save));
    assert!(app.settings.is_none());
    assert_eq!(app.palette(), Preset::Morbius.palette());
    assert_eq!(
        theme::Store::at(path).load().unwrap(),
        Preset::Morbius.palette()
    );
}

#[test]
fn failed_theme_save_keeps_the_modal_open_and_shows_the_error() {
    use crate::theme::Preset;
    let (mut app, _) = App::from_bookmarks(Err("test store".into()));
    app.notice = None;
    let _ = app.update(Message::OpenSettings);
    let _ = app.update(Message::Settings(settings::Message::Preset(Preset::Buffy)));
    let _ = app.update(Message::Settings(settings::Message::Save));
    assert!(app.notice.as_deref().unwrap().contains("Theme settings"));
    assert_eq!(app.palette(), Preset::Buffy.palette());
    assert_eq!(app.palette, Palette::default());
}

/// Answers every PR lookup in flight for `id`, numbering PRs from `first`.
fn answer_pr_lookups(app: &mut App, id: ProjectId, first: u64) {
    let project = app.projects.iter().find(|p| p.id == id).unwrap();
    let pending: Vec<_> = project
        .pull_requests
        .iter()
        .filter_map(|(path, check)| Some((path.clone(), check.request_id?)))
        .collect();
    for (number, (path, request)) in (first..).zip(pending) {
        let pr = Some(PullRequest::for_test(number));
        let _ = app.update(Message::PullRequestFound(id, path, request, pr));
    }
}

/// Completes the refresh in flight for `id` with a fresh `git worktree list`.
fn finish_refresh(app: &mut App, id: ProjectId) -> Task<Message> {
    let project = app.projects.iter().find(|p| p.id == id).unwrap();
    let refresh_id = project.refresh_id.unwrap();
    let inventory = git::inventory(&project.checkout);
    app.update(Message::Refreshed(refresh_id, inventory))
}

#[test]
fn window_focus_refreshes_every_project_automatically() {
    let (tmp, mut app, _id, _root, _linked) = app_with_linked_checkout();
    let other = tmp.path().join("other repo");
    std::fs::create_dir(&other).unwrap();
    run_git(&other, &["init", "-q"]);
    app.bookmark(PickMode::Add, git::discover(&other).unwrap())
        .unwrap();

    assert_eq!(app.update(Message::WindowFocused).units(), 2);
    assert!(app.projects.iter().all(|p| p.refresh_id.is_some()));
    assert!(
        app.projects
            .iter()
            .all(|p| p.refresh_kind == RefreshKind::Automatic)
    );
    assert_eq!(
        app.update(Message::WindowFocused).units(),
        0,
        "refreshes already in flight are not repeated"
    );
}

#[test]
fn automatic_refresh_keeps_pr_badges_and_runs_gh_only_for_changed_branches() {
    let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
    refresh_now(&mut app, id);
    answer_pr_lookups(&mut app, id, 1);
    assert!(app.projects[0].pull_request(&root).is_some());
    assert!(app.projects[0].pull_request(&linked).is_some());

    // Nothing changed: no `gh`, and the badges stay.
    let _ = app.update(Message::WindowFocused);
    assert_eq!(finish_refresh(&mut app, id).units(), 0);
    assert!(app.projects[0].pull_request(&root).is_some());
    assert!(app.projects[0].pull_request(&linked).is_some());

    // The linked checkout switched branches outside the app: only it is
    // looked up again. (All lookups share one task, so check which are in flight.)
    let in_flight =
        |app: &App, path: &Path| app.projects[0].pull_requests[path].request_id.is_some();
    run_git(&linked, &["checkout", "-qb", "other"]);
    let _ = app.update(Message::WindowFocused);
    assert_eq!(finish_refresh(&mut app, id).units(), 1);
    assert!(app.projects[0].pull_request(&root).is_some());
    assert!(!in_flight(&app, &root));
    assert!(app.projects[0].pull_request(&linked).is_none());
    assert!(in_flight(&app, &linked));
    answer_pr_lookups(&mut app, id, 3);

    // Refresh pressed during an automatic refresh still re-checks every PR.
    let _ = app.update(Message::WindowFocused);
    let _ = app.update(Message::Refresh);
    assert_eq!(finish_refresh(&mut app, id).units(), 1);
    assert!(in_flight(&app, &root) && in_flight(&app, &linked));
}

#[test]
fn changed_git_metadata_starts_an_automatic_refresh() {
    let (_tmp, mut app, id, _root, _linked) = app_with_linked_checkout();
    assert_eq!(app.update(Message::WatchTick).units(), 1);
    assert_eq!(
        app.update(Message::WatchTick).units(),
        0,
        "one check at a time"
    );
    let request = app.watch_request.unwrap();
    let baseline = app.update(Message::Watched(request, vec![(id, 1)]));
    assert_eq!(baseline.units(), 0, "the first fingerprint is a baseline");

    let _ = app.update(Message::WatchTick);
    let request = app.watch_request.unwrap();
    let stale = app.update(Message::Watched(request - 1, vec![(id, 2)]));
    assert_eq!(stale.units(), 0, "a stale check is ignored");
    let unchanged = app.update(Message::Watched(request, vec![(id, 1)]));
    assert_eq!(unchanged.units(), 0);
    assert!(app.projects[0].refresh_id.is_none());

    let _ = app.update(Message::WatchTick);
    let request = app.watch_request.unwrap();
    let changed = app.update(Message::Watched(request, vec![(id, 2)]));
    assert_eq!(changed.units(), 1);
    assert!(app.projects[0].refresh_id.is_some());
    assert_eq!(app.projects[0].refresh_kind, RefreshKind::Automatic);
}
