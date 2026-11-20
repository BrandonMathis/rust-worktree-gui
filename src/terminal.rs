use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use tokio::sync::oneshot;

const DEFAULT_BINARY: &str = "/Applications/Alacritty.app/Contents/MacOS/alacritty";

pub async fn launch(worktree: PathBuf) -> Result<(), String> {
    let executable = std::env::var_os("WORKTREE_BROWSER_ALACRITTY")
        .map_or_else(|| PathBuf::from(DEFAULT_BINARY), PathBuf::from);
    launch_with(executable, worktree).await
}

async fn launch_with(executable: PathBuf, worktree: PathBuf) -> Result<(), String> {
    if !executable.is_absolute() {
        return Err("WORKTREE_BROWSER_ALACRITTY must be an absolute path to the executable".into());
    }
    let (started, result) = oneshot::channel();
    // The worker owns the child. Report spawn immediately, then wait off the UI and task threads.
    thread::Builder::new()
        .name("alacritty-reaper".into())
        .spawn(move || match spawn(&executable, &worktree) {
            Ok(mut child) => {
                let _ = started.send(Ok(()));
                if let Err(error) = child.wait() {
                    eprintln!("Could not reap Alacritty: {error}");
                }
            }
            Err(error) => {
                let _ = started.send(Err(error));
            }
        })
        .map_err(|err| format!("Could not start the terminal launcher: {err}"))?;
    result
        .await
        .map_err(|_| "The terminal launcher stopped unexpectedly".to_owned())?
}

fn spawn(executable: &Path, worktree: &Path) -> Result<std::process::Child, String> {
    if !worktree.is_dir() {
        return Err(format!(
            "Worktree directory is unavailable: {}. Refresh to check Git again.",
            worktree.display()
        ));
    }
    Command::new(executable)
        .arg("--working-directory")
        .arg(worktree)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|err| {
            format!(
                "Could not open Alacritty at {}: {err}",
                executable.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    fn launch_for_test(executable: PathBuf, worktree: PathBuf) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(launch_with(executable, worktree))
    }

    #[test]
    fn passes_the_worktree_as_a_single_literal_argument() {
        let tmp = tempfile::tempdir().unwrap();
        let worktree = tmp.path().join("linked checkout\nwith $spaces");
        let executable = tmp.path().join("fake Alacritty");
        std::fs::create_dir(&worktree).unwrap();
        std::fs::write(&executable, "#!/bin/sh\n[ \"$#\" = 2 ] && [ \"$1\" = --working-directory ] && [ \"$2\" -ef . ] && touch launched\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        launch_for_test(executable, worktree.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !worktree.join("launched").is_file() {
            assert!(
                Instant::now() < deadline,
                "launcher did not receive the expected arguments"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_missing_binary_returns_a_readable_launch_error() {
        let tmp = tempfile::tempdir().unwrap();
        let executable = tmp.path().join("not installed");
        let error = launch_for_test(executable.clone(), tmp.path().to_path_buf()).unwrap_err();
        assert!(error.contains("Could not open Alacritty"));
        assert!(error.contains(executable.to_str().unwrap()));
    }
}
