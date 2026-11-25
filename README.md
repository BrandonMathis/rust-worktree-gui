# Worktree Browser

A macOS app for working across Git worktrees. Bookmark repositories, browse their worktrees in a sidebar, and click one to open its own persistent tmux session in an embedded terminal. The terminal runs Alacritty's engine (via `iced_term`), so it doesn't need the Alacritty app, and it picks up your Alacritty font, colors and key bindings. A worktree can also be *managed*: its pane is a small agent harness for Claude, Codex or Pi instead of a terminal.

Requirements live in [`docs/frd/`](docs/frd/README.md).

## Run

Needs Rust 1.90.0 (pinned in `.tool-versions`), Git 2.41+ and tmux 3.5+.

```sh
brew install tmux   # if needed
cargo run --locked
```

To use a specific tmux binary, set `WORKTREE_BROWSER_TMUX` to its absolute path.

To free resources, the app closes idle tmux sessions it created: after 1 day without input if no bookmarked project lists the worktree, otherwise after 7 days. Attached sessions are never closed. When you reopen that worktree, the notice bar explains why its shell is new.

Managed worktrees also need Node.js 22.19 or newer (`brew install node`); set `WORKTREE_BROWSER_NODE` to use a specific one. The first managed worktree asks to install the agent SDKs (about 1 GB, into `~/Library/Application Support/Worktree Browser/agent-bridge`).

## Develop

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --features rust-worktree-gui/capture -- -D warnings
cargo test --locked
(cd agent-bridge && node --test)   # the agent bridge's event normalizers
```

CI (`.github/workflows/ci.yml`) runs these plus docs, `cargo deny` and coverage on every pull request. Contributor and agent guidelines are in [AGENTS.md](AGENTS.md); security policy is in [SECURITY.md](SECURITY.md).
