# FRD-08 — Constraints and non-functional requirements

**Source:** initial request (2026-11-20T21:47Z): *"we're going to build in Rust… We are running on macOS… I specifically want to use Rust. I want to use Rust common conventions."* Project goals: (1) a small running Rust prototype, (2) learn Rust UI development coming from React.

| ID | Priority | Requirement | Verification | Status |
| --- | --- | --- | --- | --- |
| NFR-01 | Must | A standalone Rust application (edition 2024, Rust 1.90 pinned in `.tool-versions`) using iced 0.14 with the Elm-style `State → Message → update → view` architecture. It isn't a plugin to Pi or Alacritty. | Build: `cargo build --locked` | Implemented |
| NFR-02 | Must | Targets macOS. Required external tools: Git ≥ 2.41 and tmux ≥ 3.5. Optional: `gh` (PR badge) and VS Code (Open in VS Code). | Manual | Implemented |
| NFR-03 | Must | Subprocesses use argument vectors, never shell-built strings. Paths are kept as `PathBuf`/bytes end to end. | `preserves_paths_and_parses_flags`, `isolates_sessions_without_overriding_home_config_and_passes_a_literal_cwd` | Implemented |
| NFR-04 | Must | Git, tmux, `gh` and `open` run off the UI thread and return typed results as messages. | `switching_worktrees_ignores_a_stale_connection_result`, `stale_refresh_results_are_ignored_after_replacing_the_anchor`, `pr_badge_belongs_to_the_selected_worktree_only` | Implemented |
| NFR-05 | Must | The app never modifies Git state (no add, remove or prune), user configs, or tmux sessions other than its own name migration. | `deduplicates_linked_checkouts_and_persists_anchor_replacement` (forget leaves Git intact); Manual (code review) | Implemented |
| NFR-06 | Must | Only HTTPS URLs are passed to the OS opener. | `rejects_unsafe_or_mismatched_pull_request_urls` | Implemented |
| NFR-07 | Must | `cargo fmt --check`, `cargo clippy --all-targets -D warnings` and `cargo test` pass. | CI command set in [README](README.md#how-to-verify) | Implemented |

## Deferred (Won't yet)

- Creating worktrees from the GUI (`PLAN.md` step 4).
- `.app` bundle packaging.
- File watching and automatic refresh.
- Launching agents automatically (presets).
