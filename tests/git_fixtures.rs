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
