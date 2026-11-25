# Rust worktree GUI — macOS prototype

This is the original external-window prototype plan. The app now uses an embedded Alacritty engine and persistent tmux sessions; see [docs/frd](docs/frd/README.md) for the current requirements.

Build a standalone project browser in Rust. Git tells us which worktrees exist and where they live. The app remembers only the projects you choose and opens coding sessions in the installed Alacritty. No Pi extension, repository scanning, or terminal fork.

## 1. Pick the UI framework

| Framework | What it teaches | Trade-off for this prototype |
| --- | --- | --- |
| [iced](https://docs.rs/iced/0.14.0/iced/) | An Elm-style flow: state, typed messages, `update`, `view` and `Task`. Widgets are built in Rust. | **Recommended.** The UI is small enough to learn Rust enums and borrowing without also learning a WebView stack. Docs are sparse in places; start with the Pocket Guide and examples for the pinned version. |
| [egui / eframe](https://docs.rs/eframe/latest/eframe/) | Immediate-mode UI: describe the interface each frame and react to widget responses. | Good fallback if iced gets in the way of a working demo. Fast for a tool like this, but the app's event flow is less explicit by default. |
| [Dioxus Desktop](https://docs.rs/dioxus-desktop/latest/dioxus_desktop/) | React-like components, hooks, and `rsx!`, rendered in the system WebView. | The shortest bridge from React, but the UI is still modeled on HTML and CSS. Worth revisiting if an embedded WebView terminal becomes a requirement. |
| [Slint](https://slint.dev/declarative-rust-gui) | Declarative `.slint` layouts and bindings connected to Rust. | Viable, but adds a separate UI language when the learning goal is to write the UI in Rust. |

In iced, think of `view(&State)` as render, `Message` as a typed reducer action, and `update(&mut State, Message)` as the reducer. `Task` handles work such as fetching Git's inventory and returns a completion message. The first exercise: build an expandable sidebar from fake data and trace one click through that loop.

## 2. Define what the app owns

- **Projects:** **Add Project** opens a macOS folder picker (`rfd`). A repo root, a subdirectory, or a linked worktree can all identify the same project. The sidebar shows only projects explicitly added; each expands to **Worktrees** and **Sessions**. **Forget Project** removes it from the app, not from disk.
- **Worktrees:** Show branch (or detached HEAD), path, and whether the directory is unavailable. Selecting one shows its details and enables **Open in Alacritty** when usable. Refresh on startup, on add, and via a **Refresh** button. Worktrees created outside the GUI appear on refresh; removing one externally doesn't delete any app data.
- **Sessions:** **New Session** lets you choose an existing worktree (preselect the current one), then opens a shell there and adds a row under the project. Start Pi or another agent yourself in that shell. Multiple sessions may use one worktree. Rows are launch records for this run, not a live process monitor; restarting the GUI restores projects, not terminals or session rows.

### Git as the source of truth

Use `std::process::Command` with separate arguments, never shell-built strings. Run Git discovery off the UI thread and return typed results to `update`.

```sh
git -C "$picked" rev-parse --show-toplevel
git -C "$picked" rev-parse --path-format=absolute --git-common-dir
git -C "$checkout" worktree list --porcelain -z
```

Use the canonical common Git directory to deduplicate the same repository picked from different checkouts. Persist the selected checkout root as the anchor for later Git calls. Parse the NUL-delimited inventory rather than splitting on whitespace or lines; keep paths as `PathBuf` and use display strings only for labels. Store project bookmarks in a small `serde` JSON file in macOS Application Support (`directories`); don't cache Git's inventory there. If a saved anchor disappears, show it as unavailable and let the user forget it or pick a live checkout to replace its anchor. Don't prune anything automatically. See [git-worktree](https://git-scm.com/docs/git-worktree) and [git-rev-parse](https://git-scm.com/docs/git-rev-parse).

### Alacritty as an external terminal

Launch the app bundle's executable directly with `Command::spawn` so it doesn't depend on the GUI's `PATH`. Use `/Applications/Alacritty.app/Contents/MacOS/alacritty` on this machine, with an override if needed.

```sh
/Applications/Alacritty.app/Contents/MacOS/alacritty --working-directory "$worktree"
```

Don't wait on the UI thread; reap child processes in the background and surface launch errors. A later explicit agent preset can pass `-e <executable> <args...>` (last on the command line); don't discover or run agent commands from repository contents. Test the login shell's `PATH` on macOS before turning on presets.

This is a standalone GUI **with an external terminal dependency**, not a plugin or a single-window terminal. [Alacritty's `--embed`](https://alacritty.org/cmd-alacritty.html) takes an X11 window ID and doesn't embed on macOS. `alacritty msg create-window` is an optional IPC approach for sharing an existing Alacritty process, not embedding. [`alacritty_terminal`](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/) is an emulation core, not a ready-made GUI widget; an integrated terminal would need its own rendering, input, clipboard, and resize work. Revisit that only if switching between windows is a real pain point.

## 3. Build in small, runnable steps

One Cargo binary crate, Rust edition 2024. Start with `src/main.rs` and `src/app.rs`; extract `git.rs`, `terminal.rs`, and `config.rs` as each boundary appears. Keep tests next to the parser and put real Git fixture tests in `tests/`. Commit `Cargo.lock`; use `cargo fmt`, `cargo clippy --all-targets`, and `cargo test`.

| Step | Demo checkpoint | Rust to learn |
| --- | --- | --- |
| 1. Render fake projects | `cargo run` opens a window with an expandable sidebar and a detail pane. | Cargo, structs, enums, shared vs. mutable borrows. |
| 2. Browse real worktrees | Pick a folder, list its worktrees, refresh, and reopen the app with the same projects. Invalid folders and Git errors are visible without crashing. | `PathBuf`, `Result`, subprocesses, background tasks, `serde`. |
| 3. Open a coding session | Pick a linked worktree, click **New Session**, run `pwd` in Alacritty to verify the directory, then start your agent. Open a second session without disturbing the first. | Argument handling, child-process ownership, separating launch records from UI widgets. |
| 4. Optional: create a worktree from the GUI | Add **Create worktree and open** as a second New Session route. Ask for a branch, destination, and base ref (default `HEAD` from the selected checkout). Run `git worktree add -b <branch> -- <destination> <base>` from the selected checkout, refresh, and launch only on success. If the terminal fails after creation, keep the worktree visible for retry. No force, remove, or prune. | Sequencing fallible operations and handling partial success. |

**Prototype is done after step 3.** Test Git parsing with spaces and newlines in paths, detached HEAD, and missing/prunable entries. In a temporary repository, verify that choosing the main checkout and a linked checkout adds just one project. On macOS, also check that a worktree added externally appears after Refresh, that a project survives app restart, and that a missing Alacritty binary yields a readable error. Leave file watching, terminal reattachment, dirty-status scans, and `.app` packaging out of the first pass.

### Local preflight

This Mac has Git 2.41.0, Xcode, and Alacritty 0.17.0 in `/Applications`. The default asdf Rust is 1.76.0, but 1.90.0 is installed and meets [iced 0.14's declared Rust 1.88 minimum](https://github.com/iced-rs/iced/blob/0.14.0/Cargo.toml). Use `ASDF_RUST_VERSION=1.90.0 cargo …` during the first build, then pin the working version in this repo's `.tool-versions`. The Git commands above were checked against a temporary repo and its linked worktree; no GUI has been built yet.
