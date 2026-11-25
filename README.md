# Worktree Browser

A macOS app for working across Git worktrees. Bookmark repositories, browse their worktrees in a sidebar, and click one to open its own persistent tmux session in an embedded terminal. The terminal runs Alacritty's engine (via `iced_term`), so it doesn't need the Alacritty app, and it picks up your Alacritty font, colors and key bindings.

Requirements live in [`docs/frd/`](docs/frd/README.md).

## Run

Needs Rust 1.90.0 (pinned in `.tool-versions`), Git 2.41+ and tmux 3.5+.

```sh
brew install tmux   # if needed
cargo run --locked
```

To use a specific tmux binary, set `WORKTREE_BROWSER_TMUX` to its absolute path.

## Develop

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --features rust-worktree-gui/capture -- -D warnings
cargo test --locked
```

CI (`.github/workflows/ci.yml`) runs these plus docs, `cargo deny` and coverage on every pull request. Contributor and agent guidelines are in [AGENTS.md](AGENTS.md); security policy is in [SECURITY.md](SECURITY.md).
