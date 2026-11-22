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

## Traceability summary

| Area | Requirements | Automated | Manual only |
| --- | --- | --- | --- |
| Projects | 9 | 9 | 0 |
| Worktrees | 8 | 8 | 0 |
| Terminal sessions | 12 | 10 | 2 |
| Terminal preferences | 9 | 6 | 3 |
| Actions and rename | 8 | 8 | 0 |
| Pull requests | 6 | 6 | 0 |
| Interface | 11 | 5 | 6 |
| Constraints | 7 | 5 | 2 |
| Settings and theme | 10 | 8 | 2 |
| **Total** | **80** | **65** | **15** |

Manual items are marked **Manual** in the Verification column of each document. The tests cover every requirement whose behavior lives in the state machine, Git or tmux integration, config parsing, or validation.

## How to verify

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Tests that need tmux (`migrates_tmux_session_in_place_and_preserves_a_conflicting_session`) skip themselves when tmux isn't installed. They use a private socket and never touch the user's `worktree-browser` server.
