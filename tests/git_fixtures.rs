use crate::git;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn fixture() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("main checkout\n");
    std::fs::create_dir(&root).unwrap();
    run_git(&root, &["init", "-q", "-b", "main"]);
    run_git(&root, &["commit", "-qm", "initial", "--allow-empty"]);
    (tmp, root)
}

#[test]
fn checkout_subdirectory_and_linked_checkout_share_repository_identity() {
    let (_tmp, root) = fixture();
    let nested = root.join("nested");
    std::fs::create_dir(&nested).unwrap();
    run_git(
        &root,
        &["worktree", "add", "-qb", "topic", "../linked checkout"],
    );
    let linked = root.with_file_name("linked checkout");
    let main = git::discover(&root).unwrap();
    assert_eq!(main.root, root);
    assert_eq!(git::discover(&nested).unwrap(), main);
    let other = git::discover(&linked).unwrap();
    assert_eq!(other.root, linked);
    assert_eq!(main.common_dir, other.common_dir);
    assert_eq!(
        git::inventory(&main).unwrap(),
        git::inventory(&other).unwrap()
    );
}

#[test]
fn refresh_finds_external_additions_and_missing_directories() {
    let (_tmp, root) = fixture();
    let main = git::discover(&root).unwrap();
    assert_eq!(git::inventory(&main).unwrap().len(), 1);
    let relative = "../detached checkout\n";
    run_git(
        &root,
        &["worktree", "add", "-q", "--detach", relative, "HEAD"],
    );
    let detached = root.with_file_name("detached checkout\n");
    let trees = git::inventory(&main).unwrap();
    assert_eq!(trees.len(), 2);
    assert_eq!(trees[1].path, detached);
    assert_eq!(trees[1].label(), "Detached HEAD");
    assert!(trees[1].unavailable_reason().is_none());

    std::fs::remove_dir_all(&detached).unwrap();
    let trees = git::inventory(&main).unwrap();
    assert_eq!(trees.len(), 2);
    assert_eq!(trees[1].path, detached);
    assert!(trees[1].prunable.is_some());
    assert!(trees[1].unavailable_reason().is_some());

    run_git(&root, &["worktree", "prune"]);
    assert_eq!(git::inventory(&main).unwrap().len(), 1);
}

#[test]
fn missing_anchor_is_an_error_but_a_live_checkout_can_replace_it() {
    let (_tmp, root) = fixture();
    run_git(&root, &["worktree", "add", "-qb", "old", "../old checkout"]);
    let old = root.with_file_name("old checkout");
    let anchor = git::discover(&old).unwrap();
    run_git(&root, &["worktree", "remove", "../old checkout"]);
    assert!(
        git::inventory(&anchor)
            .unwrap_err()
            .contains("Checkout not found")
    );
    let live = git::discover(&root).unwrap();
    assert_eq!(live.common_dir, anchor.common_dir);
    assert_eq!(git::inventory(&live).unwrap().len(), 1);
}

#[test]
fn creating_a_project_initializes_git_without_overwriting_an_existing_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().canonicalize().unwrap();
    let folder = parent.join("New project");
    let checkout = git::new_project(&folder).unwrap();
    assert_eq!(checkout.root, folder);
    assert_eq!(git::discover(&folder).unwrap(), checkout);
    assert_eq!(git::inventory(&checkout).unwrap().len(), 1);
    assert!(git::new_project(&folder).is_err());
    let error =
        git::add_worktree(&checkout, &folder, "first", &parent.join("first"), "HEAD").unwrap_err();
    assert!(error.contains("make its first commit"));
    assert!(!parent.join("first").exists());
}

#[test]
fn creating_a_worktree_uses_the_requested_branch_base_and_destination() {
    let (_tmp, root) = fixture();
    let project = git::discover(&root).unwrap();
    let destination = root.with_file_name("Feature checkout");
    let created = git::add_worktree(&project, &root, "feature/new", &destination, "HEAD").unwrap();
    assert_eq!(created, destination);
    let trees = git::inventory(&project).unwrap();
    assert_eq!(trees[1].path, destination);
    assert_eq!(trees[1].branch.as_deref(), Some("feature/new"));
    assert_eq!(trees[0].head, trees[1].head);
    assert!(git::add_worktree(&project, &root, "another", &destination, "HEAD").is_err());
    assert!(
        git::add_worktree(&project, &root, "-bad", &root.with_file_name("bad"), "HEAD").is_err()
    );
    assert_eq!(git::inventory(&project).unwrap().len(), 2);
}

#[test]
fn a_folder_outside_git_is_a_readable_error() {
    let tmp = tempfile::tempdir().unwrap();
    let error = git::discover(tmp.path()).unwrap_err();
    assert!(
        error.contains("git rev-parse --show-toplevel failed"),
        "{error}"
    );
}

#[test]
fn projects_are_named_after_the_repository_not_the_anchor_checkout() {
    use std::ffi::OsStr;
    let (tmp, root) = fixture();
    run_git(
        &root,
        &[
            "worktree",
            "add",
            "-qb",
            "feature-x",
            "../feature-x checkout",
        ],
    );
    let main = git::discover(&root).unwrap();
    let linked = git::discover(&root.with_file_name("feature-x checkout")).unwrap();
    assert_eq!(main.repository_name(), OsStr::new("main checkout\n"));
    assert_eq!(linked.repository_name(), OsStr::new("main checkout\n"));

    // A bare repository: `shared repo.git` is named without its `.git` suffix.
    let parent = tmp.path().canonicalize().unwrap();
    let bare = parent.join("shared repo.git");
    run_git(
        &parent,
        &[
            "clone",
            "-q",
            "--bare",
            "main checkout\n",
            "shared repo.git",
        ],
    );
    run_git(
        &bare,
        &["worktree", "add", "-q", "../bare checkout", "main"],
    );
    let checkout = git::discover(&parent.join("bare checkout")).unwrap();
    assert_eq!(checkout.repository_name(), OsStr::new("shared repo"));

    // A bare repository hidden inside its project folder is named after the folder.
    let project = parent.join("my project");
    std::fs::create_dir(&project).unwrap();
    run_git(
        &project,
        &["clone", "-q", "--bare", "../main checkout\n", ".bare"],
    );
    run_git(
        &project.join(".bare"),
        &["worktree", "add", "-q", "../main", "main"],
    );
    let checkout = git::discover(&project.join("main")).unwrap();
    assert_eq!(checkout.repository_name(), OsStr::new("my project"));
}
