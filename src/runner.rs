//! One place to find and run external tools (`git`, `tmux`, `gh`, `open`, `node`).
//!
//! Every subprocess the app starts goes through [`command`] and [`run`], so they all:
//! - are found the same way ([`find`]): `PATH`, then the usual Homebrew and MacPorts
//!   folders, because a macOS app opened from Finder gets a minimal `PATH`;
//! - drop the Git variables that would point them at another repository
//!   ([`GIT_OVERRIDES`]);
//! - get no stdin, and are stopped after a per-call timeout, so a hung tool
//!   reports an error instead of blocking a request id forever.
//!
//! [`run`] blocks, so call it from `background(...)`, never from `update`.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Environment variables that make Git use a repository other than the one found
/// from the working directory. Git exports some of these to hooks, so launching the
/// app from a hook (or a shell inside one) would otherwise send every `git` call,
/// and every shell in the embedded terminal, to the hook's repository.
///
/// This is Git's own `local_repo_env` list (the variables Git clears itself when it
/// switches repositories, for example into a submodule), plus the discovery controls
/// `GIT_NAMESPACE`, `GIT_CEILING_DIRECTORIES` and `GIT_DISCOVERY_ACROSS_FILESYSTEM`.
/// See `git help git`, section "Environment Variables".
pub const GIT_OVERRIDES: [&str; 18] = [
    // Which repository, work tree and index to use.
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_PREFIX",
    // Where objects, grafts, replacements and shallow info come from.
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_GRAFT_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_SHALLOW_FILE",
    "GIT_NAMESPACE",
    // Configuration passed down from a parent `git -c …` call.
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    // How far repository discovery walks up from the working directory.
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
];

/// Folders searched after `PATH`, in order.
const FALLBACK_DIRS: [&str; 4] = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "/usr/bin",
];

/// An external program the app depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Git,
    Tmux,
    Gh,
    /// Runs the agent bridge for managed worktrees (`agent_bridge.rs`).
    Node,
    /// Installs the bridge's SDKs.
    Npm,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Tmux => "tmux",
            Self::Gh => "gh",
            Self::Node => "node",
            Self::Npm => "npm",
        }
    }

    /// An environment variable naming the exact executable to use instead of searching.
    fn override_var(self) -> Option<&'static str> {
        match self {
            Self::Tmux => Some("WORKTREE_BROWSER_TMUX"),
            Self::Node => Some("WORKTREE_BROWSER_NODE"),
            Self::Git | Self::Gh | Self::Npm => None,
        }
    }

    fn install_hint(self) -> &'static str {
        match self {
            Self::Git => "Install the Xcode Command Line Tools or `brew install git`.",
            Self::Tmux => "Install it with `brew install tmux`.",
            Self::Gh => "Install it with `brew install gh`.",
            Self::Node | Self::Npm => {
                "Managed worktrees need Node.js 22.19 or newer. Install it with `brew install node`."
            }
        }
    }
}

/// Finds `tool` using the real environment. See [`find_in`].
pub fn find(tool: Tool) -> Result<PathBuf, String> {
    let overridden = tool.override_var().and_then(std::env::var_os);
    find_in(tool, overridden, std::env::var_os("PATH"))
}

/// Finds `tool`: the override path if one is set (it must be an absolute path to an
/// executable), otherwise the first executable on `path_var` or in [`FALLBACK_DIRS`].
/// The environment is passed in so tests don't have to change the process environment.
fn find_in(
    tool: Tool,
    overridden: Option<OsString>,
    path_var: Option<OsString>,
) -> Result<PathBuf, String> {
    if let (Some(var), Some(path)) = (tool.override_var(), overridden) {
        let path = PathBuf::from(path);
        return if is_executable(&path) {
            Ok(path)
        } else {
            Err(format!(
                "{var} must be an absolute path to an executable {}. Could not use: {}",
                tool.name(),
                path.display()
            ))
        };
    }
    std::env::split_paths(&path_var.unwrap_or_default())
        .chain(FALLBACK_DIRS.map(PathBuf::from))
        .map(|dir| dir.join(tool.name()))
        .find(|path| is_executable(path))
        .ok_or_else(|| {
            let mut message = format!("{} not found. {}", tool.name(), tool.install_hint());
            if let Some(var) = tool.override_var() {
                message.push_str(&format!(" Or set {var} to its executable."));
            }
            message
        })
}

/// True for an absolute path to a file with an execute bit. Relative `PATH` entries
/// are skipped, so the current directory can never supply a tool.
fn is_executable(path: &Path) -> bool {
    path.is_absolute()
        && path
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// A `Command` for `program` with the Git overrides removed and stdin closed.
/// Add arguments, a working directory or extra variables, then pass it to [`run`].
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    strip_git_overrides(&mut command);
    command.stdin(Stdio::null());
    command
}

/// Removes [`GIT_OVERRIDES`] from what `command` inherits (and from anything set on it
/// earlier). Later `.env(...)` calls on the same command still apply.
fn strip_git_overrides(command: &mut Command) {
    for key in GIT_OVERRIDES {
        command.env_remove(key);
    }
}

/// Removes [`GIT_OVERRIDES`] from this process's own environment.
///
/// The embedded terminal's PTY runs the tmux client with the app's environment (the
/// terminal backend can add variables but not remove them), and a tmux server started
/// by that client hands its environment to every shell. Clearing the variables here,
/// once, keeps them out of those shells too.
///
/// # Safety
///
/// Changing the environment is only sound while no other thread can read it (Rust
/// 2024 marks `remove_var` `unsafe` for that reason). Call this first thing in `main`,
/// before any thread is spawned.
pub unsafe fn remove_git_overrides_from_process() {
    for key in GIT_OVERRIDES {
        // SAFETY: the caller guarantees the process is still single-threaded.
        unsafe { std::env::remove_var(key) };
    }
}

/// Why [`run`] produced no output.
#[derive(Debug)]
pub enum RunError {
    /// The program could not be started (missing, not executable, …).
    Spawn(std::io::Error),
    /// The program didn't exit in time and was killed.
    TimedOut(Duration),
    /// Waiting for the program or reading its output failed.
    Wait(std::io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "could not start: {error}"),
            Self::TimedOut(limit) => write!(
                f,
                "did not finish within {} and was stopped",
                describe(*limit)
            ),
            Self::Wait(error) => write!(f, "failed while running: {error}"),
        }
    }
}

fn describe(limit: Duration) -> String {
    if limit.as_secs() >= 1 {
        format!("{} seconds", limit.as_secs())
    } else {
        format!("{} ms", limit.as_millis())
    }
}

/// Runs `command` to completion, capturing stdout and stderr, like `Command::output`,
/// but kills the child and returns [`RunError::TimedOut`] if it takes longer than
/// `timeout`. A non-zero exit is still `Ok`: check `output.status`.
///
/// Only the child itself is killed. A grandchild that keeps the output pipes open
/// can't delay the result past the deadline, but it is left running.
pub fn run(command: &mut Command, timeout: Duration) -> Result<Output, RunError> {
    let deadline = Instant::now() + timeout;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Spawn)?;
    // Read both pipes on their own threads. A child that fills a pipe buffer blocks
    // until someone reads it, so waiting first and reading afterwards could deadlock.
    let stdout = read_in_background(child.stdout.take());
    let stderr = read_in_background(child.stderr.take());
    let status = wait_until(&mut child, deadline)?.ok_or(RunError::TimedOut(timeout))?;
    let collect = |pipe: mpsc::Receiver<std::io::Result<Vec<u8>>>| {
        match pipe.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(bytes) => bytes.map_err(RunError::Wait),
            // The child exited but something it started still holds the pipe open.
            Err(_) => Err(RunError::TimedOut(timeout)),
        }
    };
    Ok(Output {
        status,
        stdout: collect(stdout)?,
        stderr: collect(stderr)?,
    })
}

/// Polls the child until it exits (`Some(status)`) or `deadline` passes, in which
/// case it is killed (`None`). `std` has no blocking wait-with-timeout, and a 5 ms
/// poll is cheap next to starting a process.
fn wait_until(child: &mut Child, deadline: Instant) -> Result<Option<ExitStatus>, RunError> {
    loop {
        if let Some(status) = child.try_wait().map_err(RunError::Wait)? {
            return Ok(Some(status));
        }
        let now = Instant::now();
        if now >= deadline {
            // Ignore errors: the child may have exited between the two calls.
            let _ = child.kill();
            let _ = child.wait(); // Reap it so it does not linger as a zombie.
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(5).min(deadline - now));
    }
}

/// Reads a whole pipe on a new thread and sends the bytes back over a channel.
/// (A channel is used instead of joining the thread so the caller can give up on
/// it at the deadline.)
fn read_in_background(
    pipe: Option<impl Read + Send + 'static>,
) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = match pipe {
            Some(mut pipe) => pipe.read_to_end(&mut bytes).map(|_| bytes),
            None => Ok(bytes),
        };
        // The receiver is gone only if `run` already gave up; nothing to do then.
        let _ = sender.send(result);
    });
    receiver
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    /// Writes an executable shell script into `dir`.
    pub(crate) fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn subprocesses_never_inherit_git_repository_overrides() {
        // Set the overrides on the command itself to stand in for an environment
        // inherited from a Git hook, without changing this test process's environment.
        let mut env = Command::new("/usr/bin/env");
        for key in GIT_OVERRIDES {
            env.env(key, "/hook/repository");
        }
        env.env("GIT_TERMINAL_PROMPT", "0");
        strip_git_overrides(&mut env);
        let output = run(&mut env, Duration::from_secs(5)).unwrap();
        let listed = String::from_utf8(output.stdout).unwrap();
        for key in GIT_OVERRIDES {
            assert!(!listed.contains(&format!("{key}=")), "{key} leaked");
        }
        assert!(
            listed.contains("GIT_TERMINAL_PROMPT=0"),
            "other variables kept"
        );

        // `command` applies the same cleanup and closes stdin.
        let removed: Vec<_> = command("/usr/bin/true")
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_owned())
            .collect();
        assert_eq!(removed.len(), GIT_OVERRIDES.len());
        assert!(removed.iter().any(|key| key == "GIT_DIR"));
    }

    #[test]
    fn a_hung_tool_is_stopped_at_its_timeout_with_a_readable_error() {
        let dir = tempfile::tempdir().unwrap();
        // `exec` makes the shell itself become `sleep`, so the kill reaches it.
        let slow = script(dir.path(), "slow tool", "exec /bin/sleep 30");
        let started = Instant::now();
        let error = run(&mut command(&slow), Duration::from_millis(200)).unwrap_err();
        assert!(matches!(error, RunError::TimedOut(_)), "{error:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            error.to_string(),
            "did not finish within 200 ms and was stopped"
        );

        // A quick tool still returns its output and exit status.
        let quick = script(dir.path(), "quick", "printf out; printf err >&2; exit 3");
        let output = run(&mut command(&quick), Duration::from_secs(5)).unwrap();
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
        assert_eq!(output.status.code(), Some(3));
        assert!(matches!(
            run(
                &mut command(dir.path().join("missing")),
                Duration::from_secs(1)
            ),
            Err(RunError::Spawn(_))
        ));
    }

    #[test]
    fn tools_are_found_on_path_then_in_homebrew_folders_and_tmux_can_be_overridden() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin with spaces");
        std::fs::create_dir(&bin).unwrap();
        let gh = script(&bin, "gh", "exit 0");
        let path_var = std::env::join_paths([Path::new("relative"), &bin]).unwrap();
        assert_eq!(find_in(Tool::Gh, None, Some(path_var.clone())), Ok(gh));
        // A non-executable file on PATH is skipped.
        std::fs::write(bin.join("git"), "").unwrap();
        let git = find_in(Tool::Git, None, Some(path_var)).unwrap();
        assert!(
            FALLBACK_DIRS.iter().any(|dir| git.starts_with(dir)),
            "{git:?}"
        );
        // Git has no override variable, so a stray value is ignored.
        assert!(find_in(Tool::Git, Some(OsStr::from_bytes(b"/nope").into()), None).is_ok());
    }

    #[test]
    fn rejects_missing_or_relative_tmux_override() {
        let dir = tempfile::tempdir().unwrap();
        let tmux = script(dir.path(), "my tmux", "exit 0");
        assert_eq!(
            find_in(Tool::Tmux, Some(tmux.clone().into()), None),
            Ok(tmux)
        );
        let missing = dir.path().join("not-installed/tmux");
        let error = find_in(Tool::Tmux, Some(missing.into()), None).unwrap_err();
        assert!(error.contains("WORKTREE_BROWSER_TMUX"), "{error}");
        assert!(error.contains("not-installed/tmux"), "{error}");
        assert!(find_in(Tool::Tmux, Some("tmux".into()), None).is_err());
    }
}
