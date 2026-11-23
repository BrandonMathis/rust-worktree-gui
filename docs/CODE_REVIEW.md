# Code review against the FRD — 2026-11-20

An independent reviewer agent did a read-only pass over `src/`, the tests and [docs/frd/](frd/README.md). At review time `cargo test` (43 tests) and `cargo clippy -D warnings` were clean. `app.rs` line numbers may drift: it was being edited concurrently.

## Prioritized changes

There are no Critical or High findings. Security is sound: subprocesses use argument vectors, `open -b … --` guards paths, and only HTTPS PR URLs are opened.

### Medium

| # | Where | Problem | Fix |
| --- | --- | --- | --- |
| M1 | `app.rs` `Message::Refreshed` | A failed refresh (git error, anchor gone) stores `Err`, so `worktrees()` returns nothing. The selected worktree is then treated as deleted: the selection jumps to Overview and the terminal detaches, even though the worktree still exists. Violates WT-06. | Keep the last good inventory. Reconcile the selection only on `Ok`. Add a test. |
| M2 | `app.rs` `Message::ToggleProject` | Expanding another project in the sidebar selects it, which detaches the current terminal and clears the PR badge. Browsing the sidebar loses your view. | Expanding changes only `expanded`. Change the selection through Overview or a worktree click. |
| M3 | `app.rs` `Project::name` | **Addressed:** `Checkout::repository_name` derives the label from `common_dir` (PRJ-10). The project label is the anchor folder's name. After adding from a linked checkout, or after **Replace Anchor**, the project is labeled with that worktree's folder (for example `feature-x`). | Derive the name from `common_dir` (the parent of `.git`), or store a display name. |

### Low

| # | Where | Problem | Fix |
| --- | --- | --- | --- |
| L1 | `git.rs`, `tmux.rs`, `pull_request.rs` | **Addressed** by R5 (`src/runner.rs`, NFR-10). Only `git` removes the `GIT_*` overrides, and only four of them. tmux and `gh` inherit the full environment, so launching from a git hook leaks `GIT_DIR` into every embedded shell. | One shared helper that removes the overrides, used for every subprocess. |
| L2 | `terminal_config.rs` `read_config` | Cycles are detected only by a depth limit of 5, so a legitimate chain of 6 imports is reported as a cycle. | Track visited canonical paths. Keep the depth limit as a backstop. |
| L3 | `app.rs` `App::new` | If the bookmarks file and the Alacritty config both fail, the Alacritty error is dropped. | Join the two messages. |
| L4 | `app.rs` `SelectWorktree` | **Addressed:** re-selecting skips the lookup, lookups are batched per project, and a refresh keeps badges until the answer arrives (PR-01, PR-05). Clicking the already-selected worktree runs `gh` again and makes the PR badge flicker. | Skip the lookup when the selection is unchanged. |
| L5 | `app.rs` `Forget` | Forgetting happens on a single click and deletes all display names saved for that project. | Ask for confirmation, or offer Undo in the notice. |
| L6 | `config.rs` load | Display names read from disk skip the rename validation, and names for pruned worktrees are kept forever. | Validate on load. Drop names whose paths are gone after a successful refresh. |
| L7 | `git.rs` | **Addressed** by R5 (`src/runner.rs`, NFR-10). `git` is found only through `PATH`, while tmux and `gh` have Homebrew fallbacks. Git calls have no timeout, so a hung git disables Refresh for the rest of the session. | One resolver for all tools, with timeouts. |
| L8 | `app.rs` `Pick` | Opening the folder picker clears any notice, including an unread config error. | Clear only notices from earlier actions. |
| L9 | `tmux.rs` | tmux 3.5 is required but never checked. An old tmux fails with only "exited with status 1". | Check `tmux -V` once and show upgrade guidance. |
| L10 | tests | The TRM-02 test re-sends `Reconnect` instead of selecting the worktree a second time. PR-04's timeout, PRJ-09's atomicity and ACT-02's tmux pane-directory path aren't automated. The FRDs now mark these as manual or partial. | Add an injectable timeout, and assert that a second `SelectWorktree` adds no connection. |

## Terminal performance (2026-11-21)

A `sample` profile of the debug build while typing showed the main thread about 72% busy. GPU text preparation and the canvas rebuild in `vendor/iced_term` dominated, followed by palette hex parsing in `Theme::get_color`. NFR-08 fixes the low-hanging causes. Remaining follow-ups:

| # | Size | Change |
| --- | --- | --- |
| P1 | M | `TerminalView::draw` emits one `fill_text` (and one `String`) per cell, so a 120×40 grid prepares about 4,800 text items per frame. Emitting one run per line and style would cut most of the remaining cost, but needs care to keep the grid aligned to Alacritty's cell metrics. |
| P2 | S | `Backend::sync` clones the whole visible grid on every event. A damage-aware sync, or borrowing the grid under the lock during `draw`, would avoid the copy. |
| P3 | S | `Cargo.toml` lists only `tiny-skia`, but `vendor/iced_term` depends on iced with default features, so `wgpu` is enabled and is the renderer actually used. Set `default-features = false` there and name `wgpu` explicitly so the manifest says what runs. |

## Refactors for extensibility

| # | Size | Change |
| --- | --- | --- |
| R1 | L | Split `app.rs` (about 1,800 lines) into `app/{state,update,tests}.rs` and `app/view/{sidebar,header,overview}.rs`. Break the 30-arm `update` match into named handlers. |
| R2 | M | Group related state: `TerminalPane { state, next_id, scale, prefs }`, `PrBadge { request_id, pr }`, and an id generator for the counters. |
| R3 | S | One `on_selection_changed()` that detaches the terminal, clears the PR and cancels the rename draft. Today this logic is repeated in `select`, `Refreshed` and `forget`, which clears the draft twice. |
| R4 | M | Typed errors (`GitError`, `TmuxError`, `StoreError`) instead of `String`, rendered to user text in one place. Tests then match variants instead of `contains`. |
| R5 | M | **Addressed:** `src/runner.rs` finds tools, removes the Git overrides and applies per-call timeouts for git, tmux, `gh` and `open` (NFR-10). Tests still fake tools with shell scripts. Original proposal: a `Runner` abstraction for subprocesses, owning tool lookup, environment cleanup and timeouts. Fixes L1 and L7 together and makes tmux, `gh` and `open` testable without shell-script fakes. |
| R6 | S | Move `tests/git_fixtures.rs` to `src/test_support.rs` and drop `autotests = false` and the `#[path]` include. It isn't an integration test. |
| R7 | S | Drop the `x11` and `wayland` iced features from this macOS-only build. |

## Dead or drifted code

| Item | Problem | Proposed change |
| --- | --- | --- |
| `Console`, `ConsoleState`, `style::console` | "Console" naming from an earlier phase. `style::console` also styles every tooltip. | `EmbeddedTerminal`, `TerminalState`, `style::panel` |
| `Message::Reconnect` | It drives Connect, Retry and Reconnect. | `Message::Connect` |
| `PickMode { Add, Replace }` | The name doesn't say what is being picked. | `FolderPickPurpose { AddProject, ReplaceAnchor }` |
| `SessionNames.legacy` | Migration-only code for the old 35-character names. | Rename to `v1_long_name` and add a removal note. Remove from `pane_directory` once migration is retired. |
| `Worktree.locked` | Parsed but never shown. | Show a lock badge, or remove the field. |
| `icons::connecting` | Also used for the Refresh button. | Add `icons::refresh`. |
| `DirectoryAction::run(root, repo)` | `repo` is the common Git dir, used only for the tmux lookup. | Rename to `session_repo` and add a doc comment. |
| `terminal_config.rs` `scale_factor: 1.27`, `let _ = scale;` | Unexplained magic number, and a workaround for an unused variable. | Add a comment, and use `cfg_attr(…, allow(unused))`. |
| "native sheet" comment on the folder picker | It's an app-modal panel with no parent window, not a sheet. | Fix the comment. |
| `pub` on `ProjectId`, `PickMode`, `SidebarZone`, `Selection`, `MenuAction` | Not needed in a binary crate. | Make them private or `pub(crate)`. |
| `row_menu::show(format!("{target:?}"), …)` | Uses `Debug` output as the widget identity. | Add a `Selection::key()`. |
| `PLAN.md` | Still describes **Sessions** rows, **New Session** and the external Alacritty window. | Move it to `docs/history/PLAN-external-alacritty.md` and update the FRD links. |
