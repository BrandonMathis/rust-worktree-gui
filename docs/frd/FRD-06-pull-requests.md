# FRD-06 — Pull request detection

**Source:** 2026-11-20T23:05Z: *"attempt to detect if there is PR open for this worktree. If so have icon next to dropleft in upper right that is PR icon with Worktree # and number that when you click opens worktree in os default browser"*. Confirmed at 23:39Z that only **open** PRs show a badge (merged PRs don't).

| ID | Priority | Requirement | Verification | Status |
| --- | --- | --- | --- | --- |
| PR-01 | Must | When a worktree on a branch is selected (or on **Refresh**), the app asks the GitHub CLI (`gh pr view`) from inside that worktree for its PR, using the user's existing `gh` login. | `queries_gh_from_the_selected_worktree_without_prompting`, `selecting_a_worktree_connects_its_terminal_and_looks_up_its_pr` | Implemented |
| PR-02 | Must | A **PR #N** button appears left of **Actions** only if the PR is **open** and its head branch matches the worktree's branch. | `shows_only_open_pull_requests_for_the_selected_branch` | Implemented |
| PR-03 | Must | Clicking the badge opens the PR URL in the default browser. Only `https://…/pull/<N>` URLs are accepted; other schemes, credentials in the host, whitespace or a mismatched number hide the badge. | `shows_only_open_pull_requests_for_the_selected_branch`, `rejects_unsafe_or_mismatched_pull_request_urls` | Implemented |
| PR-04 | Must | The lookup never prompts (`GH_PROMPT_DISABLED`, `GIT_TERMINAL_PROMPT=0`, no stdin), ignores `GH_REPO`, and times out after 8 seconds. A missing `gh`, error, or timeout hides the badge silently. | `queries_gh_from_the_selected_worktree_without_prompting`, `a_failing_gh_hides_the_badge`; timeout: Manual (code review) | Implemented |
| PR-05 | Must | The badge belongs to the selected worktree only. Switching worktrees clears it, and late answers for a previous selection are ignored. | `pr_badge_belongs_to_the_selected_worktree_only` | Implemented |
| PR-06 | Must | Detached HEAD and unavailable worktrees aren't looked up. | `selecting_a_worktree_connects_its_terminal_and_looks_up_its_pr` | Implemented |
