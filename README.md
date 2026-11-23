# Worktree Browser

A macOS workspace for Git worktrees. Click a checkout in the sidebar to open or resume its tmux session in an embedded terminal. The terminal uses Alacritty’s Rust engine via `iced_term`; it does not require the Alacritty app or open a separate window.

## Run

Use Rust 1.90.0 (pinned in `.tool-versions`), Git 2.41 or newer, and tmux 3.5 or newer.

```sh
brew install tmux  # if needed
cargo run --locked
```

The browser checks `PATH` and common macOS package manager locations. To use a specific tmux binary, set `WORKTREE_BROWSER_TMUX` to its absolute path:

```sh
WORKTREE_BROWSER_TMUX=/opt/homebrew/bin/tmux cargo run --locked
```

## Use

1. Click **+** beside **Projects**. Choose **New Project** to name a new folder, choose its parent location, and confirm **Create Project**. The app initializes Git and bookmarks the new folder. Or choose **Add Existing Repository** and select a Git checkout or a folder inside one. Adding another checkout of the same repository selects its existing bookmark.
2. Click a worktree to enter its shell. New panes default to `/bin/zsh` unless your tmux config chooses another shell or command. Reattaching leaves existing shells and commands untouched. Use `⌘C` and `⌘V` for selection copy and paste, `Ctrl+C` to interrupt, and your configured tmux prefix (then `[` for scrollback if you haven't remapped it).
3. Switch worktrees to swap sessions. **Detach** disconnects the visible terminal without stopping it. Clicking the worktree or **Connect** attaches again. Press **⌘Q** to open the quit confirmation; press **Return** to quit or **Escape** to cancel. Closing the browser leaves sessions running. Exiting the last shell in a session ends it; the next connection starts a new shell.

To add a worktree, hover the project name and choose **⋮ → New Worktree…**. Choose a new branch, base ref, and checkout location, then confirm **Create Worktree**. The default base is `HEAD` of the selected checkout (or the project anchor). The app refreshes the project and opens the new worktree in the terminal. A newly initialized repository needs its first commit before it can have linked worktrees.

With tmux mouse mode enabled, scroll over a pane to use that pane’s tmux history. Hold **Shift** to bypass tmux mouse handling and scroll the embedded terminal’s own history.

Each session is identified by its repository and canonical worktree path, not its branch name. Renaming a branch keeps its session; moving the checkout to a new path starts a different one. tmux sessions persist while their server is running, not across a reboot. The browser keeps them on a separate tmux socket, but tmux reads your normal configuration (such as `~/.tmux.conf`) when that server starts. After editing the config, reload it without stopping your sessions with `tmux -L worktree-browser source-file ~/.tmux.conf` (substitute your config path if different). Inspect or attach to sessions from another terminal with `tmux -L worktree-browser list-sessions` or `tmux -L worktree-browser attach -t <name>`.

Use **Actions** to open the active tmux pane’s directory in Finder or VS Code, or copy its path. If the pane directory isn’t available, the menu uses the worktree root. The project row’s **⋮** menu uses the saved checkout for these directory actions.

Each sidebar worktree has one icon on its left: an orange **house** for the main checkout, a cyan **branch** for a linked worktree, a red **warning** if it is unavailable, or its pull request state. For GitHub worktrees, the PR icon is **green** for open, **gray** for draft, **red** for closed, and **purple** for merged. Hover a row to see the PR number and status. Click **PR #…** next to **Actions** to open the selected worktree’s PR in your default browser. The app checks worktrees when it loads or refreshes the project, and rechecks a worktree when selected. This uses your existing GitHub CLI login (`brew install gh`, then `gh auth login` if needed). If GitHub is unavailable or no PR is found, no PR badge appears.

The sidebar keeps worktree names on one line; hover a row or the worktree title for details. The sidebar icon hides the list to give the terminal the full width. Hover over the restore icon to preview and use the sidebar without resizing the terminal. Move away to dismiss the preview, or click the icon to pin the sidebar again. Hover a worktree and click **⋮** to use the same commands as its **Actions** menu without switching terminals. Choose **Rename Worktree** in either menu to set a shorter display name. Clear the field to show the branch again. Names are app-only and do not change Git, the folder, or the tmux session.

Click the **gear** at the bottom left of the sidebar to open **Settings**. On the **Theme** page, pick a preset (Dracula, Alucard, or a Dracula Pro variant such as Blade, Buffy, Lincoln, Morbius or Van Helsing), or click a color and drag on the wheel, move the brightness slider, or type a hex code. Changes preview live. **Save & Confirm** keeps them in `~/Library/Application Support/Worktree Browser/theme.json`; **Cancel**, Escape or clicking the gear again discards them. The theme colors the app itself; the terminal keeps your Alacritty colors.

Project bookmarks and worktree display names are saved in `~/Library/Application Support/Worktree Browser/projects.json`. Click **Refresh** after outside Git changes. In the project row’s **⋮** menu, use **Replace Anchor** if the saved checkout is gone, or **Forget Project** to remove the bookmark without touching Git or stopping its sessions.

## Terminal preferences

On startup the app looks for your Alacritty TOML in `$XDG_CONFIG_HOME/alacritty/alacritty.toml`, `$XDG_CONFIG_HOME/alacritty.toml`, `~/.config/alacritty/alacritty.toml`, and `~/.alacritty.toml`, in that order. It follows imports and honors the normal font family and size, primary, ANSI, and cursor colors, `font.offset`, and supported `keyboard.bindings` that send literal `chars` (including modified Enter, Tab, and character keys). On macOS the embedded grid uses the same Core Text metrics as Alacritty, including half-point cells on Retina displays. The GUI uses the same font family at its own interface sizes. Restart the browser after editing the file; running tmux sessions are unaffected.

The embedded widget still paints text with iced rather than Alacritty’s renderer, so antialiasing and glyph placement can differ slightly. The embedded terminal intentionally has no window padding; the `window.padding` and `dynamic_padding` settings only apply in standalone Alacritty. Window decorations, selection styling, global Option-as-Alt for printable keys, and Alacritty-specific binding actions aren’t imported. The app does not modify your home config.

## Verify

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo doc --locked --no-deps --document-private-items   # with RUSTDOCFLAGS="-D warnings"
cargo deny check                                        # advisories, licenses, sources
cargo llvm-cov --locked                                 # coverage (needs llvm-tools)
```

GitHub Actions runs all of these on every pull request and push to `main` (`.github/workflows/ci.yml`). Lint levels live in `Cargo.toml` under `[lints]`, and `deny.toml` holds the dependency policy.

Security checks (daily RustSec audit, dependency review, CodeQL, workflow audit) run in `.github/workflows/security.yml`, and a weekly report flags dependencies that are falling out of date or support. Dependabot proposes minor and major updates only; patch releases arrive only as security fixes. [SECURITY.md](SECURITY.md) explains the policy and how to report a vulnerability.

For a native smoke test, open a linked worktree and run `pwd`, export a shell variable, then switch away and back. Check the variable both after switching and after restarting the app. Resize the window or split a tmux pane to check that the terminal redraws correctly. To check the error for a missing binary, run with `WORKTREE_BROWSER_TMUX=/nonexistent/tmux`.

See [PLAN.md](PLAN.md) for the original external-window prototype.
