use iced_term::settings::BackendSettings;
use sha2::{Digest, Sha256};
use std::env;
use std::os::unix::{
    ffi::{OsStrExt, OsStringExt},
    fs::PermissionsExt,
};
use std::path::{Path, PathBuf};
use std::process::Command;

const SOCKET: &str = "worktree-browser";
const OVERRIDE: &str = "WORKTREE_BROWSER_TMUX";

pub fn prepare(common_dir: &Path, worktree: &Path) -> Result<BackendSettings, String> {
    let worktree = worktree.canonicalize().map_err(|error| {
        format!(
            "Could not open worktree {}: {error}. Refresh to check Git again.",
            worktree.display()
        )
    })?;
    if !worktree.is_dir() {
        return Err(format!(
            "Worktree directory is unavailable: {}",
            worktree.display()
        ));
    }
    let executable = find_tmux()?;
    let program = executable.to_str().ok_or_else(|| {
        format!(
            "The tmux executable path must be UTF-8: {}",
            executable.display()
        )
    })?;
    let names = SessionNames::new(common_dir, &worktree);
    let name = names.migrate(|| tmux_command(&executable));
    Ok(settings(program.to_owned(), name, worktree))
}

pub fn pane_directory(common_dir: &Path, worktree: &Path) -> Option<PathBuf> {
    let worktree = worktree.canonicalize().ok()?;
    let executable = find_tmux().ok()?;
    let names = SessionNames::new(common_dir, &worktree);
    let name = if has_session(tmux_command(&executable), &names.legacy) {
        &names.legacy
    } else {
        names.short()
    };
    let target = format!("={name}:");
    let out = tmux_command(&executable)
        .args([
            "display-message",
            "-p",
            "-t",
            &target,
            "#{pane_current_path}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut bytes = out.stdout;
    if bytes.pop() != Some(b'\n') {
        return None;
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    (path.is_absolute() && path.is_dir()).then_some(path)
}

fn settings(program: String, name: &str, worktree: PathBuf) -> BackendSettings {
    BackendSettings {
        program,
        // Keep a separate socket, but let tmux find its normal config files and apply
        // the user's options. -T advertises embedded terminal capabilities, not preferences.
        args: [
            "-L",
            SOCKET,
            "-u",
            "-T",
            "RGB,extkeys",
            "new-session",
            "-A",
            "-s",
            name,
        ]
        .map(str::to_owned)
        .into(),
        // Inherit the PTY's cwd when creating the first pane. This avoids tmux format expansion
        // in -c and preserves non-UTF-8 worktree paths without passing them through a shell.
        working_directory: Some(worktree),
        env: [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("SHELL", "/bin/zsh"),
            ("TMUX", ""),
            ("TMUX_PANE", ""),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .into(),
    }
}

struct SessionNames {
    legacy: String,
}

impl SessionNames {
    fn new(common_dir: &Path, worktree: &Path) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"worktree-browser.v1\0");
        hash.update(common_dir.as_os_str().as_bytes());
        hash.update(b"\0");
        hash.update(worktree.as_os_str().as_bytes());
        let digest = hash.finalize();
        Self {
            legacy: format!("wt-{digest:x}")[..35].to_owned(),
        }
    }

    fn short(&self) -> &str {
        &self.legacy[..13] // wt- followed by ten hex digits.
    }

    fn migrate(&self, tmux: impl Fn() -> Command) -> &str {
        // tmux refuses to rename a session onto an existing name. If that happens,
        // keep using the old name instead of attaching to the other session.
        let renamed = succeeded(tmux().args([
            "rename-session",
            "-t",
            &format!("={}", self.legacy),
            self.short(),
        ]));
        if !renamed && has_session(tmux(), &self.legacy) {
            &self.legacy
        } else {
            self.short()
        }
    }
}

fn tmux_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.args(["-L", SOCKET]);
    command
}

fn has_session(mut tmux: Command, name: &str) -> bool {
    succeeded(tmux.args(["has-session", "-t", &format!("={name}")]))
}

fn succeeded(command: &mut Command) -> bool {
    command.output().is_ok_and(|output| output.status.success())
}

fn find_tmux() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os(OVERRIDE) {
        return validate_override(PathBuf::from(path));
    }
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path)
        .map(|dir| dir.join("tmux"))
        .chain(["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux", "/opt/local/bin/tmux", "/usr/bin/tmux"].map(PathBuf::from))
        .find(|path| executable(path))
        .ok_or_else(|| format!("tmux not found. Install it with `brew install tmux`, or set {OVERRIDE} to its executable."))
}

fn validate_override(path: PathBuf) -> Result<PathBuf, String> {
    if executable(&path) {
        Ok(path)
    } else {
        Err(format!(
            "{OVERRIDE} must be an absolute path to an executable tmux. Could not use: {}",
            path.display()
        ))
    }
}

fn executable(path: &Path) -> bool {
    path.is_absolute()
        && path
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        git,
        git_fixtures::{fixture, run_git},
    };

    #[test]
    fn reuses_name_by_repository_and_worktree_not_branch_or_anchor() {
        let (_tmp, root) = fixture();
        run_git(
            &root,
            &["worktree", "add", "-qb", "topic", "../linked # checkout"],
        );
        let linked = root.with_file_name("linked # checkout");
        let main_anchor = git::discover(&root).unwrap();
        let linked_anchor = git::discover(&linked).unwrap();
        let names = SessionNames::new(&main_anchor.common_dir, &linked);
        let name = names.short();
        assert_eq!(
            name,
            SessionNames::new(&linked_anchor.common_dir, &linked).short()
        );
        assert_ne!(
            name,
            SessionNames::new(&main_anchor.common_dir, &root).short()
        );
        assert_ne!(
            name,
            SessionNames::new(Path::new("/another/.git"), &linked).short()
        );
        assert_eq!(name.len(), 13);
        assert_eq!(names.legacy.len(), 35);
        assert!(name[3..].bytes().all(|byte| byte.is_ascii_hexdigit()));
        run_git(&linked, &["branch", "-m", "renamed"]);
        assert_eq!(
            name,
            SessionNames::new(&git::discover(&linked).unwrap().common_dir, &linked).short()
        );
    }

    #[test]
    fn isolates_sessions_without_overriding_home_config_and_passes_a_literal_cwd() {
        let worktree = PathBuf::from("/a # directory\n");
        let spec = settings("/opt/bin/tmux".into(), "wt-abcdef1234", worktree.clone());
        assert_eq!(
            spec.args,
            [
                "-L",
                SOCKET,
                "-u",
                "-T",
                "RGB,extkeys",
                "new-session",
                "-A",
                "-s",
                "wt-abcdef1234"
            ]
        );
        assert_eq!(spec.env["SHELL"], "/bin/zsh");
        assert_eq!(spec.working_directory, Some(worktree));
        assert_eq!(spec.env["TMUX"], "");
    }

    #[test]
    fn migrates_tmux_session_in_place_and_preserves_a_conflicting_session() {
        let Ok(program) = find_tmux() else { return };
        let fixture = tempfile::tempdir().unwrap();
        let socket = fixture.path().join("tmux.sock");
        let tmux = || {
            let mut command = Command::new(&program);
            command.arg("-S").arg(&socket).args(["-f", "/dev/null"]);
            command
        };
        struct Server(Command);
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = self.0.arg("kill-server").output();
            }
        }
        let _server = Server(tmux());
        let names = SessionNames::new(Path::new("/repo/.git"), Path::new("/repo"));
        let start =
            |name| succeeded(tmux().args(["new-session", "-d", "-s", name, "/bin/sleep", "60"]));
        let identity = |name: &str| {
            let out = tmux()
                .args([
                    "display-message",
                    "-p",
                    "-t",
                    &format!("={name}:"),
                    "#{session_id} #{pane_pid}",
                ])
                .output()
                .unwrap();
            assert!(out.status.success());
            out.stdout
        };
        assert!(start(&names.legacy));
        let original = identity(&names.legacy);
        assert_eq!(names.migrate(tmux), names.short());
        assert_eq!(identity(names.short()), original);
        assert!(!has_session(tmux(), &names.legacy));

        assert!(start(&names.legacy));
        let conflicting = identity(&names.legacy);
        assert_eq!(names.migrate(tmux), &names.legacy);
        assert_eq!(identity(&names.legacy), conflicting);
        assert_eq!(identity(names.short()), original);
    }

    #[test]
    fn rejects_missing_or_relative_tmux_override() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = validate_override(tmp.path().join("not-installed/tmux")).unwrap_err();
        assert!(missing.contains(OVERRIDE));
        assert!(missing.contains("not-installed/tmux"));
        assert!(validate_override(PathBuf::from("tmux")).is_err());
    }
}
