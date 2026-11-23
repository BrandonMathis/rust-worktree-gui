# FRD-02 — Worktree discovery

**Goal:** Git alone defines which worktrees a project has and where they live.

**Source:** initial request (2026-11-20T21:47Z): *"Currently we use a Pi extension for managing worktrees, so we don't want to necessarily rely on that. If we could just use Git only, that would be preferable."*

| ID | Priority | Requirement | Verification | Status |
| --- | --- | --- | --- | --- |
| WT-01 | Must | Worktrees are read from `git worktree list --porcelain -z`, run from the project's anchor checkout with Git environment overrides (`GIT_DIR` etc.) removed. No Pi extension or other metadata is consulted. | `checkout_subdirectory_and_linked_checkout_share_repository_identity` | Implemented |
| WT-02 | Must | Paths with spaces, newlines and non-UTF-8 bytes are preserved exactly. Malformed or truncated Git output is rejected rather than half-parsed. | `preserves_paths_and_parses_flags`, `rejects_truncated_or_malformed_output` | Implemented |
| WT-03 | Must | Each worktree shows its branch name, `Detached HEAD`, or `Bare repository`. | `preserves_paths_and_parses_flags`, `refresh_finds_external_additions_and_missing_directories` | Implemented |
| WT-04 | Must | A worktree whose directory is missing, that Git marks prunable, or that is bare is shown as **unavailable** with a reason. It can't open a terminal. | `refresh_finds_external_additions_and_missing_directories`, `unavailable_worktrees_never_start_a_terminal` | Implemented |
| WT-05 | Must | **Refresh** (and startup, adding a project, and the automatic refreshes in WT-09 and WT-10) re-reads Git. Worktrees added or pruned outside the app appear or disappear. The app never prunes or deletes anything itself. | `refresh_finds_external_additions_and_missing_directories` | Implemented |
| WT-06 | Must | If the selected worktree disappears from Git, the selection falls back to the project overview and the terminal detaches. If its directory merely becomes unavailable, the row stays selected and the terminal detaches. A *failed* refresh (Git error, anchor gone) keeps the last good worktree list, the selection, the terminal and the PR badges, and shows the error next to the project. | `refresh_detaches_or_clears_a_selection_that_changed_outside_the_app`, `failed_refresh_keeps_the_selection_terminal_and_pr_badges` | Implemented |
| WT-07 | Must | Only one refresh per project runs at a time. Results from a refresh started before **Replace Anchor** are discarded. | `stale_refresh_results_are_ignored_after_replacing_the_anchor` | Implemented |
| WT-08 | Must | The anchor is re-validated on each refresh. If its path now belongs to a different repository, refresh fails with a message telling the user to replace the anchor. | `missing_anchor_is_an_error_but_a_live_checkout_can_replace_it` | Implemented |
| WT-09 | Should | When the app window gains focus, every project refreshes automatically. A refresh already in flight is not repeated. | `window_focus_refreshes_every_project_automatically` | Implemented |
| WT-10 | Should | While the app runs, it checks each repository's Git metadata (`HEAD`, `refs/heads`, `packed-refs`, `worktrees/*/{HEAD,gitdir,locked}` and whether each linked checkout still exists) with `stat` every 2 seconds, off the UI thread. A change refreshes that project, so a worktree added, removed, moved or switched from the embedded terminal appears without clicking Refresh. Only one check runs at a time, and stale results are ignored. Nothing about the inventory is cached: Git is re-read on every change. | `fingerprint_changes_when_worktrees_or_branches_change_outside_the_app`, `changed_git_metadata_starts_an_automatic_refresh` | Implemented |
| WT-11 | Should | Automatic refreshes keep the PR badges of worktrees whose branch didn't change and run `gh` only for new or changed worktrees. A manual **Refresh**, even one pressed while an automatic refresh runs, looks up every PR again. | `automatic_refresh_keeps_pr_badges_and_runs_gh_only_for_changed_branches` | Implemented |

## Out of scope

- File-system event watchers (FSEvents). WT-10 polls a few metadata files instead, which needs no dependency and isn't flooded with events while Git rewrites its files. A commit on a nested branch (`refs/heads/feature/x`) doesn't trigger a refresh, so the commit hash in the header tooltip can lag until the next refresh.
- Dirty-status scans.
