use crate::actions::OPEN_TIMEOUT;
use crate::runner::{self, Tool};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct PullRequest {
    pub number: u64,
    pub status: Status,
    url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Open,
    Draft,
    Closed,
    Merged,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Draft => "Draft",
            Self::Closed => "Closed",
            Self::Merged => "Merged",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct View {
    number: u64,
    url: String,
    state: String,
    #[serde(default)]
    is_draft: bool,
    head_ref_name: String,
}

pub async fn lookup(worktree: &Path, branch: &str) -> Option<PullRequest> {
    let cli = runner::find(Tool::Gh).ok()?;
    lookup_with(&cli, worktree, branch).await
}

async fn lookup_with(cli: &Path, worktree: &Path, branch: &str) -> Option<PullRequest> {
    // In worktree context, gh resolves the current branch and its GitHub remotes,
    // including fork/upstream relationships. A branch name alone lacks that context.
    // Start from the shared runner's command so the Git overrides are removed too.
    let mut command = tokio::process::Command::from(runner::command(cli));
    command
        .args([
            "pr",
            "view",
            "--json",
            "number,url,state,isDraft,headRefName",
        ])
        .current_dir(worktree)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GH_REPO")
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(8), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    from_response(&output.stdout, branch)
}

fn from_response(bytes: &[u8], branch: &str) -> Option<PullRequest> {
    let pr: View = serde_json::from_slice(bytes).ok()?;
    if pr.number == 0 || pr.head_ref_name != branch {
        return None;
    }
    let status = match pr.state.as_str() {
        "OPEN" if pr.is_draft => Status::Draft,
        "OPEN" => Status::Open,
        "CLOSED" => Status::Closed,
        "MERGED" => Status::Merged,
        _ => return None,
    };
    // Only pass an HTTPS pull request URL to the OS, never a custom URL scheme.
    let (host, path) = pr.url.strip_prefix("https://")?.split_once('/')?;
    if host.is_empty()
        || host.contains(['@', '\\'])
        || pr.url.chars().any(char::is_whitespace)
        || !path.ends_with(&format!("/pull/{}", pr.number))
    {
        return None;
    }
    Some(PullRequest {
        number: pr.number,
        status,
        url: pr.url,
    })
}

impl PullRequest {
    pub fn label(&self) -> String {
        format!("PR #{} · {}", self.number, self.status.label())
    }

    #[cfg(test)]
    pub(crate) fn for_test(number: u64) -> Self {
        Self {
            number,
            status: Status::Open,
            url: format!("https://github.com/example/repo/pull/{number}"),
        }
    }

    fn browser_command(&self) -> Command {
        let mut command = runner::command("/usr/bin/open");
        command.arg(&self.url);
        command
    }

    pub fn open(&self) -> Result<(), String> {
        let output = runner::run(&mut self.browser_command(), OPEN_TIMEOUT)
            .map_err(|err| format!("Could not open PR #{}: {err}", self.number))?;
        if !output.status.success() {
            return Err(format!(
                "Could not open PR #{}: {}",
                self.number,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_pull_request_status_for_the_matching_branch() {
        let open = br#"{"number":42,"url":"https://github.com/example/repo/pull/42","state":"OPEN","headRefName":"topic/review"}"#;
        let pr = from_response(open, "topic/review").unwrap();
        assert_eq!(pr.number, 42);
        assert_eq!(pr.status, Status::Open);
        let browser = pr.browser_command();
        assert_eq!(browser.get_program(), "/usr/bin/open");
        assert_eq!(
            browser.get_args().collect::<Vec<_>>(),
            ["https://github.com/example/repo/pull/42"]
        );
        assert!(from_response(open, "main").is_none());
        let response = |state, is_draft| {
            format!(
                r#"{{"number":42,"url":"https://github.com/example/repo/pull/42","state":"{state}","isDraft":{is_draft},"headRefName":"topic/review"}}"#,
            )
        };
        for (state, is_draft, status) in [
            ("OPEN", true, Status::Draft),
            ("CLOSED", true, Status::Closed),
            ("MERGED", false, Status::Merged),
        ] {
            assert_eq!(
                from_response(response(state, is_draft).as_bytes(), "topic/review")
                    .unwrap()
                    .status,
                status
            );
        }
        assert!(from_response(response("UNKNOWN", false).as_bytes(), "topic/review").is_none());
        let unsafe_url = String::from_utf8_lossy(open).replace("https://", "file://");
        assert!(from_response(unsafe_url.as_bytes(), "topic/review").is_none());
    }

    #[test]
    fn rejects_unsafe_or_mismatched_pull_request_urls() {
        let response = |number: u64, url: &str| {
            format!(r#"{{"number":{number},"url":"{url}","state":"OPEN","headRefName":"topic"}}"#)
        };
        for (number, url) in [
            (0, "https://github.com/o/r/pull/0"),
            (7, "http://github.com/o/r/pull/7"),
            (7, "https://github.com/o/r/pull/8"),
            (7, "https://user@github.com/o/r/pull/7"),
            (7, "https://github.com/o/r /pull/7"),
            (7, "https:///o/r/pull/7"),
            (7, "x-custom://github.com/o/r/pull/7"),
        ] {
            assert!(
                from_response(response(number, url).as_bytes(), "topic").is_none(),
                "{url}"
            );
        }
        assert!(from_response(b"not json", "topic").is_none());
        assert_eq!(
            from_response(
                response(7, "https://github.com/o/r/pull/7").as_bytes(),
                "topic"
            )
            .unwrap()
            .number,
            7
        );
    }

    #[test]
    fn a_failing_gh_hides_the_badge() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("gh");
        std::fs::write(
            &cli,
            "#!/bin/sh\necho 'no pull requests found' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(
            runtime
                .block_on(lookup_with(&cli, dir.path(), "topic"))
                .is_none()
        );
    }

    #[test]
    fn queries_gh_from_the_selected_worktree_without_prompting() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("linked checkout");
        std::fs::create_dir(&checkout).unwrap();
        let cli = dir.path().join("gh");
        std::fs::write(&cli, r#"#!/bin/sh
set -eu
[ "${PWD##*/}" = 'linked checkout' ]
[ "$GH_PROMPT_DISABLED" = 1 ]
[ "$GIT_TERMINAL_PROMPT" = 0 ]
[ -z "${GH_REPO-}" ]
[ "$*" = 'pr view --json number,url,state,isDraft,headRefName' ]
printf '%s\n' '{"number":42,"url":"https://github.example.com/example/repo/pull/42","state":"OPEN","headRefName":"topic/review"}'
"#).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            runtime
                .block_on(lookup_with(&cli, &checkout, "topic/review"))
                .unwrap()
                .number,
            42
        );
    }
}
