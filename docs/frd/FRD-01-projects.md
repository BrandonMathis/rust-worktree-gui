# FRD-01 — Project bookmarks

**Goal:** the user chooses which repositories appear in the app. Nothing is scanned automatically.

**Source:** initial request (2026-11-20T21:47Z): *"We want to selectively add projects to the GUI. So essentially I'll just add a directory and it'll appear as a project on the left-hand side and it'll be a dropdown."* Refined in `PLAN.md` §2.

| ID | Priority | Requirement | Verification | Status |
| --- | --- | --- | --- | --- |
| PRJ-01 | Must | **Add Project** opens a native macOS folder picker. The chosen folder may be a repository root, a subdirectory, or a linked worktree; all of these resolve to the same project. | `checkout_subdirectory_and_linked_checkout_share_repository_identity` | Implemented |
| PRJ-02 | Must | A repository is bookmarked at most once, identified by its canonical common Git directory. Adding another checkout of an already bookmarked repository selects the existing project and explains how to use **Replace Anchor**. | `deduplicates_linked_checkouts_and_persists_anchor_replacement`, `adding_another_checkout_of_a_bookmarked_repository_explains_replace_anchor` | Implemented |
| PRJ-03 | Must | Choosing a folder that isn't inside a Git checkout shows a readable error and adds nothing. | `a_folder_outside_git_is_a_readable_error` | Implemented |
| PRJ-04 | Must | Bookmarks persist across restarts in `~/Library/Application Support/Worktree Browser/projects.json`. Only bookmarks (anchor checkout, common dir) and display names are stored. The Git inventory is never cached. | `round_trips_only_bookmarks_and_preserves_unavailable_paths` | Implemented |
| PRJ-05 | Must | On startup, every bookmark is restored and refreshed. The first project is expanded and its overview selected. | `startup_restores_bookmarks_and_refreshes_each_project` | Implemented |
| PRJ-06 | Must | A malformed or invalid bookmarks file (bad JSON, relative paths, duplicate repositories) is reported, never silently treated as empty or overwritten. Adding projects is disabled until it's fixed. | `malformed_config_is_not_treated_as_an_empty_project_list`, `rejects_relative_paths_and_duplicate_repositories`, `unreadable_bookmarks_disable_adding_projects_and_show_the_error` | Implemented |
| PRJ-07 | Must | If the saved anchor checkout disappears, the project shows as unavailable. **Replace Anchor** accepts only another live checkout of the *same* repository. | `missing_anchor_is_an_error_but_a_live_checkout_can_replace_it`, `replacing_an_anchor_with_another_repository_is_rejected` | Implemented |
| PRJ-08 | Must | **Remove Project** asks for confirmation and names the project. Cancel leaves the bookmark and selection unchanged. Confirm removes only the bookmark, not Git data, files on disk, or running tmux sessions. If the project was selected, its terminal detaches and the first remaining project is selected. | `canceling_remove_project_keeps_the_bookmark_and_terminal`, `confirming_removal_only_unbookmarks_the_project_and_detaches_its_terminal`; Manual: choose **Remove Project** in the project menu, check the sheet names the project, and press Escape to cancel. | Implemented |
| PRJ-09 | Must | Bookmark writes are atomic (temp file + rename), so a failed save never loses existing bookmarks. Paths that aren’t valid UTF-8 round-trip exactly. | `round_trips_only_bookmarks_and_preserves_unavailable_paths` (round-trip); atomicity: Manual (code review) | Implemented |
| PRJ-10 | Must | A project is labeled with its repository's name, taken from the common Git directory, not from the anchor checkout. `…/repo/.git` and a hidden `…/repo/.bare` are named `repo`, and a bare `…/repo.git` is named `repo`. Adding the project from a linked worktree, or **Replace Anchor**, doesn't change its label. | `projects_are_named_after_the_repository_not_the_anchor_checkout` | Implemented |

## Out of scope

- Automatic repository discovery or scanning.
- Creating worktrees from the GUI (`PLAN.md` step 4, deferred).
