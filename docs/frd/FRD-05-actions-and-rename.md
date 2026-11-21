# FRD-05 — Actions menu, row menu and rename

**Sources:**
- 2026-11-20T22:25Z: *"Provide an actions button… so we could open in Finder and also open up wherever we're at in that directory in VS Code."*
- 2026-11-20T22:59Z: *"allow rename of worktree name (not branch name)"*
- 2026-11-20T23:32Z: *"have a … (vertical icon) menu appear when you hover over a worktree in the left panel… with the 'Actions' for that worktree duplicated in there"*

| ID | Priority | Requirement | Verification | Status |
| --- | --- | --- | --- | --- |
| ACT-01 | Must | The header **Actions** menu offers **Open in Finder**, **Open in VS Code** and **Copy Directory Path**. For a worktree it also offers **Rename Worktree**. | `row_menu_mirrors_the_header_actions`, `menu_labels_match_the_documented_actions` | Implemented |
| ACT-02 | Must | For a worktree with a live tmux session, actions target the active pane's current directory. Otherwise they use the worktree root. In the project overview they use the anchor checkout. | `copy_path_uses_the_worktree_root_without_a_tmux_session`; Manual: `cd` into a subfolder in tmux, then **Open in Finder**. | Implemented |
| ACT-03 | Must | Opening a missing directory shows an error instead of launching Finder or VS Code. A path that isn't valid UTF-8 can't be copied and says so. | `open_actions_report_a_missing_directory_instead_of_launching`, `copy_path_uses_the_worktree_root_without_a_tmux_session` | Implemented |
| ACT-04 | Must | Hovering a sidebar worktree reveals a **⋮** button. Its menu has the same items as the header Actions menu. | `row_menu_mirrors_the_header_actions` | Implemented |
| ACT-05 | Must | Row-menu actions apply to the hovered worktree without changing the selection or the attached terminal. | `sidebar_actions_target_their_worktree_without_switching_the_console` | Implemented |
| ACT-06 | Must | **Rename Worktree** sets an app-only display name. It never changes the Git branch, folder or tmux session, and it persists across restarts and anchor replacement. Clearing the name shows the branch again. | `worktree_display_names_survive_restart_without_changing_git` | Implemented |
| ACT-07 | Must | Display names are trimmed, single-line, at most 80 characters, with no control characters. The editor starts with the current name. **Cancel** discards the edit. | `rename_validates_input_and_cancel_discards_the_draft` | Implemented |
| ACT-08 | Must | The project overview has no rename action. | `rename_validates_input_and_cancel_discards_the_draft` | Implemented |
