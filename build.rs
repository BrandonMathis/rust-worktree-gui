//! Records which Git commit the app is built from, for the window title
//! (`src/version.rs`). Cargo runs this before compiling the crate, and the
//! `cargo:rustc-env` lines it prints become compile-time environment variables
//! that the app reads with `option_env!`.
//!
//! It only reports facts (commit, branch, tags on the commit); the app decides
//! what the title says, so that logic stays unit-testable. Without Git (a
//! source archive, or `git` not installed) it reports nothing, and the title
//! shows just the version.

use std::path::Path;
use std::process::Command;

/// Git's variables that point it at another repository (a subset of
/// `runner::GIT_OVERRIDES`). Cleared so a build started from a Git hook still
/// describes this checkout.
const GIT_OVERRIDES: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_CEILING_DIRECTORIES",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let Some(sha) = git(&["rev-parse", "--short=7", "HEAD"]) else {
        return;
    };
    println!("cargo:rustc-env=WORKTREE_BROWSER_GIT_SHA={sha}");
    // Empty on a detached HEAD (for example a CI checkout of a pull request).
    let branch = git(&["branch", "--show-current"]).unwrap_or_default();
    println!("cargo:rustc-env=WORKTREE_BROWSER_GIT_BRANCH={branch}");
    // Tag names can't contain whitespace, so one line of space-separated names is safe.
    let tags = git(&["tag", "--points-at", "HEAD"]).unwrap_or_default();
    let tags = tags.split_whitespace().collect::<Vec<_>>().join(" ");
    println!("cargo:rustc-env=WORKTREE_BROWSER_GIT_TAGS={tags}");

    // Rebuild when the commit, branch or tags change. In a linked worktree,
    // HEAD lives in the worktree's own Git dir and refs in the shared one.
    // Cargo scans a directory recursively for changes.
    let mut watched = Vec::new();
    if let Some(git_dir) = git(&["rev-parse", "--path-format=absolute", "--git-dir"]) {
        watched.push(Path::new(&git_dir).join("HEAD"));
    }
    if let Some(common_dir) = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]) {
        for refs in ["refs/heads", "refs/tags", "packed-refs"] {
            watched.push(Path::new(&common_dir).join(refs));
        }
    }
    for path in watched {
        // A path that doesn't exist would make Cargo rerun this on every build.
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

/// Runs `git` in the crate's directory and returns its trimmed output, or
/// `None` if Git is missing or the command fails.
fn git(args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.args(args).current_dir(env!("CARGO_MANIFEST_DIR"));
    for name in GIT_OVERRIDES {
        command.env_remove(name);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_owned())
}
