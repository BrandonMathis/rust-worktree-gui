# Agent instructions — Worktree Browser

A macOS desktop app in Rust (iced 0.14) for working across Git worktrees. Bookmark repositories, browse their worktrees, and open an embedded terminal (Alacritty's engine via `iced_term`) attached to a persistent per-worktree tmux session. See `README.md` for user-facing behavior.

The project is also a learning exercise: the owner is a React developer learning Rust. Prefer idiomatic, conventional Rust. Explain non-obvious Rust or iced patterns briefly in comments or replies, rather than writing clever code.

## Commands

Rust 1.90.0 is pinned in `.tool-versions` (asdf). Always pass `--locked`.

```sh
cargo run --locked                                   # launch the app (needs tmux)
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked                                  # all tests; add a name to filter
```

All three checks must pass before you call work done. To test a missing tmux, run with `WORKTREE_BROWSER_TMUX=/nonexistent/tmux`.

## Requirements are the spec

- `docs/frd/` holds the functional requirements (IDs such as `PRJ-03` or `TRM-04`). Each requirement names the test that verifies it. `docs/REQUIREMENTS.md` is the human summary. `docs/CODE_REVIEW.md` lists known issues and planned refactors.
- When you add or change behavior, update the matching FRD row (or add a new ID) and add or adjust a test in the same change. Name tests as behavior sentences, e.g. `unavailable_worktrees_never_start_a_terminal`, and cite them in the FRD.
- Mark anything a unit test can't observe (rendering, macOS integration) as **Manual**, with the check to perform.
- `PLAN.md` is the original external-Alacritty plan and is historical. Don't treat it as current.

## Architecture

Elm-style iced app: `App` state → `Message` → `App::update` returns a `Task` → `App::view`. Map this to React as: `view` = render, `Message` = action, `update` = reducer, `Task` = effect.

| Module | Responsibility |
| --- | --- |
| `main.rs` | Window setup, theme, font. Includes `tests/git_fixtures.rs` as a test module. |
| `app.rs` | All state, messages, update and view. Large; see the split proposed in `docs/CODE_REVIEW.md`. |
| `git.rs` | `discover` (resolve the checkout and its common dir) and `inventory` (`git worktree list --porcelain -z` parser). |
| `config.rs` | Bookmark store in `~/Library/Application Support/Worktree Browser/projects.json`, with atomic writes. |
| `tmux.rs` | Session naming, the private socket, backend settings, pane directory lookup. |
| `terminal.rs` | Wraps the `iced_term` widget (`Console`). |
| `terminal_config.rs` | Reads the user's Alacritty TOML (search order, imports, font, colors, bindings) and Core Text cell metrics. |
| `pull_request.rs` | `gh pr view` lookup and validation of the HTTPS PR URL. |
| `actions.rs` | Open in Finder or VS Code, copy path. |
| `create.rs`, `quit.rs` | New project and worktree creation flow; quit confirmation. |
| `row_menu.rs`, `style.rs`, `icons.rs` | Custom ⋮ menu widget, Dracula glass styles (3px radius), inline SVG icons. |

`vendor/iced_term` is a patched copy of `iced_term` 0.8.0 (via `[patch.crates-io]`). It adds things such as an explicit `cell_size` so the grid matches Alacritty's metrics. Keep patches minimal, and note why in the code you touch there.

## Rules

- **Git is the source of truth.** Never cache the worktree inventory and never depend on the Pi worktree extension. Bookmarks and display names are the only persisted state.
- **Never destroy user state.** The app must not remove or prune worktrees, delete files, kill tmux sessions, or write to the user's `~/.tmux.conf` or Alacritty config. Forgetting a project removes only the bookmark.
- **Subprocesses use argument vectors**, never shell strings. Keep paths as `PathBuf`/`OsString` end to end; paths may contain spaces, newlines or non-UTF-8 bytes. Strip Git environment overrides (`GIT_DIR` etc.) when running git.
- **No blocking work on the UI thread.** Run git, tmux, `gh` and `open` through `background(...)` or `Task::perform`, and return typed results as a `Message`. Guard async results with a request or epoch id so stale answers are ignored. Follow the existing patterns: `pr_request_id`, `refresh_id`, the `Connecting(id)` console state.
- **tmux sessions** live on the `-L worktree-browser` socket and are named `wt-` plus 10 hex digits of SHA-256(common dir, canonical worktree path). Changing the naming scheme orphans users' running sessions, so it needs a migration (see `SessionNames::migrate`).
- **Only open HTTPS URLs** with the OS opener.
- **UI:** dark Dracula theme, 3px rounded corners, compact. Don't add padding to the terminal pane, and don't let sidebar labels wrap. Check visual changes with a macOS screenshot (`screencapture`) at the default (1120×720) and minimum (820×440) window sizes.
- Errors are user-facing `String`s shown in the notice bar. Never `unwrap` or `panic` on I/O or subprocess results outside tests.
- Keep `pub` minimal. This is a single binary crate.

## Tests

- Unit tests live next to the code in `#[cfg(test)] mod tests`. Use `tests/git_fixtures.rs` (`fixture()`, `run_git()`) to build real temporary repositories. The fixture paths deliberately contain spaces and newlines.
- Drive the app state machine through `App::from_bookmarks(Store::open_path(tmp))` and `app.update(Message::…)`. Use `Task::units()` to assert which background work was started.
- Tests that need tmux must use their own `-S <tmpdir>/sock -f /dev/null` server and skip if tmux is missing. Never touch the real `worktree-browser` socket or the user's home config from a test.
- Fake external CLIs (`gh`) with small shell scripts in a tempdir, as in `pull_request.rs`.

## Collaboration

Another agent (Pi) may be editing this repository at the same time. Before broad edits, check `git status` and file modification times. Don't revert changes you didn't make, and avoid large refactors of a file someone else is actively changing.
