//! Cheap change detection for automatic refresh.
//!
//! Instead of a file-system event watcher, the app polls a small set of Git
//! metadata files with `stat` every couple of seconds (off the UI thread) and
//! folds their modification times into a fingerprint. Only when a project's
//! fingerprint changes does it run `git worktree list` again. This catches
//! worktrees added, removed, moved or switched to another branch outside the
//! app (including from the embedded terminal), with no extra dependency and
//! no event storms while Git rewrites its files.
//!
//! The fingerprint is only compared within one run of the app, so the
//! standard library's hasher (not stable across releases) is fine.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A digest of the Git state that `git worktree list` reports.
pub type Fingerprint = u64;

/// Fingerprints the repository whose shared `.git` is `common_dir`. It reads
/// only Git's own files, never the app's state, so a refresh can't change it.
/// Missing files hash as missing, so this never fails.
pub fn fingerprint(common_dir: &Path) -> Fingerprint {
    let mut hash = DefaultHasher::new();
    // `HEAD` moves when the main checkout switches branches; `refs/heads` and
    // `packed-refs` when branches are created, renamed or committed to.
    for name in ["HEAD", "refs/heads", "packed-refs", "worktrees"] {
        modified(&common_dir.join(name)).hash(&mut hash);
    }
    // One admin directory per linked worktree: added, removed, moved
    // (`gitdir`), switched (`HEAD`), locked or unlocked. `gitdir` holds the
    // path of the worktree's `.git` file: if it's gone, so is the directory,
    // and the worktree becomes unavailable.
    let mut linked: Vec<_> = std::fs::read_dir(common_dir.join("worktrees"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    linked.sort();
    for admin in &linked {
        admin.hash(&mut hash);
        for name in ["HEAD", "gitdir", "locked"] {
            modified(&admin.join(name)).hash(&mut hash);
        }
        linked_checkout(admin)
            .is_some_and(|git_file| git_file.exists())
            .hash(&mut hash);
    }
    hash.finish()
}

/// The `.git` file of the worktree that admin directory `admin` belongs to.
fn linked_checkout(admin: &Path) -> Option<PathBuf> {
    let mut bytes = std::fs::read(admin.join("gitdir")).ok()?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    // Paths are bytes on Unix and may not be UTF-8.
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

fn modified(path: &Path) -> Option<SystemTime> {
    path.metadata().and_then(|meta| meta.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git;
    use crate::git_fixtures::{fixture, run_git};

    #[test]
    fn fingerprint_changes_when_worktrees_or_branches_change_outside_the_app() {
        let (_tmp, root) = fixture();
        let common_dir = git::discover(&root).unwrap().common_dir;
        let linked = root.with_file_name("linked checkout");
        let before = fingerprint(&common_dir);
        assert_eq!(fingerprint(&common_dir), before, "stable when idle");

        // Modification times can be coarse; make sure each change is visible.
        let settle = || std::thread::sleep(std::time::Duration::from_millis(20));
        settle();
        run_git(
            &root,
            &["worktree", "add", "-qb", "topic", "../linked checkout"],
        );
        let added = fingerprint(&common_dir);
        assert_ne!(added, before, "a worktree was added");

        settle();
        run_git(&linked, &["checkout", "-qb", "other"]);
        let switched = fingerprint(&common_dir);
        assert_ne!(switched, added, "a worktree switched branches");

        std::fs::rename(&linked, root.with_file_name("moved")).unwrap();
        assert_ne!(
            fingerprint(&common_dir),
            switched,
            "a worktree directory disappeared"
        );
    }
}
