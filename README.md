# Worktree Browser

A macOS app for working across Git worktrees. Bookmark your repositories, browse their worktrees in a sidebar, and open each one in an embedded terminal attached to its own persistent tmux session. The terminal runs Alacritty's engine through `iced_term`, so it needs neither the Alacritty app nor a separate window.

## Requirements

- macOS
- Rust 1.90.0 (pinned in `.tool-versions`)
- Git 2.41 or newer
- tmux 3.5 or newer (`brew install tmux`)
- Optional: the GitHub CLI (`gh`), logged in, to show pull request status

## Run

```sh
cargo run --locked
```

The app finds tmux on `PATH` and in the usual Homebrew locations. To use a specific binary, set `WORKTREE_BROWSER_TMUX` to its absolute path.

## How it works

- **Git is the source of truth.** The app reads worktrees from Git each time and never caches them. Changes made outside the app show up on their own.
- **Sessions outlive the app.** Each worktree has its own tmux session on a private socket (`tmux -L worktree-browser`). Switching worktrees, detaching or quitting leaves your shells running. From another terminal, `tmux -L worktree-browser list-sessions` lists them, and `tmux -L worktree-browser source-file ~/.tmux.conf` reloads your tmux config without stopping them.
- **It never destroys your work.** The app doesn't remove worktrees, delete files, kill tmux sessions, or edit your `~/.tmux.conf` or Alacritty config. Removing a project removes only its bookmark.
- **It follows your terminal setup.** The embedded terminal uses the font, colors and key bindings from your Alacritty config, and tmux reads your usual tmux config.

The app saves only its own settings, in `~/Library/Application Support/Worktree Browser/`: project bookmarks, worktree display names and the app theme.

## Documentation

- [`docs/frd/`](docs/frd/README.md): the functional requirements. They describe what the app does, feature by feature, and name the test or manual check that verifies each one.
- [`AGENTS.md`](AGENTS.md): architecture, conventions and the commands CI runs, for contributors and coding agents.
- [`SECURITY.md`](SECURITY.md): the dependency and supply-chain policy, and how to report a vulnerability.
