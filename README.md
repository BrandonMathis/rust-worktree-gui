# Worktree Browser

A standalone macOS GUI for projects you bookmark. Git supplies the worktree list; Alacritty opens shells in separate windows. The browser doesn't scan for repositories or run coding agents.

## Run

Use Rust 1.90.0 (pinned in `.tool-versions`), Git 2.41 or newer, and Alacritty installed in `/Applications`.

```sh
cargo run --locked
```

If Alacritty is installed elsewhere, set the absolute path to its executable, not the `.app` bundle:

```sh
WORKTREE_BROWSER_ALACRITTY='/path/to/Alacritty.app/Contents/MacOS/alacritty' cargo run --locked
```

## Use

1. Click **Add Project** and choose a checkout or any folder inside one. Adding a linked checkout of an already bookmarked repository selects the existing project instead of adding a duplicate.
2. Expand the project and select a worktree to see its branch or detached HEAD, path, and availability. Click **Refresh** after adding, moving, or removing worktrees outside the browser.
3. Click **Open in Alacritty** from a worktree, or click **New Session** to choose which worktree to open. The selected worktree is preselected if usable. Start Pi or another agent yourself in the new shell.

Only bookmarks are saved, in `~/Library/Application Support/Worktree Browser/projects.json`. Each bookmark stores the chosen checkout as its anchor. If the anchor disappears, use **Replace Anchor** to choose a live checkout of the same repository. **Forget Project** only removes the bookmark and its in-app rows; it doesn't touch Git, delete files, or close terminals.

Session rows record launches from the current run. They don't track running processes, and restarting the browser neither closes nor reattaches existing Alacritty windows.

## Verify

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The tests use temporary Git repositories and a stand-in terminal executable. To smoke-test the native flow, add a temporary repository with a linked worktree, add another worktree externally and refresh, then open the linked worktree twice and run `pwd` in Alacritty. Restart the browser to confirm the bookmark remains and the launch rows are gone. Run with `WORKTREE_BROWSER_ALACRITTY=/nonexistent/alacritty` to check the launch error.

See [PLAN.md](PLAN.md) for the scope. Worktree creation is left to Git in this prototype.
