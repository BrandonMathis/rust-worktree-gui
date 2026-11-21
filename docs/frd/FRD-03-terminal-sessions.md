# FRD-03 — Embedded terminal and tmux sessions

**Goal:** clicking a worktree drops the user into a persistent shell for that worktree, inside the app window, like the VS Code agents view.

**Sources:**
- 2026-11-20T22:08Z: *"We want Alacritty running inside of the GUI, not as a separate window… when we click into a work tree, the terminal session opens right back up and a TMUX session is opened or resumed for that specific worktree."*
- 2026-11-20T22:25Z: *"Z shell is default inside of terminal."*
- 2026-11-20T22:44Z: *"shorten the tmux session name."*

This supersedes the original plan (`PLAN.md`), which launched Alacritty as an external window.

| ID | Priority | Requirement | Verification | Status |
| --- | --- | --- | --- | --- |
| TRM-01 | Must | The terminal is embedded in the main window using Alacritty's Rust terminal engine (`alacritty_terminal` via `iced_term`). It doesn't require the Alacritty app or open another window. | Manual: select a worktree; no new window appears. | Implemented |
| TRM-02 | Must | Selecting an available worktree immediately starts connecting its terminal. Selecting it again while it's connecting doesn't start a second connection. | `selecting_a_worktree_connects_its_terminal_and_looks_up_its_pr` | Implemented |
| TRM-03 | Must | Each worktree maps to one tmux session (`new-session -A`: create or attach). The session name is derived from the repository's common Git dir and the canonical worktree path, not the branch or the anchor, so renaming a branch keeps the session. | `reuses_name_by_repository_and_worktree_not_branch_or_anchor` | Implemented |
| TRM-04 | Must | Session names are short: `wt-` followed by 10 hex digits. Sessions created under the older 32-hex name are renamed in place without killing them. If the short name is already taken, the legacy session keeps its name. | `reuses_name_by_repository_and_worktree_not_branch_or_anchor`, `migrates_tmux_session_in_place_and_preserves_a_conflicting_session` | Implemented |
| TRM-05 | Must | All app sessions live on a dedicated tmux socket (`-L worktree-browser`), isolated from the user's default tmux server. | `isolates_sessions_without_overriding_home_config_and_passes_a_literal_cwd` | Implemented |
| TRM-06 | Must | New panes default to `/bin/zsh` (unless the user's tmux config chooses otherwise). The first pane starts in the worktree directory, passed literally (no shell or tmux format expansion). | `isolates_sessions_without_overriding_home_config_and_passes_a_literal_cwd` (settings); Manual: `echo $0` in a new pane shows zsh. tmux reads `SHELL` only when its server starts. | Implemented |
| TRM-07 | Must | Switching worktrees, **Detach**, collapsing the sidebar, or quitting the app disconnects the view without stopping the shell. Reconnecting resumes the same shell. | `collapsing_sidebar_keeps_the_selected_worktree_and_connection`, `selecting_a_worktree_connects_its_terminal_and_looks_up_its_pr`; Manual: export a variable, switch away and back, restart the app, check it's still set. | Implemented |
| TRM-08 | Must | A connection result for a worktree that is no longer selected is ignored. | `switching_worktrees_ignores_a_stale_connection_result` | Implemented |
| TRM-09 | Must | Header status shows **Connecting…**, **Attached**, **Detached** or **Could not connect**, with the matching action (Cancel, Detach, Connect/Reconnect, Retry). | Manual (rendering); state transitions covered by `selecting_a_worktree_connects_its_terminal_and_looks_up_its_pr`. | Implemented |
| TRM-10 | Must | tmux is found on `PATH` or in common Homebrew/MacPorts locations. `WORKTREE_BROWSER_TMUX` overrides it but must be an absolute path to an executable. A missing tmux shows install guidance. | `rejects_missing_or_relative_tmux_override` | Implemented |
| TRM-11 | Must | Unavailable worktrees never start a terminal. | `unavailable_worktrees_never_start_a_terminal` | Implemented |
| TRM-12 | Should | The terminal redraws correctly on resize and tmux pane splits. `⌘C`/`⌘V` copy and paste, and `Ctrl+C` interrupts. | Manual | Implemented |

## Out of scope

- Sessions surviving a reboot (tmux servers don't).
- Multiple simultaneous terminals or tabs per worktree (use tmux windows and panes).
- Automatically launching a coding agent. The user starts it in the shell.
