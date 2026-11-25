//! Runs the agent bridge: the Node.js sidecar that drives the Claude, Codex and
//! Pi SDKs for managed worktrees.
//!
//! None of the three harnesses has a Rust SDK, so `agent-bridge/bridge.mjs` uses
//! their official TypeScript SDKs and this module talks to it over stdin and
//! stdout, one JSON object per line (`agent::Command` in, `agent::Event` out).
//!
//! The bridge's source and its `package-lock.json` are compiled into the app
//! and copied to `~/Library/Application Support/Worktree Browser/agent-bridge`.
//! The SDKs are large, so they are installed there once, when the user asks
//! ([`install`] runs `npm ci` against the bundled lockfile), and shared by every
//! copy of the app.
//!
//! [`prepare`] and [`install`] block: call them through `background(...)`.
//! [`list_models`] and [`session`] are async and run on iced's Tokio runtime.

use crate::agent::{Command, Event, Harness, Models};
use crate::config;
use crate::runner::{self, Tool};
use iced::futures::channel::{mpsc, oneshot};
use iced::futures::future::{Either, select};
use iced::futures::{SinkExt, Stream, StreamExt};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;

/// The bridge's files, compiled in so the app needs no checkout at run time.
const FILES: [(&str, &str); 4] = [
    ("bridge.mjs", include_str!("../agent-bridge/bridge.mjs")),
    (
        "normalize.mjs",
        include_str!("../agent-bridge/normalize.mjs"),
    ),
    ("package.json", include_str!("../agent-bridge/package.json")),
    ("package-lock.json", LOCKFILE),
];
const LOCKFILE: &str = include_str!("../agent-bridge/package-lock.json");

/// Written after a successful install, holding the hash of the lockfile that
/// was installed. A newer app with a different lockfile asks to install again.
const STAMP: &str = "node_modules/.worktree-browser-lock";

/// A folder that already holds the bridge and its `node_modules`, used as it
/// is: nothing is written or installed there. For development and tests.
const DIR_OVERRIDE: &str = "WORKTREE_BROWSER_AGENT_BRIDGE";

/// The oldest Node.js the SDKs run on (Pi's requirement, the strictest).
const MIN_NODE: (u32, u32) = (22, 19);

const NODE_TIMEOUT: Duration = Duration::from_secs(20);
/// `npm ci` downloads several hundred megabytes of SDK binaries.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const MODELS_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a bridge gets to stop its agent after its stdin closes. Tests
/// use a short one so the suite stays fast.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(if cfg!(test) { 2000 } else { 5000 });

/// How much of the bridge's stderr is kept to explain a crash.
const STDERR_TAIL: usize = 1500;

/// An installed bridge, ready to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bridge {
    /// The real Node.js binary. A version manager's shim (asdf, mise) picks a
    /// version from the working directory, and sessions run inside worktrees.
    node: PathBuf,
    dir: PathBuf,
}

impl Bridge {
    /// Tests only: a bridge that is never run.
    #[cfg(test)]
    pub(crate) fn fake() -> Self {
        Self {
            node: PathBuf::from("/nonexistent/node"),
            dir: PathBuf::from("/nonexistent/agent-bridge"),
        }
    }
}

/// What [`prepare`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prepared {
    Ready(Bridge),
    /// The SDKs aren't installed (or are from another lockfile): run [`install`].
    NeedsInstall,
}

/// One item from a [`session`].
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    Event(Event),
    /// The bridge process ended, with the reason. Always the last item.
    Exited(String),
}

/// Finds Node.js, copies the bundled bridge into place, and reports whether
/// its SDKs are installed.
pub fn prepare() -> Result<Prepared, String> {
    let node = runner::find(Tool::Node)?;
    if let Some(dir) = std::env::var_os(DIR_OVERRIDE) {
        let dir = PathBuf::from(dir);
        let node = resolve_node(&node, &dir)?;
        return Ok(Prepared::Ready(Bridge { node, dir }));
    }
    let dir = config::support_dir()?.join("agent-bridge");
    write_files(&dir)?;
    let node = resolve_node(&node, &dir)?;
    Ok(if installed(&dir) {
        Prepared::Ready(Bridge { node, dir })
    } else {
        Prepared::NeedsInstall
    })
}

/// Installs the SDKs with `npm ci`, which installs exactly what the bundled
/// lockfile pins and verifies each package's checksum.
pub fn install() -> Result<Bridge, String> {
    let dir = config::support_dir()?.join("agent-bridge");
    write_files(&dir)?;
    let node = resolve_node(&runner::find(Tool::Node)?, &dir)?;
    // npm ships with Node.js, next to it; otherwise look it up like any tool.
    let npm = Some(node.with_file_name("npm"))
        .filter(|npm| npm.is_file())
        .map_or_else(|| runner::find(Tool::Npm), Ok)?;
    install_in(&dir, &node, &npm)?;
    Ok(Bridge { node, dir })
}

fn install_in(dir: &Path, node: &Path, npm: &Path) -> Result<(), String> {
    let mut command = runner::command(npm);
    command
        // No lifecycle scripts: installing must not run code from a package.
        .args([
            "ci",
            "--omit=dev",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ])
        .current_dir(dir)
        .env("PATH", path_with(node));
    let output = runner::run(&mut command, INSTALL_TIMEOUT)
        .map_err(|err| format!("Could not install the agent SDKs: npm {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not install the agent SDKs: {}",
            tail(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    std::fs::write(dir.join(STAMP), lock_hash())
        .map_err(|err| format!("Could not finish installing the agent SDKs: {err}"))
}

/// Copies the bundled files into `dir`, replacing any that differ.
fn write_files(dir: &Path) -> Result<(), String> {
    let error = |err: std::io::Error| {
        format!(
            "Could not set up the agent bridge in {}: {err}",
            dir.display()
        )
    };
    std::fs::create_dir_all(dir).map_err(error)?;
    for (name, contents) in FILES {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|current| current == contents) {
            continue;
        }
        std::fs::write(&path, contents).map_err(error)?;
    }
    Ok(())
}

fn installed(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(STAMP)).is_ok_and(|stamp| stamp == lock_hash())
}

fn lock_hash() -> String {
    Sha256::digest(LOCKFILE.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Asks `node` (possibly a version manager's shim) for the binary it really
/// runs, from `cwd`, and checks that it is new enough for the SDKs.
fn resolve_node(node: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let mut command = runner::command(node);
    command
        .args(["-p", "process.execPath + '\\n' + process.versions.node"])
        .current_dir(cwd);
    let output = runner::run(&mut command, NODE_TIMEOUT)
        .map_err(|err| format!("Could not run Node.js at {}: {err}", node.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let (Some(path), Some(version), true) = (lines.next(), lines.next(), output.status.success())
    else {
        // A version manager's shim explains itself in its first line or two
        // (asdf: "No version is set for command node"), then lists every
        // installed version.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason: Vec<_> = stderr.lines().map(str::trim).take(2).collect();
        return Err(format!(
            "Could not run Node.js at {}: {}. Set WORKTREE_BROWSER_NODE to a Node.js {}.{} or newer binary.",
            node.display(),
            reason.join(" ").trim_end_matches('.'),
            MIN_NODE.0,
            MIN_NODE.1
        ));
    };
    let mut parts = version.split('.').map(|part| part.parse::<u32>().ok());
    let found = (parts.next().flatten(), parts.next().flatten());
    if !matches!(found, (Some(major), Some(minor)) if (major, minor) >= MIN_NODE) {
        return Err(format!(
            "Managed worktrees need Node.js {}.{} or newer, but {path} is {version}. Update Node.js, or set WORKTREE_BROWSER_NODE to a newer one.",
            MIN_NODE.0, MIN_NODE.1
        ));
    }
    Ok(PathBuf::from(path))
}

/// `PATH` with Node's folder first, for tools whose launcher runs `env node`.
fn path_with(node: &Path) -> OsString {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let dirs = node
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .chain(std::env::split_paths(&current));
    std::env::join_paths(dirs).unwrap_or(current)
}

/// The end of a tool's error output, short enough for the notice bar.
fn tail(text: &str) -> String {
    let text = text.trim();
    let start = text
        .char_indices()
        .rev()
        .nth(STDERR_TAIL.saturating_sub(1))
        .map_or(0, |(index, _)| index);
    text[start..].to_owned()
}

/// A running bridge process with its pipes.
struct Process {
    // Dropping the child kills it (`kill_on_drop`), so it never outlives the
    // session that owns it.
    child: Child,
    stdin: ChildStdin,
    lines: tokio::io::Lines<BufReader<ChildStdout>>,
    /// Collects the end of stderr while the process runs. A pipe nobody reads
    /// fills up and blocks the writer, so it has to be drained as we go.
    stderr: JoinHandle<String>,
}

impl Bridge {
    /// Starts the bridge in `cwd`. `writable` are extra folders the agent may
    /// write to. Paths are passed as the working directory and as arguments,
    /// never through JSON, so they need not be UTF-8.
    fn spawn(&self, cwd: &Path, writable: &[PathBuf]) -> Result<Process, String> {
        let error =
            |err: &dyn std::fmt::Display| format!("Could not start the agent bridge: {err}");
        let mut command = runner::command(&self.node);
        command
            .arg(self.dir.join("bridge.mjs"))
            .args(writable)
            .current_dir(cwd)
            .env("PATH", path_with(&self.node));
        // `tokio::process::Command` wraps the std one and adds async pipes.
        let mut command = tokio::process::Command::from(command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| error(&err))?;
        let (Some(stdin), Some(stdout), Some(mut stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(error(&"its pipes are unavailable"));
        };
        let stderr = tokio::spawn(async move {
            let mut kept = Vec::new();
            let mut chunk = [0; 4096];
            while let Ok(read) = stderr.read(&mut chunk).await {
                if read == 0 {
                    break;
                }
                kept.extend_from_slice(&chunk[..read]);
                if kept.len() > 4 * STDERR_TAIL {
                    kept.drain(..kept.len() - 2 * STDERR_TAIL);
                }
            }
            tail(&String::from_utf8_lossy(&kept))
        });
        Ok(Process {
            child,
            stdin,
            lines: BufReader::new(stdout).lines(),
            stderr,
        })
    }
}

impl Process {
    /// Waits for the process to end and explains why, from its stderr.
    async fn last_words(mut self) -> String {
        drop(self.stdin);
        let _ = self.child.wait().await;
        match self.stderr.await {
            Ok(stderr) if !stderr.is_empty() => stderr,
            _ => "The agent bridge stopped unexpectedly.".into(),
        }
    }
}

async fn send(stdin: &mut ChildStdin, command: &Command) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(command)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await
}

/// Asks `harness` which models the signed-in user can use.
pub async fn list_models(bridge: Bridge, harness: Harness) -> Result<Models, String> {
    let ask = async {
        // Run outside any repository so no project settings change the list.
        let cwd = directories::BaseDirs::new()
            .map_or_else(|| bridge.dir.clone(), |dirs| dirs.home_dir().to_path_buf());
        let mut process = bridge.spawn(&cwd, &[])?;
        send(&mut process.stdin, &Command::Models { harness })
            .await
            .map_err(|err| format!("Could not ask {} for its models: {err}", harness.name()))?;
        while let Ok(Some(line)) = process.lines.next_line().await {
            match Event::parse(&line) {
                Some(Event::Models(models)) => return Ok(models),
                Some(Event::Error { message }) => return Err(message),
                _ => {}
            }
        }
        Err(process.last_words().await)
    };
    match tokio::time::timeout(MODELS_TIMEOUT, ask).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "{} did not list its models in time.",
            harness.name()
        )),
    }
    .map_err(|err| format!("Could not load {} models: {err}", harness.name()))
}

/// Aborts a spawned task when dropped. A plain `JoinHandle` detaches instead.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs one conversation in `worktree`: forwards `commands` to the bridge and
/// yields what it reports, ending with [`Update::Exited`].
///
/// The conversation ends when `commands` closes (its sender was dropped). The
/// bridge's stdin is then closed, which tells it to stop its agent and exit;
/// one that hasn't after [`SHUTDOWN_GRACE`] is killed. Dropping the stream
/// kills the bridge at once, which can leave the harness's own subprocess
/// running, so prefer dropping the sender. Nothing the agent wrote is touched.
pub fn session(
    bridge: Bridge,
    worktree: PathBuf,
    writable: Vec<PathBuf>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> impl Stream<Item = Update> {
    // `stream::channel` turns a future that pushes into a channel into a
    // stream, like an async generator with `yield`.
    iced::stream::channel(64, async move |mut output: mpsc::Sender<Update>| {
        let Process {
            mut child,
            mut stdin,
            mut lines,
            stderr,
        } = match bridge.spawn(&worktree, &writable) {
            Ok(process) => process,
            Err(error) => {
                let _ = output.send(Update::Exited(error)).await;
                return;
            }
        };
        // Write commands on their own task so a long turn's output and the
        // next command (an interrupt) never wait on each other. When the
        // commands close, so does stdin (the task drops it), and `closed`
        // tells the reader below to expect the bridge to exit.
        let (closing, mut closed) = oneshot::channel::<()>();
        let _writer = AbortOnDrop(tokio::spawn(async move {
            while let Some(command) = commands.next().await {
                if send(&mut stdin, &command).await.is_err() {
                    break;
                }
            }
            drop(stdin);
            let _ = closing.send(());
        }));
        // Set once the commands have closed: the time by which the bridge
        // must have exited.
        let mut deadline = None;
        loop {
            let next = match deadline {
                // `select` waits for whichever comes first: a line of output,
                // or the end of the commands. (`next_line` is safe to drop
                // midway.) The first `match` ends the borrow of `lines`.
                None => {
                    let step = match select(pin!(lines.next_line()), &mut closed).await {
                        Either::Left((line, _)) => Some(line),
                        Either::Right(_) => None,
                    };
                    match step {
                        Some(line) => line,
                        None => {
                            deadline = Some(tokio::time::Instant::now() + SHUTDOWN_GRACE);
                            continue;
                        }
                    }
                }
                // Shutting down: keep forwarding what it says, but not forever.
                Some(deadline) => {
                    match tokio::time::timeout_at(deadline, lines.next_line()).await {
                        Ok(line) => line,
                        Err(_) => {
                            let _ = child.kill().await;
                            break;
                        }
                    }
                }
            };
            let Ok(Some(line)) = next else {
                break; // The bridge closed its output.
            };
            if let Some(event) = Event::parse(&line)
                && output.send(Update::Event(event)).await.is_err()
            {
                return; // Nobody is listening: dropping `child` stops it.
            }
        }
        let _ = child.wait().await;
        let reason = match stderr.await {
            Ok(stderr) if !stderr.is_empty() => stderr,
            _ => "The agent bridge stopped unexpectedly.".into(),
        };
        let _ = output.send(Update::Exited(reason)).await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{ToolStatus, TurnStatus};
    use crate::runner::tests::script;

    /// A stand-in for `node bridge.mjs`: answers each command line like the
    /// bridge would, with some log noise in between.
    const FAKE_BRIDGE: &str = r#"
while IFS= read -r line; do
  case "$line" in
    *'"type":"models"'*)
      echo 'a dependency logged this'
      echo '{"type":"models","default_model":"m2","models":[{"id":"m1","name":"One"},{"id":"m2","name":"Two","thinking":["low"]}]}' ;;
    *'"type":"start"'*)
      printf '{"type":"notice","text":"%s|%s"}\n' "$(pwd -P)" "$2" ;;
    *'"type":"prompt"'*)
      echo '{"type":"tool","id":"t","name":"Read","input":{"path":"a"},"status":"running"}'
      echo '{"type":"text","id":"x","delta":"hi"}'
      echo '{"type":"turn_end","status":"done"}' ;;
    *'"type":"interrupt"'*)
      echo 'boom: out of memory' >&2
      exit 3 ;;
  esac
done
"#;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn fake_bridge(tmp: &Path, body: &str) -> Bridge {
        Bridge {
            node: script(tmp, "node", body),
            dir: tmp.to_path_buf(),
        }
    }

    #[test]
    fn a_session_relays_commands_and_events_until_its_commands_close() {
        let tmp = tempfile::tempdir().unwrap();
        let worktree = tmp.path().join("tree");
        std::fs::create_dir(&worktree).unwrap();
        let git_dir = tmp.path().join("repo.git");
        let bridge = fake_bridge(tmp.path(), FAKE_BRIDGE);
        let (commands, receiver) = mpsc::unbounded();
        for command in [
            Command::Start {
                harness: Harness::Pi,
                model: None,
                thinking: None,
                thinking_summaries: false,
            },
            Command::Prompt { text: "go".into() },
        ] {
            commands.unbounded_send(command).unwrap();
        }
        // Dropping the sender is how the app ends a session.
        drop(commands);
        let updates: Vec<_> = runtime()
            .block_on(session(bridge, worktree.clone(), vec![git_dir.clone()], receiver).collect());
        let started = format!(
            "{}|{}",
            worktree.canonicalize().unwrap().display(),
            git_dir.display()
        );
        assert_eq!(
            updates[..4],
            [
                Update::Event(Event::Notice { text: started }),
                Update::Event(Event::Tool {
                    id: "t".into(),
                    name: Some("Read".into()),
                    input: Some(serde_json::json!({"path": "a"})),
                    status: ToolStatus::Running,
                    output: None,
                }),
                Update::Event(Event::Text {
                    id: "x".into(),
                    delta: Some("hi".into()),
                    text: None,
                }),
                Update::Event(Event::TurnEnd {
                    status: TurnStatus::Done,
                    error: None,
                }),
            ]
        );
        assert!(matches!(updates.last(), Some(Update::Exited(_))));
        assert_eq!(updates.len(), 5);
    }

    #[test]
    fn a_bridge_that_dies_reports_its_last_words() {
        let tmp = tempfile::tempdir().unwrap();
        let bridge = fake_bridge(tmp.path(), FAKE_BRIDGE);
        let (commands, receiver) = mpsc::unbounded();
        commands.unbounded_send(Command::Interrupt).unwrap();
        let updates: Vec<_> = runtime()
            .block_on(session(bridge, tmp.path().to_path_buf(), Vec::new(), receiver).collect());
        assert_eq!(updates, [Update::Exited("boom: out of memory".into())]);

        let missing = Bridge {
            node: tmp.path().join("no-such-node"),
            dir: tmp.path().to_path_buf(),
        };
        let (_commands, receiver) = mpsc::unbounded();
        let updates: Vec<_> = runtime()
            .block_on(session(missing, tmp.path().to_path_buf(), Vec::new(), receiver).collect());
        assert!(matches!(
            updates.as_slice(),
            [Update::Exited(error)] if error.starts_with("Could not start the agent bridge")
        ));
    }

    #[test]
    fn a_bridge_that_ignores_the_end_of_its_session_is_stopped() {
        let tmp = tempfile::tempdir().unwrap();
        // Never reads stdin, so it can't notice that it was closed.
        let stubborn = fake_bridge(tmp.path(), "exec sleep 600");
        let (commands, receiver) = mpsc::unbounded::<Command>();
        drop(commands);
        let started = std::time::Instant::now();
        let updates: Vec<_> = runtime()
            .block_on(session(stubborn, tmp.path().to_path_buf(), Vec::new(), receiver).collect());
        assert!(matches!(updates.as_slice(), [Update::Exited(_)]));
        assert!(started.elapsed() < SHUTDOWN_GRACE + Duration::from_secs(5));
    }

    #[test]
    fn listing_models_returns_the_harness_answer_or_its_error() {
        let tmp = tempfile::tempdir().unwrap();
        let models = runtime()
            .block_on(list_models(
                fake_bridge(tmp.path(), FAKE_BRIDGE),
                Harness::Claude,
            ))
            .unwrap();
        assert_eq!(models.preferred().unwrap().name, "Two");
        assert_eq!(models.models.len(), 2);

        let failing = fake_bridge(
            tmp.path(),
            r#"read -r line; echo '{"type":"error","message":"Not signed in"}'"#,
        );
        assert_eq!(
            runtime()
                .block_on(list_models(failing, Harness::Codex))
                .unwrap_err(),
            "Could not load Codex models: Not signed in"
        );

        let crashing = fake_bridge(tmp.path(), "echo 'Cannot find module' >&2; exit 1");
        assert_eq!(
            runtime()
                .block_on(list_models(crashing, Harness::Pi))
                .unwrap_err(),
            "Could not load Pi models: Cannot find module"
        );
    }

    #[test]
    fn the_sdks_count_as_installed_only_for_the_bundled_lockfile() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agent-bridge");
        write_files(&dir).unwrap();
        for (name, contents) in FILES {
            assert_eq!(std::fs::read_to_string(dir.join(name)).unwrap(), contents);
        }
        assert!(!installed(&dir));

        // A stand-in npm: records how it was called and "installs" a package.
        let npm = script(
            tmp.path(),
            "npm",
            r#"echo "$*" > npm-args; mkdir -p node_modules/sdk"#,
        );
        install_in(&dir, Path::new("/usr/bin/true"), &npm).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("npm-args"))
                .unwrap()
                .trim(),
            "ci --omit=dev --ignore-scripts --no-audit --no-fund"
        );
        assert!(installed(&dir));

        // An install from an older app's lockfile must be redone.
        std::fs::write(dir.join(STAMP), "another lockfile").unwrap();
        assert!(!installed(&dir));

        let failing = script(
            tmp.path(),
            "npm-fails",
            "echo 'ENOTFOUND registry' >&2; exit 1",
        );
        assert_eq!(
            install_in(&dir, Path::new("/usr/bin/true"), &failing).unwrap_err(),
            "Could not install the agent SDKs: ENOTFOUND registry"
        );
        assert!(!installed(&dir));
    }

    #[test]
    fn a_node_too_old_for_the_sdks_is_refused_with_what_was_found() {
        let tmp = tempfile::tempdir().unwrap();
        let current = script(tmp.path(), "node", "echo /opt/node/bin/node; echo 24.4.1");
        assert_eq!(
            resolve_node(&current, tmp.path()).unwrap(),
            Path::new("/opt/node/bin/node")
        );
        let old = script(tmp.path(), "old-node", "echo /old/node; echo 22.18.0");
        let error = resolve_node(&old, tmp.path()).unwrap_err();
        assert!(error.contains("22.19 or newer"), "{error}");
        assert!(error.contains("/old/node is 22.18.0"), "{error}");
        // A shim's long list of installed versions is left out of the notice.
        let broken = script(
            tmp.path(),
            "broken-node",
            "printf 'No version is set\nConsider adding one\nnodejs 1\nnodejs 2\n' >&2; exit 126",
        );
        let error = resolve_node(&broken, tmp.path()).unwrap_err();
        assert!(
            error.contains("No version is set Consider adding one."),
            "{error}"
        );
        assert!(!error.contains("nodejs 1"), "{error}");
        assert!(error.contains("WORKTREE_BROWSER_NODE"), "{error}");
    }
}
