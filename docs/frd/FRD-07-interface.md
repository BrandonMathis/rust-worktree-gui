# FRD-07 — Look, layout and sidebar behavior

**Sources:**
- 2026-11-20T22:08Z: *"dark look interface that is Dracula themed with 3-pixel rounded borders and a semi-glass-like modern macOS feel."*
- 2026-11-20T22:25Z: *"The heading is a bit too big… Use some iconography to show current status."*
- 2026-11-20T22:59Z: *"make sidebar more compact, do not wrap branch name"*
- 2026-11-20T23:09Z and 23:14Z: *"Decrease non-valuable padding as much as possible… provide as much space for work as possible and be minimal and compact"*
- 2026-11-20T23:41Z: *"if the side-panel is collapsed and you hover over the expand toggle… show the side-panel… z-indexed on top of window contents… When you mouse out it goes away"*
- 2026-11-22: *"Project names are more distinguishable from worktree name. Icon on left of worktree is the only icon… different icon for main dir, worktree, worktree with pr, pr open, pr close, pr merge… Expand/collapse caret should be larger for project dir"*

| ID | Priority | Requirement | Verification | Status |
| --- | --- | --- | --- | --- |
| UI-01 | Must | Dark Dracula theme by default; the palette is user-editable (FRD-09). Panels, buttons and menus have 3-pixel rounded corners. The window is translucent with native macOS blur behind glass-style panels. | `panels_and_controls_use_three_pixel_dracula_rounding`; Manual: blur visible. | Implemented |
| UI-02 | Must | The sidebar lists projects as expandable rows, each with a large chevron caret and a bold name on a faint header fill. An expanded project shows its worktrees indented beneath it. | Manual: projects and worktrees are told apart at a glance at 1120×720 and 820×440. | Implemented |
| UI-03 | Must | Worktree names never wrap. Hovering a row (or the header title) shows the full name, branch and path in a tooltip. | Manual | Implemented |
| UI-04 | Must | Each sidebar worktree has exactly one icon, on its left: warning (unavailable), house (main checkout), branch (linked worktree with no PR), or the PR state (open, draft, closed, merged). The main checkout keeps the house even when it has a PR. Connection status is shown in the header, not the sidebar. | `sidebar_icon_marks_the_main_checkout_pr_state_and_unavailable_worktrees`; Manual: icon shapes and colors. | Implemented |
| UI-05 | Must | The worktree header is a single compact row: sidebar toggle (when hidden), title, terminal status/action, PR badge, Actions. | Manual: verified with macOS screenshots at default and minimum (820×440) window sizes. | Implemented |
| UI-06 | Must | The terminal fills its pane with no padding. Window margins and gutters are minimal. | Manual | Implemented |
| UI-07 | Must | The sidebar can be hidden to give the terminal full width, and restored from the header. Toggling never detaches the terminal or changes the selection. | `collapsing_sidebar_keeps_the_selected_worktree_and_connection` | Implemented |
| UI-08 | Must | With the sidebar hidden, hovering the show-sidebar toggle reveals the sidebar as an overlay above the workspace. The user can move into it and use it normally. It disappears shortly after the pointer leaves both the toggle and the panel. | `collapsed_sidebar_previews_until_the_pointer_leaves_both_zones` | Implemented |
| UI-09 | Must | Errors appear in a dismissible notice bar and never crash the app. | `unreadable_bookmarks_disable_adding_projects_and_show_the_error` (notice state); Manual (rendering) | Implemented |
| UI-10 | Should | The default window is 1120×720, with a minimum of 820×440. | Manual | Implemented |
| UI-11 | Should | Toolbar buttons are icon-only with tooltips (Add Project, Refresh, Hide sidebar). Unavailable actions are disabled (for example, while a folder picker is open). | `unreadable_bookmarks_disable_adding_projects_and_show_the_error` | Implemented |
| UI-12 | Must | Expanding or collapsing a project in the sidebar only shows or hides its worktrees. It never changes the selection, detaches the terminal or clears the PR badge. (A project is selected on expand only when nothing is selected yet.) | `expanding_another_project_keeps_the_selected_worktree_terminal_and_pr` | Implemented |
