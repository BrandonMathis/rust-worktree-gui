//! The window title: the app's version, plus the branch and commit when the
//! build isn't a release.
//!
//! `build.rs` records the commit the app was built from as compile-time
//! environment variables. `option_env!` reads them while compiling and yields
//! `None` when one wasn't set (a build without Git), so nothing here runs Git.

const APP_NAME: &str = "Worktree Browser";

/// The Git commit the binary was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Build<'a> {
    sha: &'a str,
    /// Empty on a detached HEAD.
    branch: &'a str,
    /// Space-separated tags pointing at the commit.
    tags: &'a str,
}

/// The title of this binary's window, from the version in `Cargo.toml` and
/// the commit recorded by `build.rs`.
pub fn window_title() -> String {
    let build = option_env!("WORKTREE_BROWSER_GIT_SHA").map(|sha| Build {
        sha,
        branch: option_env!("WORKTREE_BROWSER_GIT_BRANCH").unwrap_or_default(),
        tags: option_env!("WORKTREE_BROWSER_GIT_TAGS").unwrap_or_default(),
    });
    title(env!("CARGO_PKG_VERSION"), build)
}

/// `Worktree Browser v0.3.1` for a release: a commit tagged with its own
/// version, as the release script tags it. Any other commit also names the
/// branch (or `detached`) and the short SHA, so a development build can't be
/// mistaken for the release it started from. Without Git details there's
/// nothing to add, so the title shows just the version.
fn title(version: &str, build: Option<Build>) -> String {
    let release_tag = format!("v{version}");
    match build {
        Some(build) if !build.tags.split(' ').any(|tag| tag == release_tag) => {
            let branch = if build.branch.is_empty() {
                "detached"
            } else {
                build.branch
            };
            format!("{APP_NAME} {release_tag} ({branch} @ {})", build.sha)
        }
        _ => format!("{APP_NAME} {release_tag}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build<'a>(branch: &'a str, tags: &'a str) -> Option<Build<'a>> {
        Some(Build {
            sha: "2be64ec",
            branch,
            tags,
        })
    }

    #[test]
    fn a_release_build_shows_only_its_version() {
        assert_eq!(
            title("0.3.1", build("main", "v0.3.1")),
            "Worktree Browser v0.3.1"
        );
        // A detached checkout of the release tag is still the release.
        assert_eq!(
            title("0.3.1", build("", "nightly v0.3.1")),
            "Worktree Browser v0.3.1"
        );
    }

    #[test]
    fn a_development_build_shows_its_branch_and_commit() {
        assert_eq!(
            title("0.3.1", build("feat/version-title", "")),
            "Worktree Browser v0.3.1 (feat/version-title @ 2be64ec)"
        );
        // Another version's tag doesn't make this a release.
        assert_eq!(
            title("0.3.2", build("main", "v0.3.1")),
            "Worktree Browser v0.3.2 (main @ 2be64ec)"
        );
        assert_eq!(
            title("0.3.1", build("", "")),
            "Worktree Browser v0.3.1 (detached @ 2be64ec)"
        );
    }

    #[test]
    fn a_build_without_git_shows_only_its_version() {
        assert_eq!(title("0.3.1", None), "Worktree Browser v0.3.1");
    }
}
