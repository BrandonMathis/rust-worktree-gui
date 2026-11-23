# Worktree Browser — Functional Requirements (FRD)

These documents define what Worktree Browser must do. They were derived from the Pi coding sessions that built the prototype (`~/.pi/agent/sessions/--Users-brandonmathis-Dev-rust-worktree-gui--/`, 2026-11-20), cross-checked against the code, `PLAN.md` and `README.md`.

For a short, non-technical overview, read [../REQUIREMENTS.md](../REQUIREMENTS.md).

## Documents

| Doc | Area | IDs |
| --- | --- | --- |
| [FRD-01](FRD-01-projects.md) | Project bookmarks | `PRJ-*` |
| [FRD-02](FRD-02-worktrees.md) | Worktree discovery (Git as source of truth) | `WT-*` |
| [FRD-03](FRD-03-terminal-sessions.md) | Embedded terminal and tmux sessions | `TRM-*` |
| [FRD-04](FRD-04-terminal-preferences.md) | Alacritty and tmux configuration parity | `CFG-*` |
| [FRD-05](FRD-05-actions-and-rename.md) | Actions menu, row menu, rename | `ACT-*` |
| [FRD-06](FRD-06-pull-requests.md) | Pull request detection | `PR-*` |
| [FRD-07](FRD-07-interface.md) | Look, layout and sidebar behavior | `UI-*` |
| [FRD-08](FRD-08-constraints.md) | Constraints and non-functional requirements | `NFR-*` |
| [FRD-09](FRD-09-settings.md) | Settings and theme | `SET-*` |

## Conventions

- **Priority:** *Must* = required for the prototype to be considered working. *Should* = expected, but the app is usable without it. *Won't (yet)* = explicitly deferred.
- **Source:** the session turn (UTC timestamp) where the requirement was requested or decided.
- **Verification:** an automated test name (run with `cargo test --locked <name>`), or **Manual** with the check to perform. Manual items are GUI rendering or OS integration that unit tests can't observe.
- **Status:** *Implemented*, *Partial* or *Not implemented*.

## Traceability

Each FRD document is the source of truth for its requirements. Every row names the test that verifies it, or is marked **Manual** with the check to perform. Manual items are GUI rendering or OS integration that unit tests can't observe. The tests cover every requirement whose behavior lives in the state machine, Git or tmux integration, config parsing, or validation.

This index deliberately has no per-area requirement counts. Keeping totals here meant every feature branch edited the same lines, which caused merge conflicts. To count, look at the FRD tables directly.

## How to verify

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked   # includes the vendored iced_term tests
```

Tests that need tmux (`migrates_tmux_session_in_place_and_preserves_a_conflicting_session`) skip themselves when tmux isn't installed. They use a private socket and never touch the user's `worktree-browser` server.
