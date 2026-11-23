//! Pull request badges from the GitHub CLI.
//!
//! One `gh pr list` call per project returns recent PRs. Each worktree is then matched
//! to a PR by its branch name ([`best_for_branch`]), and the result is kept in
//! [`Badges`] until the next lookup answers.

use crate::actions::OPEN_TIMEOUT;
use crate::git::Worktree;
use crate::runner::{self, Tool};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// `gh` talks to the network, so give up quietly after this long.
const TIMEOUT: Duration = Duration::from_secs(8);
/// How many PRs to ask for, newest first. A branch whose only PR is older than this
/// shows no badge.
const LIMIT: &str = "200";
const FIELDS: &str = "number,state,isDraft,headRefName,url,isCrossRepository";

#[derive(Clone, Debug)]
pub struct PullRequest {
    pub number: u64,
    pub status: Status,
    url: String,
    /// The PR's head branch. A badge is dropped when its worktree leaves this branch.
    head: String,
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

    fn is_open(self) -> bool {
        matches!(self, Self::Open | Self::Draft)
    }
}

/// One entry of `gh pr list --json …`. Serde fills it from the JSON by field name.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    number: u64,
    url: String,
    state: String,
    #[serde(default)]
    is_draft: bool,
    head_ref_name: String,
    #[serde(default)]
    is_cross_repository: bool,
}

/// Lists the recent PRs of the repository checked out at `checkout`. Blocking: run it
/// with `background(...)`. `None` means no badges: `gh` is missing, not logged in,
/// failed, timed out or returned something unexpected.
pub fn list(checkout: &Path) -> Option<Vec<PullRequest>> {
    let cli = runner::find(Tool::Gh).ok()?;
    list_with(&cli, checkout, TIMEOUT)
}

fn list_with(cli: &Path, checkout: &Path, timeout: Duration) -> Option<Vec<PullRequest>> {
    let mut command = runner::command(cli);
    command
        .args(["pr", "list", "--state", "all", "--limit", LIMIT])
        .args(["--json", FIELDS])
        // gh finds the GitHub repository from this checkout's remotes.
        .current_dir(checkout)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GH_REPO");
    let output = runner::run(&mut command, timeout).ok()?;
    if !output.status.success() {
        return None;
    }
    from_list(&output.stdout)
}

/// Parses the JSON array. Entries that fail validation are skipped, not fatal.
fn from_list(bytes: &[u8]) -> Option<Vec<PullRequest>> {
    let listed: Vec<Listed> = serde_json::from_slice(bytes).ok()?;
    Some(listed.into_iter().filter_map(validate).collect())
}

fn validate(pr: Listed) -> Option<PullRequest> {
    // A PR from a fork can reuse any branch name (`main`, `patch-1`), so matching it by
    // name alone would badge the wrong worktree. Only same-repository PRs count.
    if pr.number == 0 || pr.is_cross_repository || pr.head_ref_name.is_empty() {
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
        head: pr.head_ref_name,
    })
}

/// The PR to badge for `branch`. If several PRs share the head branch (for example
/// one was closed and a new one opened), an open or draft PR wins over a closed or
/// merged one. Within the same group the highest number (the most recently created)
/// wins.
fn best_for_branch<'a>(prs: &'a [PullRequest], branch: &str) -> Option<&'a PullRequest> {
    prs.iter()
        .filter(|pr| pr.head == branch)
        // Tuples compare field by field: `true > false`, then the larger number.
        .max_by_key(|pr| (pr.status.is_open(), pr.number))
}

/// The branch to match a worktree's PR by, or `None` if it has no badge: detached
/// HEAD, bare, missing or prunable worktrees are never looked up.
pub fn branch_to_match(tree: &Worktree) -> Option<&str> {
    if tree.detached || tree.unavailable_reason().is_some() {
        return None;
    }
    tree.branch.as_deref()
}

/// One project's PR badges, keyed by worktree path, and the lookup in flight.
#[derive(Debug, Default)]
pub struct Badges {
    by_worktree: BTreeMap<PathBuf, PullRequest>,
    request_id: Option<u64>,
}

impl Badges {
    pub fn get(&self, worktree: &Path) -> Option<&PullRequest> {
        self.by_worktree.get(worktree)
    }

    pub fn in_flight(&self) -> bool {
        self.request_id.is_some()
    }

    /// Records that lookup `request_id` started. Only its answer will be applied.
    pub fn start(&mut self, request_id: u64) {
        self.request_id = Some(request_id);
    }

    /// Applies the answer to lookup `request_id`, matched against the worktrees listed
    /// now (they may have changed since the lookup started). An answer to an older or
    /// cancelled lookup is ignored. `None` (no usable answer from `gh`) clears the badges.
    pub fn finish(
        &mut self,
        request_id: u64,
        prs: Option<Vec<PullRequest>>,
        worktrees: &[Worktree],
    ) {
        if self.request_id != Some(request_id) {
            return;
        }
        self.request_id = None;
        let prs = prs.unwrap_or_default();
        self.by_worktree = worktrees
            .iter()
            .filter_map(|tree| {
                let pr = best_for_branch(&prs, branch_to_match(tree)?)?;
                Some((tree.path.clone(), pr.clone()))
            })
            .collect();
    }

    /// Keeps each badge while its worktree is still listed on the same branch, so a
    /// refresh doesn't make every badge flicker. Drops the rest.
    pub fn retain_listed(&mut self, worktrees: &[Worktree]) {
        self.by_worktree.retain(|path, pr| {
            worktrees
                .iter()
                .any(|tree| tree.path == *path && branch_to_match(tree) == Some(pr.head.as_str()))
        });
    }

    /// Drops every badge and ignores the lookup in flight (for example after
    /// **Replace Anchor**).
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

impl PullRequest {
    pub fn label(&self) -> String {
        format!("PR #{} · {}", self.number, self.status.label())
    }

    #[cfg(test)]
    pub(crate) fn for_test(number: u64, head: &str) -> Self {
        Self {
            number,
            status: Status::Open,
            url: format!("https://github.com/example/repo/pull/{number}"),
            head: head.to_owned(),
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
    use crate::runner::tests::script;

    fn entry(number: u64, state: &str, is_draft: bool, head: &str) -> String {
        format!(
            r#"{{"number":{number},"url":"https://github.com/example/repo/pull/{number}","state":"{state}","isDraft":{is_draft},"headRefName":"{head}","isCrossRepository":false}}"#
        )
    }

    fn list_of(entries: &[String]) -> Vec<PullRequest> {
        from_list(format!("[{}]", entries.join(",")).as_bytes()).unwrap()
    }

    #[test]
    fn detects_each_pull_request_status_for_the_matching_branch() {
        let prs = list_of(&[entry(42, "OPEN", false, "topic/review")]);
        let pr = best_for_branch(&prs, "topic/review").unwrap();
        assert_eq!(pr.number, 42);
        assert_eq!(pr.status, Status::Open);
        let browser = pr.browser_command();
        assert_eq!(browser.get_program(), "/usr/bin/open");
        assert_eq!(
            browser.get_args().collect::<Vec<_>>(),
            ["https://github.com/example/repo/pull/42"]
        );
        assert!(best_for_branch(&prs, "main").is_none());
        for (state, is_draft, status) in [
            ("OPEN", true, Status::Draft),
            ("CLOSED", true, Status::Closed),
            ("MERGED", false, Status::Merged),
        ] {
            let prs = list_of(&[entry(42, state, is_draft, "topic/review")]);
            assert_eq!(prs[0].status, status);
        }
        assert!(list_of(&[entry(42, "UNKNOWN", false, "topic/review")]).is_empty());
    }

    #[test]
    fn several_pull_requests_for_one_branch_prefer_open_then_most_recent() {
        let prs = list_of(&[
            entry(30, "MERGED", false, "topic"),
            entry(12, "OPEN", true, "topic"),
            entry(10, "OPEN", false, "topic"),
            entry(31, "CLOSED", false, "other"),
            entry(29, "CLOSED", false, "other"),
        ]);
        assert_eq!(best_for_branch(&prs, "topic").unwrap().number, 12);
        assert_eq!(best_for_branch(&prs, "other").unwrap().number, 31);

        // A fork's PR reusing the branch name never badges the local worktree.
        let fork = entry(40, "OPEN", false, "topic").replace(
            r#""isCrossRepository":false"#,
            r#""isCrossRepository":true"#,
        );
        let prs = list_of(&[fork, entry(5, "MERGED", false, "topic")]);
        assert_eq!(best_for_branch(&prs, "topic").unwrap().number, 5);
    }

    #[test]
    fn rejects_unsafe_or_mismatched_pull_request_urls() {
        let response = |number: u64, url: &str| {
            format!(r#"[{{"number":{number},"url":"{url}","state":"OPEN","headRefName":"topic"}}]"#)
        };
        for (number, url) in [
            (0, "https://github.com/o/r/pull/0"),
            (7, "http://github.com/o/r/pull/7"),
            (7, "https://github.com/o/r/pull/8"),
            (7, "https://user@github.com/o/r/pull/7"),
            (7, "https://github.com/o/r /pull/7"),
            (7, "https:///o/r/pull/7"),
            (7, "x-custom://github.com/o/r/pull/7"),
            (7, "file://github.com/o/r/pull/7"),
        ] {
            let prs = from_list(response(number, url).as_bytes()).unwrap();
            assert!(prs.is_empty(), "{url}");
        }
        assert!(from_list(b"not json").is_none());
        let prs = from_list(response(7, "https://github.com/o/r/pull/7").as_bytes()).unwrap();
        assert_eq!(prs[0].number, 7);
    }

    #[test]
    fn a_failing_gh_hides_the_badge() {
        let dir = tempfile::tempdir().unwrap();
        let cli = script(
            dir.path(),
            "gh",
            "echo 'To get started with GitHub CLI, please run: gh auth login' >&2\nexit 4",
        );
        assert!(list_with(&cli, dir.path(), TIMEOUT).is_none());
    }

    #[test]
    fn a_slow_gh_times_out_and_hides_the_badge() {
        let dir = tempfile::tempdir().unwrap();
        let cli = script(dir.path(), "gh", "exec /bin/sleep 30");
        let started = std::time::Instant::now();
        assert!(list_with(&cli, dir.path(), Duration::from_millis(200)).is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn lists_pull_requests_once_from_the_project_checkout_without_prompting() {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("main checkout");
        std::fs::create_dir(&checkout).unwrap();
        let cli = script(
            dir.path(),
            "gh",
            &format!(
                r#"set -eu
[ "${{PWD##*/}}" = 'main checkout' ]
[ "$GH_PROMPT_DISABLED" = 1 ]
[ "$GIT_TERMINAL_PROMPT" = 0 ]
[ -z "${{GH_REPO-}}" ]
[ "$*" = 'pr list --state all --limit 200 --json {FIELDS}' ]
printf '%s\n' '[{}, {}]'"#,
                entry(42, "OPEN", false, "topic/review"),
                entry(7, "MERGED", false, "old"),
            ),
        );
        let prs = list_with(&cli, &checkout, TIMEOUT).unwrap();
        assert_eq!(prs.len(), 2);
        assert_eq!(best_for_branch(&prs, "topic/review").unwrap().number, 42);
    }
}
