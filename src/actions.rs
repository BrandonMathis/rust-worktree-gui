use crate::{runner, tmux};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `open` returns as soon as Launch Services has handed the path to the app.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryAction {
    Finder,
    VsCode,
    CopyPath,
}

impl DirectoryAction {
    pub const ALL: [Self; 3] = [Self::Finder, Self::VsCode, Self::CopyPath];

    pub fn run(self, root: PathBuf, repo: Option<PathBuf>) -> Result<Option<String>, String> {
        let path = repo
            .and_then(|repo| tmux::pane_directory(&repo, &root))
            .unwrap_or(root);
        if self == Self::CopyPath {
            return path
                .into_os_string()
                .into_string()
                .map(Some)
                .map_err(|_| "This directory path cannot be copied as text.".into());
        }
        if !path.is_dir() {
            return Err(format!("Directory not available: {}", path.display()));
        }
        let app = match self {
            Self::Finder => "com.apple.finder",
            Self::VsCode => "com.microsoft.VSCode",
            Self::CopyPath => unreachable!(),
        };
        let mut open = runner::command(Path::new("/usr/bin/open"));
        open.args(["-b", app, "--"]).arg(&path);
        let output =
            runner::run(&mut open, OPEN_TIMEOUT).map_err(|err| format!("{self}: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "{self}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(None)
    }
}

/// Whether `url` is a plain HTTPS link: the only kind the app hands to the OS.
/// A link in an agent's reply is untrusted text, so anything else (`file:`,
/// custom schemes that launch apps, embedded credentials) is refused.
fn is_https_link(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| {
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        !host.is_empty()
            && !host.contains('@')
            && !url.chars().any(|c| c.is_control() || c.is_whitespace())
    })
}

/// Opens an HTTPS link in the default browser.
pub fn open_https(url: &str) -> Result<(), String> {
    if !is_https_link(url) {
        return Err(format!("Only https:// links can be opened: {url}"));
    }
    let mut open = runner::command(Path::new("/usr/bin/open"));
    open.arg(url);
    let output = runner::run(&mut open, OPEN_TIMEOUT)
        .map_err(|err| format!("Could not open {url}: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not open {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

impl std::fmt::Display for DirectoryAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Finder => "Open in Finder",
            Self::VsCode => "Open in VS Code",
            Self::CopyPath => "Copy Directory Path",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn copy_path_uses_the_worktree_root_without_a_tmux_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout with spaces");
        assert_eq!(
            DirectoryAction::CopyPath.run(root.clone(), None),
            Ok(Some(root.display().to_string()))
        );
        let binary = dir.path().join(OsString::from_vec(b"bad\xff".to_vec()));
        assert!(DirectoryAction::CopyPath.run(binary, None).is_err());
    }

    #[test]
    fn open_actions_report_a_missing_directory_instead_of_launching() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        for action in [DirectoryAction::Finder, DirectoryAction::VsCode] {
            let error = action.run(missing.clone(), None).unwrap_err();
            assert!(error.starts_with("Directory not available"), "{error}");
        }
    }

    #[test]
    fn only_plain_https_links_are_opened() {
        assert!(is_https_link("https://example.com/docs?q=1#top"));
        for url in [
            "http://example.com",
            "file:///etc/passwd",
            "vscode://open?x",
            "https://",
            "https://user:pw@example.com/",
            "https://example.com/a b",
            "https://example.com/\nfile:///x",
            "-a Calculator",
        ] {
            assert!(!is_https_link(url), "{url}");
            assert!(open_https(url).is_err(), "{url}");
        }
    }

    #[test]
    fn menu_labels_match_the_documented_actions() {
        assert_eq!(
            DirectoryAction::ALL.map(|a| a.to_string()),
            ["Open in Finder", "Open in VS Code", "Copy Directory Path"]
        );
    }
}
