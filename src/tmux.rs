use iced_term::settings::BackendSettings;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::runner::{self, Tool};

const SOCKET: &str = "worktree-browser";
/// Limit for each short tmux query (`has-session`, `rename-session`, `display-message`).
const TIMEOUT: Duration = Duration::from_secs(5);

pub fn prepare(common_dir: &Path, worktree: &Path) -> Result<BackendSettings, String> {
    let worktree = canonical_worktree(worktree)?;
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

/// Switches the tmux client with process id `client_pid` (the one running
/// in the app's terminal) to `worktree`'s session, so the running terminal
/// is reused instead of starting a new client. A missing session is created
/// first, detached, in the worktree directory. Blocking: run it off the UI
/// thread. On any error the caller falls back to `prepare`.
pub fn switch_client(client_pid: u32, common_dir: &Path, worktree: &Path) -> Result<(), String> {
    let executable = find_tmux()?;
    switch_client_with(
        || tmux_command(&executable),
        client_pid,
        common_dir,
        worktree,
    )
}

fn switch_client_with(
    tmux: impl Fn() -> Command,
    client_pid: u32,
    common_dir: &Path,
    worktree: &Path,
) -> Result<(), String> {
    let worktree = canonical_worktree(worktree)?;
    let names = SessionNames::new(common_dir, &worktree);
    let name = names.migrate(&tmux).to_owned();
    if !has_session(tmux(), &name) {
        // Like the terminal in `prepare`, the first pane inherits this
        // command's working directory: no tmux format expansion of `-c`,
        // and non-UTF-8 paths survive.
        let mut create = tmux();
        create
            .current_dir(&worktree)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args(["new-session", "-d", "-s", &name]);
        run(&mut create, "create the tmux session")?;
    }
    let client = client_name(tmux(), client_pid)
        .ok_or("The terminal's tmux client is not attached to the server.")?;
    run(
        tmux().args(["switch-client", "-c", &client, "-t", &format!("={name}")]),
        "switch the tmux client",
    )
}

/// The name tmux uses for the client process `pid` (usually its tty).
fn client_name(mut tmux: Command, pid: u32) -> Option<String> {
    tmux.args(["list-clients", "-F", "#{client_pid}\t#{client_name}"]);
    let out = runner::run(&mut tmux, TIMEOUT).ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            let (client_pid, name) = line.split_once('\t')?;
            (client_pid.parse() == Ok(pid)).then(|| name.to_owned())
        })
}

fn canonical_worktree(worktree: &Path) -> Result<PathBuf, String> {
    let canonical = worktree.canonicalize().map_err(|error| {
        format!(
            "Could not open worktree {}: {error}. Refresh to check Git again.",
            worktree.display()
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "Worktree directory is unavailable: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn run(command: &mut Command, action: &str) -> Result<(), String> {
    let out = runner::run(command, TIMEOUT)
        .map_err(|error| format!("Could not {action}: tmux {error}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(format!("Could not {action}: {}", stderr.trim()))
    }
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
    let mut command = tmux_command(&executable);
    command.args([
        "display-message",
        "-p",
        "-t",
        &target,
        "#{pane_current_path}",
    ]);
    let out = runner::run(&mut command, TIMEOUT).ok()?;
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

/// What the app's tmux server reports about one session, for the sidebar's
/// activity badges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionActivity {
    /// When the session last had output, in seconds since the Unix epoch.
    pub last_activity: u64,
    /// A window rang the bell and nobody has looked at it since. tmux clears
    /// the flag when a client views that window.
    pub bell: bool,
}

/// One `list-windows -a` line per window: its session, when the window last
/// had output, and whether it rang the bell unseen. (`session_activity` only
/// moves on client input, so output is read per window.) Tabs can't occur in
/// `wt-` names.
const LIST_FORMAT: &str = "#{session_name}\t#{window_activity}\t#{window_bell_flag}";

/// Reads every session on the app's socket in one `tmux list-windows -a` call
/// and matches them to `worktrees` (`(key, common_dir, worktree)`) by the
/// session naming scheme. Read-only. A missing tmux or a server that isn't
/// running means no sessions, not an error. Blocking: run it off the UI thread.
pub fn session_activity<K>(worktrees: Vec<(K, PathBuf, PathBuf)>) -> Vec<(K, SessionActivity)> {
    let Ok(executable) = find_tmux() else {
        return Vec::new();
    };
    let sessions = list_sessions(tmux_command(&executable));
    match_sessions(&sessions, worktrees)
}

fn list_sessions(mut tmux: Command) -> BTreeMap<String, SessionActivity> {
    tmux.args(["list-windows", "-a", "-F", LIST_FORMAT]);
    match runner::run(&mut tmux, TIMEOUT) {
        // "no server running" exits non-zero: that just means no sessions.
        Ok(out) if out.status.success() => parse_sessions(&out.stdout),
        _ => BTreeMap::new(),
    }
}

/// Folds window lines into one entry per session: its latest output, and a
/// bell if any of its windows rang.
fn parse_sessions(stdout: &[u8]) -> BTreeMap<String, SessionActivity> {
    let mut sessions = BTreeMap::<String, SessionActivity>::new();
    for line in String::from_utf8_lossy(stdout).lines() {
        let mut fields = line.split('\t');
        let (Some(name), Some(Ok(last_activity)), Some(bell)) = (
            fields.next(),
            fields.next().map(str::parse::<u64>),
            fields.next(),
        ) else {
            continue;
        };
        let session = sessions.entry(name.to_owned()).or_insert(SessionActivity {
            last_activity,
            bell: false,
        });
        session.last_activity = session.last_activity.max(last_activity);
        session.bell |= bell == "1";
    }
    sessions
}

fn match_sessions<K>(
    sessions: &BTreeMap<String, SessionActivity>,
    worktrees: Vec<(K, PathBuf, PathBuf)>,
) -> Vec<(K, SessionActivity)> {
    if sessions.is_empty() {
        return Vec::new();
    }
    worktrees
        .into_iter()
        .filter_map(|(key, common_dir, worktree)| {
            // Names hash the canonical path, as `prepare` does.
            let names = SessionNames::new(&common_dir, &worktree.canonicalize().ok()?);
            let activity = sessions
                .get(names.short())
                .or_else(|| sessions.get(&names.legacy))?;
            Some((key, activity.clone()))
        })
        .collect()
}

/// The tmux client runs in the PTY with the app's environment plus `env` below. The
/// backend can only add variables, so the Git overrides are removed from the app's
/// own environment at startup instead (`runner::remove_git_overrides_from_process`).
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
    let mut command = runner::command(executable);
    command.args(["-L", SOCKET]);
    command
}

fn has_session(mut tmux: Command, name: &str) -> bool {
    succeeded(tmux.args(["has-session", "-t", &format!("={name}")]))
}

fn succeeded(command: &mut Command) -> bool {
    runner::run(command, TIMEOUT).is_ok_and(|output| output.status.success())
}

fn find_tmux() -> Result<PathBuf, String> {
    runner::find(Tool::Tmux)
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
        assert!(
            spec.env
                .keys()
                .all(|key| !runner::GIT_OVERRIDES.contains(&key.as_str()))
        );
    }

    /// A throwaway tmux server on its own socket, with no config file. It is
    /// killed on drop and never touches the user's `worktree-browser` server.
    pub(crate) struct PrivateServer {
        program: PathBuf,
        socket: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl PrivateServer {
        /// `None` (skip the test) when tmux isn't installed.
        pub(crate) fn start() -> Option<Self> {
            let program = find_tmux().ok()?;
            let dir = tempfile::tempdir().unwrap();
            Some(Self {
                program,
                socket: dir.path().join("sock"),
                _dir: dir,
            })
        }

        pub(crate) fn command(&self) -> Command {
            let mut command = Command::new(&self.program);
            command
                .arg("-S")
                .arg(&self.socket)
                .args(["-f", "/dev/null"]);
            command
        }

        /// Starts a detached session running `program` with `args`.
        pub(crate) fn new_session(&self, name: &str, program: &[&str]) {
            assert!(succeeded(
                self.command()
                    .args(["new-session", "-d", "-x", "80", "-y", "24", "-s", name])
                    .args(program)
            ));
        }
    }

    impl Drop for PrivateServer {
        fn drop(&mut self) {
            let _ = self.command().arg("kill-server").output();
        }
    }

    #[test]
    fn parses_window_output_and_bells_into_one_entry_per_session() {
        let parsed = parse_sessions(
            b"wt-0123456789\t1700000000\t0\n\
              wt-abcdefabcd\t1700000100\t0\n\
              wt-abcdefabcd\t1700000050\t1\n\
              not a session line\n\
              wt-bad\tnot-a-time\t0\n",
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed["wt-0123456789"],
            SessionActivity {
                last_activity: 1_700_000_000,
                bell: false
            }
        );
        // The latest window wins for activity; any window's bell counts.
        assert_eq!(
            parsed["wt-abcdefabcd"],
            SessionActivity {
                last_activity: 1_700_000_100,
                bell: true
            }
        );
    }

    #[test]
    fn reports_live_sessions_and_bells_by_worktree_from_a_private_server() {
        let Some(server) = PrivateServer::start() else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let (ringing, quiet, idle) = (
            tmp.path().join("ringing tree"),
            tmp.path().join("quiet tree"),
            tmp.path().join("no session"),
        );
        for dir in [&ringing, &quiet, &idle] {
            std::fs::create_dir(dir).unwrap();
        }
        let common_dir = PathBuf::from("/repo/.git");
        let name = |dir: &Path| {
            SessionNames::new(&common_dir, &dir.canonicalize().unwrap())
                .short()
                .to_owned()
        };
        server.new_session(
            &name(&ringing),
            &["/bin/sh", "-c", "printf '\\a'; exec sleep 60"],
        );
        server.new_session(&name(&quiet), &["/bin/sleep", "60"]);
        let worktrees = || {
            vec![
                ("ringing", common_dir.clone(), ringing.clone()),
                ("quiet", common_dir.clone(), quiet.clone()),
                ("idle", common_dir.clone(), idle.clone()),
            ]
        };

        // tmux flags the bell asynchronously.
        let mut found = Vec::new();
        for _ in 0..50 {
            found = match_sessions(&list_sessions(server.command()), worktrees());
            if found
                .iter()
                .any(|(key, activity)| *key == "ringing" && activity.bell)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let keys: Vec<_> = found.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, ["ringing", "quiet"], "no session, no badge");
        assert!(found[0].1.bell);
        assert!(!found[1].1.bell);
        assert!(found[1].1.last_activity > 0);

        // A server that isn't running is quiet: no sessions, no error.
        drop(server);
        assert!(list_sessions(Command::new("/nonexistent/tmux")).is_empty());
    }

    #[test]
    fn switches_the_running_client_to_another_worktree_session_without_a_new_client() {
        use crate::{terminal::Console, terminal_config::Preferences};
        let Some(server) = PrivateServer::start() else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let (first, second) = (
            tmp.path().join("first tree"),
            tmp.path().join("second tree"),
        );
        for dir in [&first, &second] {
            std::fs::create_dir(dir).unwrap();
        }
        let common_dir = PathBuf::from("/repo/.git");
        let name = |dir: &Path| {
            SessionNames::new(&common_dir, &dir.canonicalize().unwrap())
                .short()
                .to_owned()
        };

        // The app's terminal: a tmux client attached to the first worktree's
        // session, in a real PTY, but on the private server.
        let mut backend = settings(
            server.program.to_str().unwrap().to_owned(),
            &name(&first),
            first.canonicalize().unwrap(),
        );
        let socket = server.socket.to_str().unwrap().to_owned();
        backend.args = ["-S", &socket, "-f", "/dev/null", "-u"]
            .into_iter()
            .map(str::to_owned)
            .chain(backend.args.into_iter().skip(2)) // Drop `-L worktree-browser`.
            .collect();
        backend.env.insert("SHELL".into(), "/bin/sh".into());
        let console = Console::open(1, first.clone(), backend, &Preferences::default(), 1.0)
            .expect("the terminal starts");
        let clients = || {
            let out = server
                .command()
                .args(["list-clients", "-F", "#{client_pid} #{session_name}"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        let wait_for = |expected: &str| {
            for _ in 0..100 {
                if clients() == expected {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert_eq!(clients(), expected);
        };
        let pid = console.client_pid();
        wait_for(&format!("{pid} {}", name(&first)));

        switch_client_with(|| server.command(), pid, &common_dir, &second).unwrap();
        wait_for(&format!("{pid} {}", name(&second)));
        assert!(has_session(server.command(), &name(&first)), "kept running");
        let out = server
            .command()
            .args(["display-message", "-p", "-t"])
            .arg(format!("={}:", name(&second)))
            .arg("#{pane_current_path}")
            .output()
            .unwrap();
        assert_eq!(
            Path::new(String::from_utf8_lossy(&out.stdout).trim()),
            second.canonicalize().unwrap(),
            "the new session starts in its worktree"
        );

        // Switching back reuses the existing session.
        switch_client_with(|| server.command(), pid, &common_dir, &first).unwrap();
        wait_for(&format!("{pid} {}", name(&first)));

        // A client that isn't attached fails, so the app falls back to a
        // fresh connection.
        assert!(switch_client_with(|| server.command(), u32::MAX, &common_dir, &second).is_err());
        drop(console);
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
}
