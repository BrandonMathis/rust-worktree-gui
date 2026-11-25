use super::*;
use crate::git_fixtures::{fixture, run_git};

#[test]
fn only_completed_unhandled_file_drops_request_a_paste() {
    use iced::{event::Status, window};

    let id = window::Id::unique();
    let path = PathBuf::from("/tmp/diagram draft.png");
    let drop = iced::Event::Window(window::Event::FileDropped(path.clone()));
    assert!(matches!(
        window_events(drop.clone(), Status::Ignored, id),
        Some(Message::FileDropped(dropped)) if dropped == path
    ));
    assert!(window_events(drop, Status::Captured, id).is_none());
    assert!(
        window_events(
            iced::Event::Window(window::Event::FileHovered(path)),
            Status::Ignored,
            id,
        )
        .is_none()
    );
    assert!(
        window_events(
            iced::Event::Window(window::Event::FilesHoveredLeft),
            Status::Ignored,
            id,
        )
        .is_none()
    );
}

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
fn dragging_the_sidebar_edge_resizes_it_within_limits() {
    use sidebar::{
        SIDEBAR_DEFAULT_WIDTH, SIDEBAR_EDGE_OFFSET, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH,
    };
    let (mut app, _) = App::from_bookmarks(Err("test store".into()));
    assert_eq!(app.sidebar_width, SIDEBAR_DEFAULT_WIDTH);

    // Moving the cursor without pressing the handle changes nothing.
    let _ = app.update(Message::SidebarResized(300.0));
    assert_eq!(app.sidebar_width, SIDEBAR_DEFAULT_WIDTH);

    let _ = app.update(Message::SidebarResizeStarted);
    assert!(app.sidebar_resizing);
    let _ = app.update(Message::SidebarResized(300.0 + SIDEBAR_EDGE_OFFSET));
    assert_eq!(app.sidebar_width, 300.0);
    let _ = app.update(Message::SidebarResized(10_000.0));
    assert_eq!(app.sidebar_width, SIDEBAR_MAX_WIDTH);
    let _ = app.update(Message::SidebarResized(0.0));
    assert_eq!(app.sidebar_width, SIDEBAR_MIN_WIDTH);

    let _ = app.update(Message::SidebarResizeEnded);
    assert!(!app.sidebar_resizing);
    let _ = app.update(Message::SidebarResized(300.0));
    assert_eq!(app.sidebar_width, SIDEBAR_MIN_WIDTH);

    // The width survives hiding and showing the sidebar; double-click resets it.
    let _ = app.update(Message::ToggleSidebar);
    let _ = app.update(Message::SidebarResizeStarted);
    assert!(
        !app.sidebar_resizing,
        "a hidden sidebar has no handle to drag"
    );
    let _ = app.update(Message::ToggleSidebar);
    assert_eq!(app.sidebar_width, SIDEBAR_MIN_WIDTH);
    let _ = app.update(Message::SidebarResizeReset);
    assert_eq!(app.sidebar_width, SIDEBAR_DEFAULT_WIDTH);
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
fn reordering_projects_restores_order_without_switching_worktrees() {
    let (tmp, mut app, first, root, linked) = app_with_linked_checkout();
    let (_second_tmp, second_root) = fixture();
    let (_third_tmp, third_root) = fixture();
    let second = app
        .bookmark(PickMode::Add, git::discover(&second_root).unwrap())
        .unwrap();
    let third = app
        .bookmark(PickMode::Add, git::discover(&third_root).unwrap())
        .unwrap();
    app.select(first, Some(linked.clone()));
    app.projects[0].expanded = true;
    app.console = ConsoleState::Connecting(9);
    let selection = app.selection.clone().unwrap();
    app.rename_worktree(&selection, "Review").unwrap();

    for (source, before, expected) in [
        (first, None, [second, third, first]),
        (third, Some(second), [third, second, first]),
    ] {
        let task = app.update(Message::ReorderProject { source, before });
        assert_eq!(task.units(), 0, "reordering must not reconnect or refresh");
        assert_eq!(
            app.projects.iter().map(|p| p.id).collect::<Vec<_>>(),
            expected
        );
    }
    assert_eq!(app.selection.as_ref(), Some(&selection));
    assert!(matches!(app.console, ConsoleState::Connecting(9)));
    let moved = &app.projects[2];
    assert!(moved.expanded);
    assert_eq!(
        moved.worktree_name(moved.worktree(&linked).unwrap()),
        "Review"
    );

    let (restored, _) =
        App::from_bookmarks(Store::open_path(tmp.path().join("config/projects.json")));
    assert_eq!(
        restored
            .projects
            .iter()
            .map(|p| &p.checkout.root)
            .collect::<Vec<_>>(),
        [&third_root, &second_root, &root],
    );
    assert_eq!(restored.projects[2].worktree_names[&linked], "Review");
}

#[test]
fn a_failed_reorder_keeps_the_original_order_and_shows_the_error() {
    let (tmp, mut app, first, _, _) = app_with_linked_checkout();
    let (_other_tmp, root) = fixture();
    let other = app
        .bookmark(PickMode::Add, git::discover(&root).unwrap())
        .unwrap();
    // Put a directory at the file path so the atomic rename cannot succeed.
    let path = tmp.path().join("config/projects.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();

    let _ = app.update(Message::ReorderProject {
        source: first,
        before: Some(other),
    });
    assert!(
        app.notice.is_none(),
        "dropping back in the same slot must not save"
    );
    let _ = app.update(Message::ReorderProject {
        source: other,
        before: Some(first),
    });
    assert!(app.notice.as_ref().unwrap().contains("Could not save"));
    assert_eq!(
        app.projects.iter().map(|p| p.id).collect::<Vec<_>>(),
        [first, other]
    );
    assert!(path.is_dir());
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
    answer_pr_lookup(&mut app, id, Some(vec![PullRequest::for_test(2, "topic")]));
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
    answer_pr_lookup(&mut app, id, Some(vec![PullRequest::for_test(7, "topic")]));
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
fn refresh_is_usable_again_after_git_times_out() {
    let (_tmp, mut app, _id, _root, _linked) = app_with_linked_checkout();
    assert_eq!(app.update(Message::Refresh).units(), 1);
    assert_eq!(
        app.update(Message::Refresh).units(),
        0,
        "already refreshing"
    );
    // The shared runner kills a hung git and returns this error.
    let refresh_id = app.projects[0].refresh_id.unwrap();
    let timed_out = "git worktree list in /repo did not finish within 30 seconds and was stopped. Refresh to try again.";
    let _ = app.update(Message::Refreshed(refresh_id, Err(timed_out.into())));
    assert_eq!(app.projects[0].refresh_id, None);
    assert_eq!(app.update(Message::Refresh).units(), 1);
}

/// Answers the PR lookup in flight for `id` with `prs` (`None`: gh failed).
fn answer_pr_lookup(app: &mut App, id: ProjectId, prs: Option<Vec<PullRequest>>) {
    let request = app.pr_request_id;
    let _ = app.update(Message::PullRequestsListed(id, request, prs));
}

#[test]
fn canceling_remove_project_keeps_the_bookmark_and_terminal() {
    let (tmp, mut app, id, _root, linked) = app_with_linked_checkout();
    let config = tmp.path().join("config/projects.json");
    let original = Store::open_path(config.clone()).unwrap().1;
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    app.console = ConsoleState::Connecting(3);

    let request = Message::MenuAction(
        Selection {
            project: id,
            worktree: None,
        },
        MenuAction::RemoveProject,
    );
    assert!(app.update(request.clone()).units() > 0);
    assert_eq!(app.projects.len(), 1, "wait for confirmation");
    assert_eq!(Store::open_path(config.clone()).unwrap().1, original);
    assert_eq!(app.update(request.clone()).units(), 0, "only one sheet");

    let _ = app.update(Message::ConfirmationAnswered(
        Confirmation::RemoveProject(id),
        false,
    ));
    assert_eq!(app.projects.len(), 1);
    assert_eq!(Store::open_path(config).unwrap().1, original);
    assert_eq!(
        app.selection,
        Some(Selection {
            project: id,
            worktree: Some(linked),
        })
    );
    assert!(matches!(app.console, ConsoleState::Connecting(3)));
    assert!(
        app.update(request).units() > 0,
        "can ask again after canceling"
    );
}

#[test]
fn confirming_removal_only_unbookmarks_the_project_and_detaches_its_terminal() {
    let (tmp, mut app, id, root, linked) = app_with_linked_checkout();
    let checkout = git::discover(&root).unwrap();
    let config = tmp.path().join("config/projects.json");
    let other = tmp.path().join("other repo");
    std::fs::create_dir(&other).unwrap();
    run_git(&other, &["init", "-q"]);
    let next = app
        .bookmark(PickMode::Add, git::discover(&other).unwrap())
        .unwrap();
    let _ = app.update(Message::SelectWorktree(id, linked));
    answer_pr_lookup(&mut app, id, Some(vec![PullRequest::for_test(5, "topic")]));
    app.console = ConsoleState::Connecting(3);
    assert!(app.selected_pull_request().is_some());

    let request = app.update(Message::MenuAction(
        Selection {
            project: id,
            worktree: None,
        },
        MenuAction::RemoveProject,
    ));
    assert!(request.units() > 0);
    let _ = app.update(Message::ConfirmationAnswered(
        Confirmation::RemoveProject(id),
        true,
    ));
    assert_eq!(app.projects.len(), 1);
    assert_eq!(
        Store::open_path(config).unwrap().1,
        [app.projects[0].bookmark()]
    );
    assert_eq!(git::inventory(&checkout).unwrap().len(), 2);
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
fn refresh_keeps_pr_badges_until_the_new_lookup_answers() {
    let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
    refresh_now(&mut app, id);
    assert!(
        app.projects[0].pull_requests.in_flight(),
        "one lookup per project"
    );
    let first = app.pr_request_id;
    answer_pr_lookup(
        &mut app,
        id,
        Some(vec![
            PullRequest::for_test(9, "topic"),
            PullRequest::for_test(3, "main"),
        ]),
    );
    assert_eq!(app.projects[0].pull_request(&linked).unwrap().number, 9);
    assert_eq!(app.projects[0].pull_request(&root).unwrap().number, 3);

    // A refresh keeps the badges while the next lookup runs, so they don't flicker,
    // and a late answer to the previous lookup is ignored.
    refresh_now(&mut app, id);
    assert!(app.pr_request_id > first);
    assert_eq!(app.projects[0].pull_request(&linked).unwrap().number, 9);
    let stale = vec![PullRequest::for_test(8, "topic")];
    let _ = app.update(Message::PullRequestsListed(id, first, Some(stale)));
    assert_eq!(app.projects[0].pull_request(&linked).unwrap().number, 9);

    // A worktree that changes branch loses its badge at once; the others keep theirs.
    run_git(&linked, &["checkout", "-qb", "other"]);
    refresh_now(&mut app, id);
    assert!(app.projects[0].pull_request(&linked).is_none());
    assert_eq!(app.projects[0].pull_request(&root).unwrap().number, 3);
    answer_pr_lookup(
        &mut app,
        id,
        Some(vec![
            PullRequest::for_test(9, "topic"),
            PullRequest::for_test(4, "other"),
            PullRequest::for_test(3, "main"),
        ]),
    );
    assert_eq!(app.projects[0].pull_request(&linked).unwrap().number, 4);

    // So does a worktree that disappears from Git.
    run_git(
        &root,
        &["worktree", "remove", "--force", "../linked checkout"],
    );
    refresh_now(&mut app, id);
    assert!(app.projects[0].pull_request(&linked).is_none());
    assert_eq!(app.projects[0].pull_request(&root).unwrap().number, 3);
}

#[test]
fn clicking_the_selected_worktree_again_does_not_look_up_its_pr_again() {
    let (_tmp, mut app, id, _root, linked) = app_with_linked_checkout();
    assert_eq!(
        app.update(Message::SelectWorktree(id, linked.clone()))
            .units(),
        2,
        "tmux connection and PR lookup"
    );
    answer_pr_lookup(&mut app, id, Some(vec![PullRequest::for_test(7, "topic")]));
    let request = app.pr_request_id;
    assert_eq!(app.update(Message::SelectWorktree(id, linked)).units(), 0);
    assert_eq!(app.pr_request_id, request);
    assert_eq!(app.selected_pull_request().unwrap().number, 7);
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
    answer_pr_lookup(
        &mut app,
        id,
        Some(vec![
            PullRequest::for_test(3, "main"),
            PullRequest::for_test(4, "topic"),
        ]),
    );
    assert_eq!(icon(&app, &root), WorktreeIcon::Main);
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
    // Selecting another worktree while the project's lookup runs doesn't start another.
    let _ = app.update(Message::SelectWorktree(id, root.clone()));
    assert_eq!(app.pr_request_id, request);
    answer_pr_lookup(&mut app, id, Some(vec![PullRequest::for_test(12, "topic")]));
    assert_eq!(app.projects[0].pull_request(&linked).unwrap().number, 12);
    assert!(app.selected_pull_request().is_none());

    // Replace Anchor drops the badges and ignores the old anchor's lookup.
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    let stale = app.pr_request_id;
    assert!(stale > request);
    app.bookmark(PickMode::Replace(id), git::discover(&linked).unwrap())
        .unwrap();
    assert!(app.selected_pull_request().is_none());
    refresh_now(&mut app, id);
    let fresh = vec![PullRequest::for_test(15, "topic")];
    let _ = app.update(Message::PullRequestsListed(id, stale, Some(fresh)));
    assert!(app.selected_pull_request().is_none());
    answer_pr_lookup(&mut app, id, Some(vec![PullRequest::for_test(16, "topic")]));
    assert_eq!(app.selected_pull_request().unwrap().number, 16);

    // No usable answer from gh (missing, logged out, timed out) hides the badges.
    let _ = app.update(Message::SelectWorktree(id, root));
    answer_pr_lookup(&mut app, id, None);
    assert!(app.projects[0].pull_request(&linked).is_none());
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
fn row_menu_offers_the_worktree_and_project_actions() {
    use DirectoryAction::*;
    assert_eq!(
        menu_actions(MenuTarget::Worktree { managed: false }),
        [
            MenuAction::Directory(Finder),
            MenuAction::Directory(VsCode),
            MenuAction::Directory(CopyPath),
            MenuAction::RenameWorktree,
            MenuAction::UseAgent,
        ]
    );
    assert_eq!(
        menu_actions(MenuTarget::Project { unavailable: 0 }),
        [
            MenuAction::NewWorktree,
            MenuAction::Directory(Finder),
            MenuAction::Directory(VsCode),
            MenuAction::Directory(CopyPath),
            MenuAction::ReplaceAnchor,
            MenuAction::RemoveProject,
        ]
    );
    assert_eq!(MenuAction::RenameWorktree.to_string(), "Rename Worktree");
    assert_eq!(MenuAction::RemoveProject.to_string(), "Remove Project");
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
    let (restarted, _) = App::from_bookmarks(Store::open_path(config));
    assert_eq!(restarted.projects[0].checkout, main);
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

/// Answers the PR lookup in flight for `id` with one open PR per worktree
/// branch, numbered from `first`.
fn answer_pr_lookups(app: &mut App, id: ProjectId, first: u64) {
    let project = app.projects.iter().find(|p| p.id == id).unwrap();
    let prs = (first..)
        .zip(project.worktrees())
        .filter_map(|(number, tree)| Some(PullRequest::for_test(number, tree.branch.as_deref()?)))
        .collect();
    answer_pr_lookup(app, id, Some(prs));
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

    // The linked checkout switched branches outside the app: its stale badge
    // goes at once, the other stays, and the project is listed again.
    run_git(&linked, &["checkout", "-qb", "other"]);
    let _ = app.update(Message::WindowFocused);
    assert_eq!(finish_refresh(&mut app, id).units(), 1);
    assert!(app.projects[0].pull_request(&root).is_some());
    assert!(app.projects[0].pull_request(&linked).is_none());
    answer_pr_lookups(&mut app, id, 3);
    assert_eq!(app.projects[0].pull_request(&linked).unwrap().number, 4);

    // A missing gh isn't retried by automatic refreshes...
    let _ = app.update(Message::Refresh);
    assert_eq!(
        finish_refresh(&mut app, id).units(),
        1,
        "manual Refresh lists again"
    );
    answer_pr_lookup(&mut app, id, None);
    let _ = app.update(Message::WindowFocused);
    assert_eq!(finish_refresh(&mut app, id).units(), 0);

    // ...but Refresh pressed during an automatic refresh still lists PRs again.
    let _ = app.update(Message::WindowFocused);
    let _ = app.update(Message::Refresh);
    assert_eq!(finish_refresh(&mut app, id).units(), 1);
    assert!(app.projects[0].pull_requests.in_flight());
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

#[test]
fn session_badges_show_live_output_and_bells_and_clear_when_selected() {
    use crate::tmux::SessionActivity;
    use sessions::SessionBadge;
    let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
    let report = |last_activity, bell| {
        vec![(
            (id, linked.clone()),
            SessionActivity {
                last_activity,
                bell,
            },
        )]
    };
    let badge = |app: &App, path: &Path| app.projects[0].session_badge(path);

    assert_eq!(app.update(Message::SessionPollTick).units(), 1);
    assert_eq!(
        app.update(Message::SessionPollTick).units(),
        0,
        "one poll at a time"
    );
    let request = app.session_request.unwrap();
    app.sessions_polled(request, report(100, false));
    assert_eq!(badge(&app, &linked), Some(SessionBadge::Live), "a baseline");
    assert_eq!(badge(&app, &root), None, "no session, no badge");

    let poll = |app: &mut App, reports| {
        let _ = app.update(Message::SessionPollTick);
        let request = app.session_request.unwrap();
        let _ = app.update(Message::SessionsPolled(request, reports));
    };
    poll(&mut app, report(200, false));
    assert_eq!(badge(&app, &linked), Some(SessionBadge::Activity));
    poll(&mut app, report(200, true));
    assert_eq!(badge(&app, &linked), Some(SessionBadge::Bell));

    let _ = app.update(Message::SessionPollTick);
    let request = app.session_request.unwrap();
    let _ = app.update(Message::SessionsPolled(request - 1, report(300, false)));
    assert_eq!(badge(&app, &linked), Some(SessionBadge::Bell), "stale poll");
    let _ = app.update(Message::SessionsPolled(request, report(200, true)));

    // Opening the worktree clears its unseen output and bell.
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    assert_eq!(badge(&app, &linked), Some(SessionBadge::Live));
    poll(&mut app, report(200, false));
    assert_eq!(badge(&app, &linked), Some(SessionBadge::Live));

    // The session ended (or the server stopped): the badge goes away quietly.
    poll(&mut app, Vec::new());
    assert_eq!(badge(&app, &linked), None);
    assert!(app.notice.is_none());
}

/// Attaches a real terminal (running a shell that just waits, not tmux) to
/// the selected worktree, as a finished connection would.
fn attach_terminal(app: &mut App, worktree: &Path) {
    let ConsoleState::Connecting(id) = app.console else {
        panic!("expected a connection in progress");
    };
    let shell = iced_term::settings::BackendSettings {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 30".into()],
        working_directory: Some(worktree.to_path_buf()),
        env: Default::default(),
    };
    let _ = app.update(Message::Prepared(id, Ok(shell)));
    assert!(matches!(app.console, ConsoleState::Attached(_)));
}

fn switch_id(app: &App) -> u64 {
    match app.console {
        ConsoleState::Switching { id, .. } => id,
        _ => panic!("expected a switch in progress"),
    }
}

#[test]
fn switching_worktrees_reuses_the_running_terminal_client() {
    let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
    let _ = app.update(Message::SelectWorktree(id, root.clone()));
    answer_pr_lookups(&mut app, id, 1);
    attach_terminal(&mut app, &root);
    let terminal = app.console.console().unwrap().id();

    // Another worktree: the same terminal's client switches sessions.
    let task = app.update(Message::SelectWorktree(id, linked.clone()));
    assert_eq!(task.units(), 2, "tmux switch-client and PR lookup");
    let switch = switch_id(&app);
    assert_eq!(app.console.console().unwrap().id(), terminal);
    assert_eq!(app.update(Message::Reconnect).units(), 0, "one at a time");
    let _ = app.update(Message::Switched(switch - 1, Ok(())));
    assert_eq!(switch_id(&app), switch, "a stale result is ignored");
    let _ = app.update(Message::Switched(switch, Ok(())));
    let ConsoleState::Attached(console) = &app.console else {
        panic!("expected the terminal to stay attached");
    };
    assert_eq!(
        (console.id(), console.worktree()),
        (terminal, linked.as_path())
    );

    // The user moves on while a switch runs: the next switch follows.
    let _ = app.update(Message::SelectWorktree(id, root.clone()));
    let to_root = switch_id(&app);
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    assert_eq!(switch_id(&app), to_root, "no second switch-client at once");
    assert_eq!(app.update(Message::Switched(to_root, Ok(()))).units(), 1);
    let to_linked = switch_id(&app);
    assert_ne!(to_linked, to_root);

    // A failed switch falls back to a fresh connection.
    let task = app.update(Message::Switched(to_linked, Err("no client".into())));
    assert_eq!(task.units(), 1, "tmux prepare");
    assert!(matches!(app.console, ConsoleState::Connecting(_)));
    assert!(app.console.console().is_none(), "the old terminal is gone");

    // Detach, and the project overview, still detach.
    attach_terminal(&mut app, &linked);
    let _ = app.update(Message::SelectWorktree(id, root.clone()));
    let _ = app.update(Message::Detach);
    assert!(matches!(app.console, ConsoleState::Detached));
    let _ = app.update(Message::Reconnect);
    attach_terminal(&mut app, &root);
    app.select(id, None);
    assert!(matches!(app.console, ConsoleState::Detached));
}

#[test]
fn selecting_an_unavailable_worktree_detaches_instead_of_switching() {
    let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
    std::fs::rename(&linked, root.with_file_name("moved")).unwrap();
    refresh_now(&mut app, id);
    let _ = app.update(Message::SelectWorktree(id, root.clone()));
    attach_terminal(&mut app, &root);

    let task = app.update(Message::SelectWorktree(id, linked));
    assert_eq!(task.units(), 0);
    assert!(matches!(app.console, ConsoleState::Detached));
}

/// Adds `count` linked worktrees (copies of `template` at new paths) to the
/// first project's inventory and returns their paths.
fn add_worktrees(app: &mut App, template: &Path, count: usize, available: bool) -> Vec<PathBuf> {
    let project = &mut app.projects[0];
    let template = project.worktree(template).unwrap().clone();
    let start = project.worktrees().len();
    let trees: Vec<Worktree> = (start..start + count)
        .map(|n| Worktree {
            path: template.path.with_file_name(format!("extra {n}")),
            directory_exists: available,
            ..template.clone()
        })
        .collect();
    let paths = trees.iter().map(|tree| tree.path.clone()).collect();
    project.inventory.as_mut().unwrap().extend(trees);
    paths
}

/// The sidebar's rows for the first project, and how many "Show N more" hides.
fn sidebar_listing(app: &App, selected: &Path) -> (Vec<PathBuf>, usize) {
    let listing = app.projects[0].sidebar_rows(Some(selected));
    let paths = listing.rows.iter().map(|tree| tree.path.clone()).collect();
    (paths, listing.hidden)
}

#[test]
fn long_worktree_lists_show_ten_until_the_user_shows_them_all() {
    let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
    // Thirteen worktrees: the main checkout plus twelve linked ones.
    let extra = add_worktrees(&mut app, &linked, 11, true);
    let last = extra[10].clone();

    let (paths, hidden) = sidebar_listing(&app, &root);
    assert_eq!((paths.len(), hidden), (10, 3));
    assert_eq!(paths[0], root);
    assert!(app.projects[0].sidebar_rows(Some(&root)).collapsible);

    // The selected worktree stays listed even past the limit.
    let (paths, hidden) = sidebar_listing(&app, &last);
    assert_eq!((paths.len(), hidden), (11, 2));
    assert_eq!(paths.last(), Some(&last));

    let _ = app.update(Message::ShowAllWorktrees(id, true));
    assert_eq!(
        sidebar_listing(&app, &root),
        (
            app.projects[0]
                .worktrees()
                .iter()
                .map(|t| t.path.clone())
                .collect(),
            0
        )
    );

    let _ = app.update(Message::ShowAllWorktrees(id, false));
    assert_eq!(sidebar_listing(&app, &root).1, 3);
}

#[test]
fn unavailable_worktrees_leave_the_sidebar_for_a_list_in_the_project_menu() {
    let (_tmp, mut app, id, root, linked) = app_with_linked_checkout();
    let missing = add_worktrees(&mut app, &linked, 3, false);
    let project = Selection {
        project: id,
        worktree: None,
    };

    // Only available worktrees are listed, and they don't count toward the limit.
    assert_eq!(
        sidebar_listing(&app, &root),
        (vec![root.clone(), linked.clone()], 0)
    );
    assert!(!app.projects[0].sidebar_rows(Some(&root)).collapsible);
    // A selected worktree stays listed even when it becomes unavailable (WT-06).
    assert!(sidebar_listing(&app, &missing[1]).0.contains(&missing[1]));

    // The project menu counts them; worktree menus don't offer the item.
    assert!(
        menu_actions(MenuTarget::Project { unavailable: 3 })
            .contains(&MenuAction::UnavailableWorktrees(3))
    );
    assert!(
        !menu_actions(MenuTarget::Project { unavailable: 0 })
            .iter()
            .any(|a| matches!(a, MenuAction::UnavailableWorktrees(_)))
    );
    assert_eq!(
        MenuAction::UnavailableWorktrees(3).to_string(),
        "3 Unavailable Worktrees…"
    );
    assert_eq!(
        MenuAction::UnavailableWorktrees(1).to_string(),
        "1 Unavailable Worktree…"
    );

    // Choosing it opens the modal with all of them, without touching the selection.
    let _ = app.update(Message::SelectWorktree(id, root.clone()));
    let selection = app.selection.clone();
    let _ = app.update(Message::MenuAction(
        project.clone(),
        MenuAction::UnavailableWorktrees(3),
    ));
    assert_eq!(app.unavailable_list, Some(id));
    assert_eq!(app.selection, selection);
    let listed: Vec<PathBuf> = app.projects[0]
        .unavailable_worktrees()
        .iter()
        .map(|tree| tree.path.clone())
        .collect();
    assert_eq!(listed, missing);

    let _ = app.update(Message::CloseUnavailableWorktrees);
    assert_eq!(app.unavailable_list, None);

    // Removing the project closes a modal left open for it.
    let _ = app.update(Message::MenuAction(
        project.clone(),
        MenuAction::UnavailableWorktrees(3),
    ));
    let _ = app.update(Message::MenuAction(project, MenuAction::RemoveProject));
    let _ = app.update(Message::ConfirmationAnswered(
        Confirmation::RemoveProject(id),
        true,
    ));
    assert_eq!(app.unavailable_list, None);
}

// --- Managed worktrees -------------------------------------------------------

use crate::agent::{Command, Event, Harness, Item, Model, Models, ToolStatus, TurnStatus};
use crate::agent_bridge::{Bridge, Prepared, Update};
use iced::futures::channel::mpsc::UnboundedReceiver;
use managed::{AgentBadge, BridgeState, ManagedMessage, ModelList, Run, ThinkingChoice};

fn models(default: &str, entries: &[(&str, &[&str], Option<&str>)]) -> Models {
    Models {
        default_model: Some(default.into()),
        models: entries
            .iter()
            .map(|(id, thinking, default_thinking)| Model {
                id: (*id).into(),
                name: id.to_uppercase(),
                description: String::new(),
                thinking: thinking.iter().map(|level| (*level).into()).collect(),
                default_thinking: default_thinking.map(Into::into),
            })
            .collect(),
    }
}

/// The linked checkout of `app_with_linked_checkout`, marked managed and
/// selected, with the bridge installed and Claude's models loaded.
fn app_with_managed_worktree() -> (tempfile::TempDir, App, ProjectId, PathBuf, PathBuf) {
    let (tmp, mut app, id, root, linked) = app_with_linked_checkout();
    app.set_managed(id, &linked, true).unwrap();
    app.managed.bridge = BridgeState::Ready(Bridge::fake());
    app.managed.models.insert(
        Harness::Claude,
        ModelList::Ready(models(
            "sonnet",
            &[
                ("opus", &["low", "high"], None),
                ("sonnet", &["low", "high"], None),
            ],
        )),
    );
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    (tmp, app, id, root, linked)
}

fn managed(message: ManagedMessage) -> Message {
    Message::Managed(message)
}

/// Types `text` into the selected session's prompt editor.
fn type_prompt(app: &mut App, text: &str) {
    use iced::widget::text_editor::{Action, Edit};
    let paste = Action::Edit(Edit::Paste(std::sync::Arc::new(text.to_owned())));
    let _ = app.update(managed(ManagedMessage::Edit(paste)));
}

fn submit(app: &mut App, text: &str) -> Task<Message> {
    type_prompt(app, text);
    app.update(managed(ManagedMessage::Submit))
}

/// The commands sent to the bridge since the last call.
fn sent(commands: &mut UnboundedReceiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

fn prompt(text: &str) -> Command {
    Command::Prompt { text: text.into() }
}

fn turn_end(app: &mut App, path: &Path, status: TurnStatus) {
    let id = app.managed.sessions[path].id();
    let event = Event::TurnEnd {
        status,
        error: None,
    };
    let _ = app.update(managed(ManagedMessage::Bridge(
        path.to_path_buf(),
        id,
        Update::Event(event),
    )));
}

fn user_prompts(app: &App, path: &Path) -> Vec<String> {
    app.managed.sessions[path]
        .transcript
        .items()
        .iter()
        .filter_map(|item| match item {
            Item::User(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_new_managed_worktree_opens_the_agent_prompt_instead_of_a_terminal() {
    let (tmp, mut app, id, root, _linked) = app_with_linked_checkout();
    let config = tmp.path().join("config/projects.json");
    let _ = app.update(Message::NewWorktree(id));
    assert!(
        !app.creation.as_ref().unwrap().managed,
        "terminal by default"
    );
    let _ = app.update(Message::Creation(create::Message::Managed(true)));

    let checkout = app.projects[0].checkout.clone();
    let destination = root.with_file_name("agent tree");
    let created = git::add_worktree(&checkout, &root, "agent-work", &destination, "HEAD");
    let path = created.clone().unwrap();
    let _ = app.update(Message::WorktreeCreated(id, created));
    assert!(app.creation.is_none());
    assert!(
        Store::open_path(config.clone()).unwrap().1[0]
            .managed
            .contains(&path),
        "the choice is saved with the bookmark"
    );

    let refresh_id = app.projects[0].refresh_id.unwrap();
    let _ = app.update(Message::Refreshed(refresh_id, git::inventory(&checkout)));
    assert_eq!(
        app.selection.as_ref().unwrap().worktree.as_deref(),
        Some(path.as_path())
    );
    assert!(
        matches!(app.console, ConsoleState::Detached),
        "no tmux session is started"
    );
    let session = &app.managed.sessions[&path];
    assert!(matches!(session.run, Run::New));
    assert!(session.transcript.is_empty());
    assert!(matches!(app.managed.bridge, BridgeState::Checking));

    // After a restart the worktree is still managed.
    let (restarted, _) = App::from_bookmarks(Store::open_path(config));
    assert!(restarted.projects[0].is_managed(&path));
}

#[test]
fn a_worktree_switches_between_terminal_and_agent_without_stopping_either() {
    let (_tmp, mut app, id, _root, linked) = app_with_linked_checkout();
    let target = Selection {
        project: id,
        worktree: Some(linked.clone()),
    };
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    assert!(matches!(app.console, ConsoleState::Connecting(_)));

    let _ = app.update(Message::MenuAction(target.clone(), MenuAction::UseAgent));
    assert!(app.projects[0].is_managed(&linked));
    assert!(matches!(app.console, ConsoleState::Detached));
    assert!(app.managed.sessions.contains_key(&linked));
    assert_eq!(
        menu_actions(MenuTarget::Worktree { managed: true }).last(),
        Some(&MenuAction::UseTerminal)
    );
    // Clicking the managed worktree again never connects a terminal.
    let _ = app.update(Message::Reconnect);
    assert!(matches!(app.console, ConsoleState::Detached));

    let mut commands = app.managed.sessions.get_mut(&linked).unwrap().attach(true);
    let _ = app.update(Message::MenuAction(target, MenuAction::UseTerminal));
    assert!(!app.projects[0].is_managed(&linked));
    assert!(matches!(app.console, ConsoleState::Connecting(_)));
    // The agent's conversation is kept and keeps running.
    assert!(app.managed.sessions[&linked].working());
    assert!(sent(&mut commands).is_empty());
}

#[test]
fn agent_sdks_are_installed_only_when_asked_and_models_load_once_ready() {
    let (_tmp, mut app, id, _root, linked) = app_with_linked_checkout();
    app.set_managed(id, &linked, true).unwrap();
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    assert!(matches!(app.managed.bridge, BridgeState::Checking));

    let _ = app.update(managed(ManagedMessage::Prepared(Ok(
        Prepared::NeedsInstall,
    ))));
    assert!(matches!(app.managed.bridge, BridgeState::NeedsInstall));
    assert!(
        app.managed.models.is_empty(),
        "nothing runs before the install"
    );
    // A prompt can't be sent until the SDKs are there.
    assert_eq!(submit(&mut app, "hello").units(), 0);
    assert!(app.managed.sessions[&linked].transcript.is_empty());
    assert_eq!(app.managed.sessions[&linked].draft(), "hello");

    assert_eq!(app.update(managed(ManagedMessage::Install)).units(), 1);
    assert!(matches!(app.managed.bridge, BridgeState::Installing));
    assert_eq!(app.update(managed(ManagedMessage::Install)).units(), 0);
    let _ = app.update(managed(ManagedMessage::Installed(Err("offline".into()))));
    assert!(matches!(&app.managed.bridge, BridgeState::Failed(error) if error == "offline"));

    // "Try again" checks from the start.
    assert_eq!(app.update(managed(ManagedMessage::Install)).units(), 1);
    assert!(matches!(app.managed.bridge, BridgeState::Checking));
    let ready = Prepared::Ready(Bridge::fake());
    let load = app.update(managed(ManagedMessage::Prepared(Ok(ready))));
    assert_eq!(
        load.units(),
        1,
        "the selected harness's models are requested"
    );
    let Some(ModelList::Loading(request)) = app.managed.models.get(&Harness::Claude) else {
        panic!("Claude's models should be loading");
    };
    let request = *request;

    // An answer to an older request is ignored.
    let list = models("m", &[("m", &[], None)]);
    let _ = app.update(managed(ManagedMessage::ModelsListed(
        Harness::Claude,
        request + 1,
        Ok(list.clone()),
    )));
    assert!(matches!(
        app.managed.models[&Harness::Claude],
        ModelList::Loading(_)
    ));
    let _ = app.update(managed(ManagedMessage::ModelsListed(
        Harness::Claude,
        request,
        Ok(list),
    )));
    assert!(matches!(
        app.managed.models[&Harness::Claude],
        ModelList::Ready(_)
    ));
    // Loaded models are not requested again.
    assert_eq!(
        app.update(managed(ManagedMessage::Harness(Harness::Claude)))
            .units(),
        0
    );
}

#[test]
fn the_first_prompt_starts_the_chosen_harness_model_and_thinking_level() {
    let (_tmp, mut app, _id, _root, linked) = app_with_managed_worktree();
    // Defaults: the harness's default model, at its own thinking level.
    let claude = models("sonnet", &[("sonnet", &["low", "high"], None)]);
    assert_eq!(
        app.managed.sessions[&linked].start_command(Some(&claude)),
        Command::Start {
            harness: Harness::Claude,
            model: Some("sonnet".into()),
            thinking: None,
            thinking_summaries: true,
        }
    );

    // Picking another harness loads its models and forgets the Claude choices.
    let _ = app.update(managed(ManagedMessage::Model(claude.models[0].clone())));
    let load = app.update(managed(ManagedMessage::Harness(Harness::Codex)));
    assert_eq!(load.units(), 1);
    let Some(ModelList::Loading(request)) = app.managed.models.get(&Harness::Codex) else {
        panic!("Codex's models should be loading");
    };
    let codex = models(
        "gpt-a",
        &[
            ("gpt-a", &["low", "medium"], Some("medium")),
            ("gpt-b", &[], None),
        ],
    );
    let _ = app.update(managed(ManagedMessage::ModelsListed(
        Harness::Codex,
        *request,
        Ok(codex.clone()),
    )));
    assert_eq!(app.managed.sessions[&linked].model, None);

    let _ = app.update(managed(ManagedMessage::Thinking(ThinkingChoice::Level(
        "low".into(),
    ))));
    assert_eq!(
        app.managed.sessions[&linked].start_command(Some(&codex)),
        Command::Start {
            harness: Harness::Codex,
            model: Some("gpt-a".into()),
            thinking: Some("low".into()),
            thinking_summaries: true,
        }
    );
    // Thinking levels belong to a model, so changing the model resets the level.
    let _ = app.update(managed(ManagedMessage::Model(codex.models[1].clone())));
    assert_eq!(
        app.managed.sessions[&linked].start_command(Some(&codex)),
        Command::Start {
            harness: Harness::Codex,
            model: Some("gpt-b".into()),
            thinking: None,
            thinking_summaries: false,
        }
    );

    assert_eq!(
        submit(&mut app, "  \n ").units(),
        0,
        "an empty prompt is not sent"
    );
    let start = submit(&mut app, "Fix the flaky test");
    assert_eq!(start.units(), 1, "the bridge session starts");
    let session = &app.managed.sessions[&linked];
    assert!(session.working());
    assert_eq!(session.draft(), "", "the composer is cleared");
    assert_eq!(user_prompts(&app, &linked), ["Fix the flaky test"]);

    // The harness and model are fixed once the conversation has started.
    let _ = app.update(managed(ManagedMessage::Harness(Harness::Pi)));
    let _ = app.update(managed(ManagedMessage::Model(codex.models[0].clone())));
    let session = &app.managed.sessions[&linked];
    assert_eq!(session.harness, Harness::Codex);
    assert_eq!(session.model.as_deref(), Some("gpt-b"));
}

#[test]
fn follow_ups_wait_for_the_running_turn_and_go_in_one_at_a_time() {
    let (_tmp, mut app, _id, _root, linked) = app_with_managed_worktree();
    let mut commands = app.managed.sessions.get_mut(&linked).unwrap().attach(true);

    // Sent while the agent works: queued, in order, and not sent yet.
    for follow_up in ["second", "third"] {
        assert_eq!(submit(&mut app, follow_up).units(), 0);
    }
    assert_eq!(app.managed.sessions[&linked].queue, ["second", "third"]);
    assert!(sent(&mut commands).is_empty());
    assert!(user_prompts(&app, &linked).is_empty());

    // Each turn end releases exactly one, and the agent stays busy.
    turn_end(&mut app, &linked, TurnStatus::Done);
    assert_eq!(sent(&mut commands), [prompt("second")]);
    assert_eq!(app.managed.sessions[&linked].queue, ["third"]);
    assert!(app.managed.sessions[&linked].working());

    // A queued follow-up can be withdrawn before its turn.
    let _ = submit(&mut app, "fourth");
    let _ = app.update(managed(ManagedMessage::Unqueue(0)));
    assert_eq!(app.managed.sessions[&linked].queue, ["fourth"]);

    // An interrupted or failed turn is over too: the next one still goes in.
    turn_end(&mut app, &linked, TurnStatus::Interrupted);
    assert_eq!(sent(&mut commands), [prompt("fourth")]);
    turn_end(&mut app, &linked, TurnStatus::Done);
    assert!(sent(&mut commands).is_empty());
    assert!(!app.managed.sessions[&linked].working());
    assert_eq!(user_prompts(&app, &linked), ["second", "fourth"]);

    // With the agent idle, a prompt goes straight in.
    let _ = submit(&mut app, "fifth");
    assert_eq!(sent(&mut commands), [prompt("fifth")]);
    assert!(app.managed.sessions[&linked].working());
    assert!(app.managed.sessions[&linked].queue.is_empty());
}

#[test]
fn a_turn_that_ends_twice_releases_only_one_follow_up() {
    let (_tmp, mut app, _id, _root, linked) = app_with_managed_worktree();
    let mut commands = app.managed.sessions.get_mut(&linked).unwrap().attach(true);
    for follow_up in ["second", "third"] {
        let _ = submit(&mut app, follow_up);
    }
    let id = app.managed.sessions[&linked].id();
    let error = |message: &str| {
        let event = Event::Error {
            message: message.into(),
        };
        managed(ManagedMessage::Bridge(
            linked.clone(),
            id,
            Update::Event(event),
        ))
    };

    // The harness reports an error, then the failed turn it ended. Only the
    // first end counts; "third" waits for the turn "second" starts.
    let _ = app.update(error("stream closed"));
    assert_eq!(sent(&mut commands), [prompt("second")]);
    turn_end(&mut app, &linked, TurnStatus::Failed);
    assert_eq!(sent(&mut commands), [prompt("third")]);
    turn_end(&mut app, &linked, TurnStatus::Done);
    assert!(!app.managed.sessions[&linked].working());

    // With nothing running, a stray end or error starts nothing.
    turn_end(&mut app, &linked, TurnStatus::Failed);
    let _ = app.update(error("interrupt failed"));
    assert!(sent(&mut commands).is_empty());
    assert!(!app.managed.sessions[&linked].working());
    assert!(matches!(
        app.managed.sessions[&linked].transcript.items().last(),
        Some(Item::Notice(text)) if text == "interrupt failed"
    ));
}

#[test]
fn stop_interrupts_only_a_running_turn() {
    let (_tmp, mut app, _id, _root, linked) = app_with_managed_worktree();
    let mut commands = app.managed.sessions.get_mut(&linked).unwrap().attach(false);
    let _ = app.update(managed(ManagedMessage::Stop));
    assert!(sent(&mut commands).is_empty());

    let _ = submit(&mut app, "go");
    let _ = app.update(managed(ManagedMessage::Stop));
    assert_eq!(sent(&mut commands), [prompt("go"), Command::Interrupt]);
    // The turn ends when the harness says so, not when Stop is pressed.
    assert!(app.managed.sessions[&linked].working());
    turn_end(&mut app, &linked, TurnStatus::Interrupted);
    assert!(!app.managed.sessions[&linked].working());
    assert!(matches!(
        app.managed.sessions[&linked].transcript.items().last(),
        Some(Item::Notice(text)) if text == "Stopped."
    ));
}

#[test]
fn agent_activity_streams_into_collapsed_rows_that_expand_on_click() {
    let (_tmp, mut app, _id, _root, linked) = app_with_managed_worktree();
    let _commands = app.managed.sessions.get_mut(&linked).unwrap().attach(false);
    let _ = submit(&mut app, "go");
    let id = app.managed.sessions[&linked].id();
    for line in [
        r#"{"type":"thinking","id":"k","delta":"Looking around"}"#,
        r#"{"type":"tool","id":"t","name":"Read","input":{"file_path":"a.rs"},"status":"running"}"#,
        r#"{"type":"tool","id":"t","status":"done","output":"fn main() {}"}"#,
        r#"{"type":"text","id":"m","delta":"Done."}"#,
    ] {
        let update = Update::Event(Event::parse(line).unwrap());
        let _ = app.update(managed(ManagedMessage::Bridge(linked.clone(), id, update)));
    }
    let items = app.managed.sessions[&linked].transcript.items();
    assert!(matches!(&items[1], Item::Thinking { open: false, .. }));
    assert!(matches!(
        &items[2],
        Item::Tool {
            open: false,
            status: ToolStatus::Done,
            ..
        }
    ));
    assert!(matches!(&items[3], Item::Text { source, .. } if source == "Done."));

    let _ = app.update(managed(ManagedMessage::Toggle(2)));
    let items = app.managed.sessions[&linked].transcript.items();
    assert!(matches!(&items[2], Item::Tool { open: true, .. }));
    assert!(matches!(&items[1], Item::Thinking { open: false, .. }));
}

#[test]
fn a_new_chat_drops_the_conversation_and_ignores_its_late_events() {
    let (_tmp, mut app, _id, _root, linked) = app_with_managed_worktree();
    let opus = models("opus", &[("opus", &["high"], None)]).models[0].clone();
    let _ = app.update(managed(ManagedMessage::Model(opus)));
    let _commands = app.managed.sessions.get_mut(&linked).unwrap().attach(false);
    let _ = submit(&mut app, "go");
    let _ = submit(&mut app, "queued");
    let old = app.managed.sessions[&linked].id();

    let _ = app.update(managed(ManagedMessage::NewChat));
    let session = &app.managed.sessions[&linked];
    assert!(matches!(session.run, Run::New));
    assert!(session.transcript.is_empty());
    assert!(session.queue.is_empty());
    assert_eq!(
        session.model.as_deref(),
        Some("opus"),
        "the model stays picked"
    );
    assert_ne!(session.id(), old);

    // The old bridge may still report before it stops.
    let late = Update::Event(Event::Notice {
        text: "from the old chat".into(),
    });
    let _ = app.update(managed(ManagedMessage::Bridge(linked.clone(), old, late)));
    let _ = app.update(managed(ManagedMessage::Bridge(
        linked.clone(),
        old,
        Update::Exited("stopped".into()),
    )));
    let session = &app.managed.sessions[&linked];
    assert!(session.transcript.is_empty());
    assert!(matches!(session.run, Run::New));
}

#[test]
fn a_bridge_that_stops_ends_the_turn_and_returns_queued_follow_ups() {
    let (_tmp, mut app, _id, _root, linked) = app_with_managed_worktree();
    let _commands = app.managed.sessions.get_mut(&linked).unwrap().attach(true);
    let _ = submit(&mut app, "one");
    let _ = submit(&mut app, "two");
    type_prompt(&mut app, "half typed");
    let id = app.managed.sessions[&linked].id();
    let _ = app.update(managed(ManagedMessage::Bridge(
        linked.clone(),
        id,
        Update::Exited("Out of memory".into()),
    )));
    let session = &app.managed.sessions[&linked];
    assert!(matches!(session.run, Run::Ended));
    assert!(!session.working());
    assert!(matches!(
        session.transcript.items().last(),
        Some(Item::Notice(text)) if text == "Out of memory"
    ));
    // Nothing the user wrote is lost.
    assert!(session.queue.is_empty());
    assert_eq!(session.draft(), "one\n\ntwo\n\nhalf typed");

    // A bridge-level error ends the turn like a failed one, and the session lives on.
    let (_tmp, mut app, _id, _root, linked) = app_with_managed_worktree();
    let _commands = app.managed.sessions.get_mut(&linked).unwrap().attach(true);
    let id = app.managed.sessions[&linked].id();
    let error = Update::Event(Event::Error {
        message: "Pi has no model named x.".into(),
    });
    let _ = app.update(managed(ManagedMessage::Bridge(linked.clone(), id, error)));
    let session = &app.managed.sessions[&linked];
    assert!(!session.working());
    assert!(matches!(session.run, Run::Live { .. }));
    assert!(matches!(
        session.transcript.items().last(),
        Some(Item::Notice(text)) if text == "Pi has no model named x."
    ));
}

#[test]
fn an_agent_keeps_working_out_of_view_and_its_badge_reports_the_finished_turn() {
    let (_tmp, mut app, id, root, linked) = app_with_managed_worktree();
    let badge = |app: &App| app.agent_badge(&app.projects[0], &linked);
    assert_eq!(badge(&app), Some(AgentBadge::Idle));
    assert_eq!(app.agent_badge(&app.projects[0], &root), None);

    let mut commands = app.managed.sessions.get_mut(&linked).unwrap().attach(false);
    let _ = submit(&mut app, "long task");
    assert_eq!(badge(&app), Some(AgentBadge::Working));
    assert!(app.agents_working());

    // Looking at another worktree neither stops nor interrupts the agent.
    let _ = app.update(Message::SelectWorktree(id, root.clone()));
    assert!(app.managed.sessions[&linked].working());
    assert_eq!(sent(&mut commands), [prompt("long task")]);
    assert_eq!(badge(&app), Some(AgentBadge::Working));

    turn_end(&mut app, &linked, TurnStatus::Done);
    assert_eq!(badge(&app), Some(AgentBadge::Finished));
    assert!(!app.agents_working());

    // Opening the worktree marks the turn as seen.
    let _ = app.update(Message::SelectWorktree(id, linked.clone()));
    assert_eq!(badge(&app), Some(AgentBadge::Idle));

    // A turn that ends while the worktree is in view is never "unseen".
    let _ = submit(&mut app, "another");
    turn_end(&mut app, &linked, TurnStatus::Done);
    assert_eq!(badge(&app), Some(AgentBadge::Idle));
}

#[test]
fn an_unavailable_managed_worktree_starts_no_agent() {
    let (_tmp, mut app, id, _root, linked) = app_with_linked_checkout();
    app.set_managed(id, &linked, true).unwrap();
    std::fs::remove_dir_all(&linked).unwrap();
    refresh_now(&mut app, id);
    let task = app.update(Message::SelectWorktree(id, linked.clone()));
    assert_eq!(task.units(), 0);
    assert!(app.managed.sessions.is_empty());
    assert!(matches!(app.managed.bridge, BridgeState::Unknown));
    assert_eq!(submit(&mut app, "hello").units(), 0);
}

#[test]
fn removing_a_project_stops_the_agents_in_its_worktrees() {
    let (_tmp, mut app, id, _root, linked) = app_with_managed_worktree();
    let mut commands = app.managed.sessions.get_mut(&linked).unwrap().attach(true);
    assert!(app.project_agents_working(id));

    app.pending_confirmation = Some(Confirmation::RemoveProject(id));
    let _ = app.update(Message::ConfirmationAnswered(
        Confirmation::RemoveProject(id),
        true,
    ));
    assert!(app.projects.is_empty());
    // Nothing could show or stop the agent once its row is gone.
    assert!(!app.managed.sessions.contains_key(&linked));
    assert!(!app.agents_working());
    assert!(matches!(
        commands.try_recv(),
        Err(iced::futures::channel::mpsc::TryRecvError::Closed)
    ));
}

#[test]
fn quitting_ends_agent_conversations_so_their_bridges_stop() {
    let (_tmp, mut app, _id, _root, linked) = app_with_managed_worktree();
    let mut commands = app.managed.sessions.get_mut(&linked).unwrap().attach(true);
    app.pending_confirmation = Some(Confirmation::Quit);
    let quit = app.update(Message::ConfirmationAnswered(Confirmation::Quit, true));
    assert_eq!(quit.units(), 1);
    assert!(app.managed.sessions.is_empty());
    // The command channel is closed: that is what tells a bridge to stop.
    assert!(matches!(
        commands.try_recv(),
        Err(iced::futures::channel::mpsc::TryRecvError::Closed)
    ));
}
