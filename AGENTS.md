# Agent instructions — Worktree Browser

A macOS desktop app in Rust (iced 0.14) for working across Git worktrees. Bookmark repositories, browse their worktrees, and open an embedded terminal (Alacritty's engine via `iced_term`) attached to a persistent per-worktree tmux session, or a *managed* worktree: an agent pane that drives Claude, Codex or Pi. See `docs/frd/` for the required behavior.

The project is also a learning exercise: the owner is a React developer learning Rust. Prefer idiomatic, conventional Rust. Explain non-obvious Rust or iced patterns briefly in comments or replies, rather than writing clever code.

## Commands

Rust 1.90.0 is pinned in `.tool-versions` (asdf). Always pass `--locked`.

```sh
cargo run --locked                                   # launch the app (needs tmux)
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --features rust-worktree-gui/capture -- -D warnings
cargo test --locked                                  # all tests, incl. vendor/iced_term; add a name to filter
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --document-private-items
cargo deny check                                     # advisories, licenses, sources (deny.toml)
cargo llvm-cov --locked --ignore-filename-regex vendor/  # app coverage; CI enforces a line-coverage floor
ruby scripts/dependency_health.rb                    # out-of-date / out-of-support report (network)
ruby scripts/test/dependency_health_test.rb          # tests for that report
ruby script/capture/test/states_test.rb              # tests for which screenshot states a PR gets
ruby .agents/skills/new-release/test/version_bump_test.rb  # tests for how a release picks its version bump (local; not in CI)
ruby scripts/latency.rb                              # keystroke latency, HEAD vs origin/main (builds both; ~5 min)
ruby scripts/test/latency_test.rb                    # tests for the latency comparison rules
cargo test --locked --features perf perf::           # tests for the latency harness
(cd agent-bridge && node --test)                     # the agent bridge's normalizers (no `npm ci` needed)
```

`vendor/iced_term` is a workspace member and a default member, so `cargo test`, `cargo clippy` and `cargo llvm-cov` run its tests and lints too; `cargo fmt` needs `--all` to include it. Format, clippy and test must pass before you call work done. CI (`.github/workflows/ci.yml`) runs every command above on each pull request. Lint levels live in `Cargo.toml` `[lints]`: `unwrap`, `expect`, `panic`, `todo` and `dbg!` are denied in app code and allowed in tests (`clippy.toml`). Fix the code rather than adding `#[allow]`; if an allow is truly needed, scope it narrowly and say why. `cargo-deny` 0.22+ needs a newer Rust than the pin, so install `cargo-deny@0.21.0` locally. To test a missing tmux, run with `WORKTREE_BROWSER_TMUX=/nonexistent/tmux`.

## Keystroke latency

`.github/workflows/latency.yml` measures how long a key typed into the terminal takes to change the pixels on screen, on every PR that changes `src/`, `vendor/`, Cargo files or the harness, and fails the check when the PR is significantly slower than its base (NFR-11). The harness is built into the app behind `--features perf` (`src/perf/`, off in normal builds): it runs the real `App` headlessly, presses keys through iced's widget tree, and renders every frame offscreen until its pixels change. The terminal runs a tiny line editor (`src/perf/echo.rs`) instead of tmux, so the number is the app's own pipeline. `scripts/latency.rb` builds base and head and runs them in turns on one machine, which is the only fair comparison. The rules (15% or 3σ, only when significant and seen in every round) are in that script.

Run it before and after a change that touches input handling, `App::view`/`update`, the terminal widget or drawing: `ruby scripts/latency.rb` (or `--base-ref <rev>`; `--from target/latency` re-reads a run). Absolute numbers depend on the machine and its load; only the comparison means anything. When you add a keyboard-driven feature, add a scenario in `src/perf/scenarios.rs`. Results land in `target/latency/`; never commit them.

## Requirements are the spec

- `docs/frd/` holds the functional requirements (IDs such as `PRJ-03` or `TRM-04`). Each requirement names the test that verifies it. There is no separate human summary; the FRD is the only requirements document, so don't recreate `docs/REQUIREMENTS.md`.
- When you add or change behavior, update the matching FRD row (or add a new ID) and add or adjust a test in the same change. Name tests as behavior sentences, e.g. `unavailable_worktrees_never_start_a_terminal`, and cite them in the FRD.
- Don't keep requirement counts or totals in `docs/frd/README.md`. Every feature would edit the same lines and cause merge conflicts.
- Mark anything a unit test can't observe (rendering, macOS integration) as **Manual**, with the check to perform.
- `PLAN.md` is the original external-Alacritty plan and is historical. Don't treat it as current.

## Screenshots

`.github/workflows/screenshots.yml` posts screenshots on PRs that change how the app looks, of only the states that show the files the PR changes. Dependency updates (`Cargo.toml`, `Cargo.lock`) and changes no state shows get none. The PR comment shows the PNGs inline from assets of the `pr-screenshots` prerelease, because a comment can only show an image anyone can fetch (workflow artifacts need a sign-in); never commit them or push them to a branch. It builds with `--features capture` (startup hooks in `src/capture.rs`, off in normal builds) and runs `script/capture/capture.rb`, which creates fixture repos, a private tmux server with vim and shell splits, and an isolated `HOME`. It never touches your bookmarks, tmux sessions or dotfiles. Run it locally to check a visual change (a window appears briefly; your terminal needs Screen Recording permission):

```sh
cargo build --locked --features capture && ruby script/capture/capture.rb   # PNGs in target/screenshots
```

The states live in `script/capture/states.rb`. Each lists the files it shows (`covers`), and a PR gets screenshots of the states whose `covers` match a file it changes (`script/capture/select.rb`; tests in `script/capture/test/states_test.rb`). So the screenshots show the feature a PR adds or changes: when you add a UI feature that no state shows, add a state for it in the same PR, with `covers` naming the feature's files. If the app needs a new startup hook to reach that view, add a variant to `WORKTREE_BROWSER_CAPTURE_OPEN` in `src/capture.rs`. Capture one or more states locally with `--only 04-settings,01-workspace`.

## Architecture

Elm-style iced app: `App` state → `Message` → `App::update` returns a `Task` → `App::view`. Map this to React as: `view` = render, `Message` = action, `update` = reducer, `Task` = effect.

| Module | Responsibility |
| --- | --- |
| `main.rs` | Window setup, theme, font. Includes `tests/git_fixtures.rs` as a test module. |
| `app/mod.rs` | App state (`App`, `Project`, `Selection`), `Message`, construction, subscriptions, `on_selection_changed()`, `background(...)`. |
| `app/update.rs` | `App::update`: one arm per message, each calling a named handler. Small handlers (quit, picker, menus, rename, Settings). |
| `app/projects.rs` | Bookmarks, refresh from Git (keeps the last good inventory on error), selection reconcile, removal, display names. |
| `app/pull_requests.rs` | PR lookups per worktree and the header badge. |
| `app/sessions.rs` | tmux session badges (live, new output, bell) from a background `list-windows -a` poll. |
| `app/auto_refresh.rs`, `watch.rs` | Automatic refresh on window focus and when a repository's Git metadata fingerprint changes (polled with `stat`). |
| `app/terminal_pane.rs` | `ConsoleState`, connect/prepared, switching the running client to another worktree (`switch-client`, with a full-connect fallback), and the header status and pane. |
| `app/unavailable.rs` | The modal listing a project's unavailable worktrees (hidden from the sidebar), opened from the project's ⋮ menu. |
| `app/creation.rs` | Opens and submits the New Project / New Worktree form (`create::Draft` is the nested component). |
| `app/managed.rs` | Managed worktrees: the bridge install state, per-harness model lists, one `Session` per worktree (composer, transcript, follow-up queue, `Run` state), and the agent pane. |
| `app/sidebar.rs`, `app/view.rs`, `app/widgets.rs` | Sidebar (rows, icons, hover preview); window layout, workspace and header; shared view helpers. |
| `app/tests.rs` | State-machine tests driven through `App::update`. |
| `git.rs` | `discover` (resolve the checkout and its common dir) and `inventory` (`git worktree list --porcelain -z` parser). |
| `config.rs` | Bookmark store in `~/Library/Application Support/Worktree Browser/projects.json`, and the shared atomic JSON write. |
| `theme.rs` | The palette (eleven Dracula colors), presets, hex/HSV conversion, and the saved `theme.json`. |
| `settings.rs`, `color_wheel.rs` | Full-window Settings modal (a nested Elm component returning an `Outcome`); canvas hue/saturation wheel. |
| `tmux.rs` | Session naming, the private socket, backend settings, pane directory lookup, read-only session activity. |
| `terminal.rs` | Wraps the `iced_term` widget (`Console`): the worktree it shows and its tmux client's pid. |
| `terminal_config.rs` | Reads the user's Alacritty TOML (search order, imports, font, colors, bindings) and Core Text cell metrics. |
| `pull_request.rs` | One `gh pr list` per project, matching PRs to worktrees by branch, validation of the HTTPS PR URL, and the per-project `Badges`. |
| `runner.rs` | Every subprocess: tool lookup (`PATH` + Homebrew), removal of the `GIT_*` overrides, per-call timeouts. |
| `actions.rs` | Open in Finder or VS Code, copy path, open an HTTPS link. |
| `agent.rs` | The harnesses, the bridge's wire format (`Event`, `Command`), the `Transcript`, and the one-line tool and thinking summaries. |
| `agent_bridge.rs` | Runs the Node.js sidecar: copies the bundled bridge to Application Support, installs its SDKs with `npm ci`, lists models, and streams a session's events. |
| `create.rs` | Creation form state, a nested component (`Message`, `update`, `Outcome`) like `settings.rs`. |
| `confirmation.rs`, `quit.rs` | Native confirmation sheets and the macOS quit shortcut. |
| `backdrop.rs` | The macOS behind-window blur: an `NSVisualEffectView` under the content, so it ends on the window's rounded corners (winit's `blur` doesn't). |
| `perf/` | Keystroke-latency harness (`--features perf` only): headless event loop around `App`, scenarios, and the line editor the terminal runs. `app/perf.rs` holds its `App` hooks. |
| `row_menu.rs`, `style.rs`, `icons.rs` | Custom ⋮ menu widget, glass styles drawn from the active `Palette` (3px radius), inline SVG icons. |

`agent-bridge/` is the sidecar for managed worktrees. Claude, Codex and Pi have no Rust SDK, so `bridge.mjs` drives their official TypeScript SDKs and speaks JSON lines to `agent_bridge.rs`. `normalize.mjs` turns each SDK's events into the app's `agent::Event`s and imports no SDK, so `node --test` runs without `npm ci`. Both files and the lockfile are compiled into the app (`include_str!`), so a change there needs a rebuild; a changed lockfile makes installed copies ask to install again. For development, `WORKTREE_BROWSER_AGENT_BRIDGE=$PWD/agent-bridge` (after `npm ci --ignore-scripts` there) runs the checkout's copy instead of the installed one. The app decides policy (queueing follow-ups, what a row says); the bridge only translates.

`vendor/iced_term` is a patched copy of `iced_term` 0.8.0 (via `[patch.crates-io]`). It adds things such as an explicit `cell_size` so the grid matches Alacritty's metrics. Keep patches minimal, and note why in the code you touch there.

## Rules

- **Never commit code review documents.** Review findings, audits and refactor wish lists (for example a `CODE_REVIEW.md`) must not be checked into source control. Keep them out of the repository (in the PR, an issue, or an untracked local file) and record the outcome as FRD rows and tests instead.
- **Git is the source of truth.** Never cache the worktree inventory and never depend on the Pi worktree extension. Bookmarks, display names, which worktrees are managed, and the theme palette are the only persisted state. Agent conversations are not saved.
- **Never destroy user state.** The app must not remove or prune worktrees, delete files, kill tmux sessions, or write to the user's `~/.tmux.conf` or Alacritty config. Removing a project removes only the bookmark.
- **Subprocesses use argument vectors**, never shell strings. Keep paths as `PathBuf`/`OsString` end to end; paths may contain spaces, newlines or non-UTF-8 bytes. Strip Git environment overrides (`GIT_DIR` etc.) when running git.
- **No blocking work on the UI thread.** Run git, tmux, `gh` and `open` through `background(...)` or `Task::perform`, and return typed results as a `Message`. Guard async results with a request or epoch id so stale answers are ignored. Follow the existing patterns: `pr_request_id`, `refresh_id`, the `Connecting(id)` console state.
- **tmux sessions** live on the `-L worktree-browser` socket and are named `wt-` plus 10 hex digits of SHA-256(common dir, canonical worktree path). Changing the naming scheme orphans users' running sessions, so it needs a migration (see `SessionNames::migrate`).
- **Only open HTTPS URLs** with the OS opener (`actions::open_https` for links in agent replies).
- **Agents go through the bridge.** Talk to a harness only through its SDK in `agent-bridge/`, never by scraping a terminal. Keep the worktree path out of JSON (it is the bridge's working directory), guard bridge events with the session id, and never install the SDKs without the user asking.
- **UI:** dark Dracula theme by default, 3px rounded corners, compact. Keep the terminal pane's padding to its 3px inset (UI-06), and don't let sidebar labels wrap. Check visual changes with a macOS screenshot (`screencapture`) at the default (1120×720) and minimum (820×440) window sizes. Colors come from the `Palette` passed into each view and style (`style::panel(p, Panel::Glass)`, `p.muted()`); never hard-code a color in a view.
- Errors are user-facing `String`s shown in the notice bar. Never `unwrap` or `panic` on I/O or subprocess results outside tests.
- Keep `pub` minimal. This is a single binary crate.

## Dependencies and supply chain

Read `SECURITY.md` before changing `.github/dependabot.yml`, `deny.toml` or a workflow.

- Dependabot opens version PRs only for minor and major releases (`allow: update-types`). Patch releases arrive only as security updates. Never express this as an `ignore` on `semver-patch`: `ignore` also blocks security updates.
- Pin GitHub Actions to a full commit SHA with a `# vX.Y.Z` comment, and pin tools that CI installs to a version. Give each workflow `permissions: {}` and grant per job. Use `persist-credentials: false` on checkout.
- Add a crate only when it's needed. Prefer widely used crates with recent releases, and check that `cargo deny check` still passes. A new `[advisories] ignore` entry in `deny.toml` needs a `reason`.
- Helper scripts are Ruby using only the standard library (`scripts/`), so CI needs no extra installs.
- `agent-bridge/package.json` pins the three harness SDKs to exact versions, and `package-lock.json` is what users install (`npm ci --ignore-scripts`). Add an npm dependency there only for a harness SDK; keep everything else to Node's standard library.

## Tests

- Write tests only to verify app functionality: the behavior a user or an FRD requirement depends on. Don't add tests to raise the coverage number, to exercise trivial getters or library code, or to pin implementation details. Coverage is a signal for finding untested behavior, not a target.
- Unit tests live next to the code in `#[cfg(test)] mod tests`. Use `tests/git_fixtures.rs` (`fixture()`, `run_git()`) to build real temporary repositories. The fixture paths deliberately contain spaces and newlines.
- Drive the app state machine through `App::from_bookmarks(Store::open_path(tmp))` and `app.update(Message::…)`. Use `Task::units()` to assert which background work was started.
- Tests that need tmux must use their own `-S <tmpdir>/sock -f /dev/null` server and skip if tmux is missing. Never touch the real `worktree-browser` socket or the user's home config from a test.
- Fake external CLIs (`gh`) with small shell scripts in a tempdir, as in `pull_request.rs`. The agent bridge is faked the same way (a shell script standing in for `node`, in `agent_bridge.rs`); tests never run a real harness or install an SDK. For app tests, `Session::attach` gives a session a fake bridge and returns the commands sent to it.

## Collaboration

**Start every session from the latest `main`.** Unless the request names another branch, assume the work is meant for the newest `main`. Before you change anything, run `git fetch origin main` and create your branch from `origin/main` (`git switch -c <branch> origin/main`). Don't build on a stale local `main` or on whatever branch the worktree happens to have checked out. If the checkout has uncommitted changes you didn't make, leave them alone and ask.

**Finish work with a pull request.** Once format, clippy and tests pass, commit, push the branch, and open a PR with `gh pr create`. You don't need to ask first. Skip the PR only when something blocks it: the work needs more input, a decision or guidance from the owner, or it can't be made to pass the checks. When you skip it, say why. Requests that change nothing (questions, research, reviews) and releases, which commit straight to `main`, don't get one.

Another agent (Pi) may be editing this repository at the same time. Before broad edits, check `git status` and file modification times. Don't revert changes you didn't make, and avoid large refactors of a file someone else is actively changing.

Write pull request descriptions as `.agents/skills/pr-description/SKILL.md` describes: a "What changed" section and a "Why" section, with no test plans or lists of tests added.

A "release" means `.agents/skills/new-release/SKILL.md`: it picks the next major/minor/patch version from the changes since the last release, commits the bump to main, tags it `vX.Y.Z`, pushes both, and installs the build as `/Applications/Worktree Browser.app`. There is no GitHub Release. Project skills live in `.agents/skills/` and are committed, so every worktree has them. `.claude/skills` is a symlink to that folder, because Claude Code reads skills from there.
