# Agent instructions — Worktree Browser

A macOS desktop app in Rust ([iced](https://iced.rs)) for working across Git worktrees. What the app does is specified in `docs/frd/`; this file covers how to work on it.

The project is also a learning exercise: the owner is a React developer learning Rust. Prefer idiomatic, conventional Rust. Explain non-obvious Rust or iced patterns briefly in comments or replies, rather than writing clever code.

## Workflow

- **Start from the latest `main`.** Unless the request names another branch, run `git fetch origin main` and branch from it (`git switch -c <branch> origin/main`) before changing anything. If the checkout has uncommitted changes you didn't make, leave them alone and ask.
- **Other agents may be editing this repository at the same time.** Check `git status` and file modification times before broad edits, don't revert changes you didn't make, and avoid large refactors of a file someone else is changing.
- **Open a PR when the task is done.** Once checks pass, commit, push and run `gh pr create` without asking. Skip the PR only if questions are still open or something blocks it, and say what. Questions, research, reviews and releases don't get one.
- **End with the PR link.** If the task has a PR, finish your final response with its URL.
- Write PR descriptions with the `pr-description` skill.
- A "release" means the `new-release` skill; it commits straight to `main`.
- Project skills live in `.agents/skills/` and are committed. `.claude/skills` is a symlink to it, and `CLAUDE.md` is a symlink to this file.

## Commands

The Rust toolchain is pinned in `.tool-versions` (asdf). Always pass `--locked`.

```sh
cargo run --locked                                   # launch the app (needs tmux)
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --features rust-worktree-gui/capture -- -D warnings
cargo test --locked                                  # all tests, incl. vendor/iced_term; add a name to filter
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --document-private-items
cargo deny check                                     # advisories, licenses, sources (deny.toml)
cargo llvm-cov --locked --ignore-filename-regex vendor/  # coverage; CI enforces a line-coverage floor
cargo test --locked --features perf perf::           # the latency harness's own tests
(cd agent-bridge && node --test)                     # the agent bridge's tests (no `npm ci` needed)
ruby scripts/latency.rb                              # keystroke latency, HEAD vs origin/main (~5 min)
ruby scripts/dependency_health.rb                    # out-of-date / out-of-support report (network)
```

Ruby helper scripts have tests next to them (`scripts/test/`, `script/capture/test/`, `.agents/skills/*/test/`); run the ones for any script you change.

- Format, clippy and test must pass before you call work done. CI (`.github/workflows/`) runs all of the above on each pull request.
- `vendor/iced_term` is a workspace member and a default member, so `cargo test`, `clippy` and `llvm-cov` include it; `cargo fmt` needs `--all`.
- Lint levels live in `Cargo.toml` `[lints]` and `clippy.toml`: `unwrap`, `expect`, `panic`, `todo` and `dbg!` are denied in app code and allowed in tests. Fix the code rather than adding `#[allow]`; if an allow is truly needed, scope it narrowly and say why.
- `cargo-deny` 0.22+ needs a newer Rust than the pin, so install `cargo-deny@0.21.0` locally.
- To simulate a missing tmux, run with `WORKTREE_BROWSER_TMUX=/nonexistent/tmux`.

## Requirements are the spec

- `docs/frd/` is the only requirements document. Each requirement has an ID (such as `TRM-04`) and names the test that verifies it. Don't create a separate summary such as `docs/REQUIREMENTS.md`.
- When you add or change behavior, update the matching FRD row (or add a new ID) and add or adjust a test in the same change. Name tests as behavior sentences, e.g. `unavailable_worktrees_never_start_a_terminal`, and cite them in the FRD.
- Mark anything a unit test can't observe (rendering, macOS integration) as **Manual**, with the check to perform.
- Don't keep requirement counts or totals in `docs/frd/README.md`; every feature would edit the same lines and conflict.
- `PLAN.md` is historical. Don't treat it as current.

## Architecture

Elm-style iced app: `App` state → `Message` → `App::update` returns a `Task` → `App::view`. In React terms: `view` = render, `Message` = action, `update` = reducer, `Task` = effect. Larger UI pieces (forms, modals) are nested components with their own `Message`, `update` and an `Outcome` returned to the parent.

- `src/app/` is the state machine, split by concern; everything else in `src/` is UI-free logic or a widget. Each module's `//!` doc comment says what it owns: read those rather than relying on a list here.
- Every subprocess goes through `src/runner.rs`. Persisted files are written atomically through `src/config.rs`.
- `vendor/iced_term` is a patched copy of the `iced_term` crate (via `[patch.crates-io]`). Keep patches minimal and note why in the code you touch there.
- `agent-bridge/` is a Node.js sidecar that drives the coding-agent SDKs (they have no Rust SDK) and speaks JSON lines to the app. Its source and lockfile are compiled into the app with `include_str!`, so a change there needs a rebuild. For development, `WORKTREE_BROWSER_AGENT_BRIDGE=$PWD/agent-bridge` (after `npm ci --ignore-scripts` there) runs the checkout's copy. The app decides policy; the bridge only translates.
- Debug/CI-only code sits behind Cargo features (`perf`, `capture`) and stays out of normal builds.

## Rules

- **Never commit code review documents.** Review findings, audits and refactor wish lists stay out of the repository (in the PR, an issue or an untracked file). Record the outcome as FRD rows and tests instead.
- **Git is the source of truth.** Never cache the worktree inventory, and don't depend on third-party worktree metadata. Persist only user preferences, never derived state. The one exception is `conversations.json`: the harness's id for each agent conversation a managed worktree can resume, never its messages, which stay in the harness's own session files and are read back through the bridge.
- **Never destroy user state.** The app must not remove or prune worktrees, delete files, kill tmux sessions, or write to the user's tmux or Alacritty config. Removing a project removes only the bookmark.
- **tmux session names must stay stable.** Changing how they're derived orphans users' running sessions, so it needs a migration (see `src/tmux.rs`).
- **Subprocesses use argument vectors**, never shell strings. Keep paths as `PathBuf`/`OsString` end to end; they may contain spaces, newlines or non-UTF-8 bytes. Strip Git environment overrides (`GIT_DIR` etc.) when running git.
- **No blocking work on the UI thread.** Run subprocesses and I/O in a `Task` and return a typed result as a `Message`. Guard async results with a request or epoch id so stale answers are ignored; follow the existing patterns.
- **Only open HTTPS URLs** with the OS opener.
- **Agents go through the bridge.** Talk to a coding agent only through its SDK in `agent-bridge/`, never by scraping a terminal. Keep the worktree path out of JSON (it is the bridge's working directory), guard bridge events with the session id, and never install the SDKs without the user asking.
- **Errors** are user-facing `String`s shown in the app. Never `unwrap` or `panic` on I/O or subprocess results outside tests.
- **Styling:** colors come from the `Palette` passed into each view and style (see `src/style.rs`); never hard-code a color. Look and layout requirements are in the FRD (`UI-*`).
- Keep `pub` minimal. This is a single binary crate.

## Tests

- Test the behavior a user or an FRD requirement depends on. Don't add tests to raise coverage, exercise trivial getters or library code, or pin implementation details. Coverage is a signal for finding untested behavior, not a target.
- Unit tests live next to the code in `#[cfg(test)] mod tests`. Use `tests/git_fixtures.rs` to build real temporary repositories; its paths deliberately contain spaces and newlines.
- Drive the app state machine through `App::update` with a temporary bookmark store, and use `Task::units()` to assert which background work started. See `src/app/tests.rs` for the helpers.
- Tests that need tmux use their own `-S <tmpdir>/sock -f /dev/null` server and skip if tmux is missing. Never touch the app's real tmux socket or the user's home config from a test.
- Fake external CLIs (`gh`, the bridge's `node`) with small shell scripts in a tempdir, as existing tests do. Tests never run a real coding agent or install an SDK.

## Visual changes and performance

- **Screenshots.** Check visual changes at the default and minimum window sizes (`src/main.rs`). CI posts screenshots on PRs of the states whose `covers` match the changed files (`script/capture/states.rb`). When you add a UI feature no state shows, add a state for it in the same PR; if the app needs a new startup hook to reach that view, add it in `src/capture.rs`. Run locally with `cargo build --locked --features capture && ruby script/capture/capture.rb` (`--only <state>,...` to pick states; needs Screen Recording permission). It uses fixture repos, a private tmux server and an isolated `HOME`. Never commit the PNGs or push them to a branch.
- **Keystroke latency.** CI fails a PR that makes typing into the terminal significantly slower than its base. Run `ruby scripts/latency.rb` before and after changes to input handling, `App::view`/`update`, the terminal widget or drawing (`--base-ref <rev>` to compare with another revision). Only the comparison means anything; absolute numbers depend on the machine. When you add a keyboard-driven feature, add a scenario in `src/perf/scenarios.rs`. Never commit `target/latency/`.

## Dependencies and supply chain

Read `SECURITY.md` before changing `.github/dependabot.yml`, `deny.toml`, `agent-bridge/package.json` or a workflow.

- Never express the Dependabot patch policy as an `ignore` on `semver-patch`: `ignore` also blocks security updates.
- Pin GitHub Actions to a full commit SHA with a `# vX.Y.Z` comment, and pin tools CI installs to a version. Give each workflow `permissions: {}` and grant per job. Use `persist-credentials: false` on checkout.
- Add a crate only when it's needed, prefer widely used crates with recent releases, and keep `cargo deny check` passing. A new `[advisories] ignore` entry in `deny.toml` needs a `reason`.
- Helper scripts are Ruby using only the standard library, so CI needs no extra installs.
- `agent-bridge/` pins its SDKs to exact versions and users install from its lockfile. Add an npm dependency there only for an agent SDK; keep everything else to Node's standard library.
