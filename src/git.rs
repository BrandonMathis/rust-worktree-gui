use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub root: PathBuf,
    pub common_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
    pub directory_exists: bool,
}

impl Worktree {
    pub fn label(&self) -> &str {
        if self.bare {
            "Bare repository"
        } else if self.detached {
            "Detached HEAD"
        } else {
            self.branch.as_deref().unwrap_or("Unknown branch")
        }
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        if self.bare {
            Some("Bare repository (no working directory)")
        } else if !self.directory_exists {
            Some("Worktree directory is unavailable")
        } else if self.prunable.is_some() {
            Some("Git marks this worktree prunable")
        } else {
            None
        }
    }
}

pub fn discover(picked: &Path) -> Result<Checkout, String> {
    let root = git_path(picked, &["rev-parse", "--show-toplevel"])?;
    let common_dir = git_path(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(Checkout { root, common_dir })
}

pub fn inventory(anchor: &Checkout) -> Result<Vec<Worktree>, String> {
    if !anchor.root.is_dir() {
        return Err(format!("Checkout not found: {}", anchor.root.display()));
    }
    if discover(&anchor.root)? != *anchor {
        return Err(format!(
            "{} no longer points to the saved checkout. Pick a live checkout to replace the anchor.",
            anchor.root.display()
        ));
    }

    let bytes = git(&anchor.root, &["worktree", "list", "--porcelain", "-z"])?;
    let mut worktrees = parse_worktrees(&bytes)?;
    for tree in &mut worktrees {
        tree.directory_exists = tree.path.is_dir();
    }
    Ok(worktrees)
}

fn git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(cwd).args(args);
    // If the app is run from a Git hook, use the chosen checkout, not the hook's repository.
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
    ] {
        command.env_remove(key);
    }
    let output = command
        .output()
        .map_err(|err| format!("Could not run Git: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed in {} ({}): {}",
            args.join(" "),
            cwd.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn git_path(cwd: &Path, args: &[&str]) -> Result<PathBuf, String> {
    let mut bytes = git(cwd, args)?;
    // Strip only Git's terminator: even a trailing newline can be part of the path.
    if bytes.pop() != Some(b'\n') || bytes.is_empty() {
        return Err(format!("Git did not return a path for {}", cwd.display()));
    }
    let path = PathBuf::from(OsString::from_vec(bytes));
    path.canonicalize()
        .map_err(|err| format!("Could not resolve {}: {err}", path.display()))
}

fn parse_worktrees(output: &[u8]) -> Result<Vec<Worktree>, String> {
    let invalid = || "Git returned malformed worktree inventory".to_owned();
    let mut trees = Vec::new();
    let mut current: Option<Worktree> = None;
    // Each attribute ends in NUL and each record ends in an additional NUL.
    for attribute in output
        .strip_suffix(b"\0")
        .ok_or_else(invalid)?
        .split(|&byte| byte == 0)
    {
        if attribute.is_empty() {
            trees.push(current.take().ok_or_else(invalid)?);
            continue;
        }
        let (key, value) = attribute
            .iter()
            .position(|&byte| byte == b' ')
            .map_or((attribute, &b""[..]), |i| {
                (&attribute[..i], &attribute[i + 1..])
            });
        if key == b"worktree" {
            let path = PathBuf::from(OsString::from_vec(value.to_vec()));
            if !path.is_absolute() || current.is_some() {
                return Err(invalid());
            }
            current = Some(Worktree {
                path,
                branch: None,
                head: None,
                detached: false,
                bare: false,
                locked: None,
                prunable: None,
                directory_exists: false,
            });
        } else {
            let tree = current.as_mut().ok_or_else(invalid)?;
            let label = |value: &[u8]| String::from_utf8_lossy(value).into_owned();
            match key {
                b"branch" => {
                    tree.branch = Some(label(value.strip_prefix(b"refs/heads/").unwrap_or(value)))
                }
                b"HEAD" => tree.head = Some(label(value)),
                b"detached" => tree.detached = true,
                b"bare" => tree.bare = true,
                b"locked" => tree.locked = Some(label(value)),
                b"prunable" => tree.prunable = Some(label(value)),
                _ => {} // Allow future porcelain attributes.
            }
        }
    }
    if current.is_some() {
        return Err(invalid());
    }
    Ok(trees)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn preserves_paths_and_parses_flags() {
        let trees = parse_worktrees(
            b"worktree /path with spaces\n\xff\0HEAD 0123456\0branch refs/heads/topic/sub\0locked\0future field\0\0\
              worktree /missing\0HEAD 7654321\0detached\0prunable gitdir file points\nto non-existent location\0\0\
              worktree /bare\0bare\0\0",
        ).unwrap();
        assert_eq!(
            trees[0].path.as_os_str().as_bytes(),
            b"/path with spaces\n\xff"
        );
        assert_eq!(trees[0].label(), "topic/sub");
        assert_eq!(trees[0].locked.as_deref(), Some(""));
        assert_eq!(trees[1].label(), "Detached HEAD");
        assert_eq!(trees[1].head.as_deref(), Some("7654321"));
        assert_eq!(
            trees[1].prunable.as_deref(),
            Some("gitdir file points\nto non-existent location")
        );
        assert_eq!(trees[2].label(), "Bare repository");
    }

    #[test]
    fn rejects_truncated_or_malformed_output() {
        for invalid in [
            &b""[..],
            b"worktree /path\0HEAD 0123456\0",
            b"worktree /path\0worktree /other\0\0",
            b"branch refs/heads/main\0\0",
            b"worktree relative\0\0",
        ] {
            assert!(parse_worktrees(invalid).is_err(), "{invalid:?}");
        }
    }
}
