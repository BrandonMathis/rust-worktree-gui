# Worktree Browser

A macOS workspace for Git worktrees. Click a checkout in the sidebar to open or resume its tmux session in an embedded terminal, or make a worktree *managed* and hand it to Claude, Codex or Pi from a built-in prompt. The terminal uses Alacritty’s Rust engine via `iced_term`; it does not require the Alacritty app or open a separate window.

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

Managed worktrees also need Node.js 22.19 or newer (`brew install node`); see [Managed worktrees](#managed-worktrees). To use a specific one, set `WORKTREE_BROWSER_NODE` to its absolute path.

## Use

1. Click **+** beside **Projects**. Choose **New Project** to name a new folder, choose its parent location, and confirm **Create Project**. The app initializes Git and bookmarks the new folder. Or choose **Add Existing Repository** and select a Git checkout or a folder inside one. Adding another checkout of the same repository selects its existing bookmark.
2. Click a worktree to enter its shell. New panes default to `/bin/zsh` unless your tmux config chooses another shell or command. Reattaching leaves existing shells and commands untouched. Use `⌘C` and `⌘V` for selection copy and paste, `Ctrl+C` to interrupt, and your configured tmux prefix (then `[` for scrollback if you haven't remapped it).
3. Switch worktrees to swap sessions. **Detach** disconnects the visible terminal without stopping it. Clicking the worktree or **Connect** attaches again. Press **⌘Q** to open the quit confirmation; press **Return** to quit or **Escape** to cancel. Closing the browser leaves sessions running. Exiting the last shell in a session ends it; the next connection starts a new shell.

Drag the six-dot grip on a project header to reorder the **Projects** list. The insertion line shows where the project will land. The order is remembered when you reopen the app.

To add a worktree, hover the project name and choose **⋮ → New Worktree…**. Choose a new branch, base ref, and checkout location, then confirm **Create Worktree**. The default base is `HEAD` of the selected checkout (or the project anchor). The app refreshes the project and opens the new worktree in the terminal. A newly initialized repository needs its first commit before it can have linked worktrees.

Drag images, other files, or folders from Finder into the Worktree Browser window while a terminal is attached. Their paths are pasted into the active tmux pane, quoted when needed and separated by spaces. Dropping never presses Return or uploads the files.

With tmux mouse mode enabled, scroll over a pane to use that pane’s tmux history. Hold **Shift** to bypass tmux mouse handling and scroll the embedded terminal’s own history.

Each session is identified by its repository and canonical worktree path, not its branch name. Renaming a branch keeps its session; moving the checkout to a new path starts a different one. tmux sessions persist while their server is running, not across a reboot. The browser keeps them on a separate tmux socket, but tmux reads your normal configuration (such as `~/.tmux.conf`) when that server starts. After editing the config, reload it without stopping your sessions with `tmux -L worktree-browser source-file ~/.tmux.conf` (substitute your config path if different). Inspect or attach to sessions from another terminal with `tmux -L worktree-browser list-sessions` or `tmux -L worktree-browser attach -t <name>`.

Use **Actions** to open the active tmux pane’s directory in Finder or VS Code, or copy its path. If the pane directory isn’t available, the menu uses the worktree root. The project row’s **⋮** menu uses the saved checkout for these directory actions.

Each sidebar worktree has one icon on its left: an orange **house** for the main checkout, a cyan **branch** for a linked worktree, a red **warning** if it is unavailable, or its pull request state. For GitHub worktrees, the PR icon is **green** for open, **gray** for draft, **red** for closed, and **purple** for merged. Hover a row to see the PR number and status. Click **PR #…** next to **Actions** to open the selected worktree’s PR in your default browser. The app asks GitHub once per project (`gh pr list`) when it loads or refreshes the project, and again when you select a different worktree, then matches PRs to worktrees by branch. PRs from forks are ignored. If a branch has several PRs, an open one wins, then the newest. This uses your existing GitHub CLI login (`brew install gh`, then `gh auth login` if needed). If GitHub is unavailable or no PR is found, no PR badge appears. On the right of a row, a small badge shows its tmux session: a ring while it runs, a cyan dot when it has printed something since you last looked, and a yellow bell when it rang the bell (for example, an agent finished or is waiting for you). Opening the worktree clears it.

The window title shows the version (`v0.3.1`); a build that isn't a release also shows its branch and commit. The sidebar keeps worktree names on one line; hover a row or the worktree title for details. The sidebar icon hides the list to give the terminal the full width. Hover over the restore icon to preview and use the sidebar without resizing the terminal. Move away to dismiss the preview, or click the icon to pin the sidebar again. Hover a worktree and click **⋮** to use the same commands as its **Actions** menu without switching terminals. Choose **Rename Worktree** in either menu to set a shorter display name. Clear the field to show the branch again. Names are app-only and do not change Git, the folder, or the tmux session.

Click the **gear** at the bottom left of the sidebar to open **Settings**. On the **Theme** page, pick a preset (Dracula, Alucard, or a Dracula Pro variant such as Blade, Buffy, Lincoln, Morbius or Van Helsing), or click a color and drag on the wheel, move the brightness slider, or type a hex code. Changes preview live. **Save & Confirm** keeps them in `~/Library/Application Support/Worktree Browser/theme.json`; **Cancel**, Escape or clicking the gear again discards them. The theme colors the app itself; the terminal keeps your Alacritty colors.

Project bookmarks, worktree display names and which worktrees are managed are saved in `~/Library/Application Support/Worktree Browser/projects.json`. Worktrees added, removed or switched outside the app (including from the embedded terminal) show up within a few seconds and whenever the window gains focus; **Refresh** also re-checks every pull request. In the project row’s **⋮** menu, use **Replace Anchor** if the saved checkout is gone, or **Remove Project** to confirm removing the bookmark without touching Git, worktree files, or running tmux sessions.

## Managed worktrees

A managed worktree shows a coding agent instead of a terminal. In **New Worktree…**, set **Open with** to **Managed Agent**; or switch an existing worktree by hovering its sidebar row and choosing **⋮ → Use Managed Agent** (and back with **Use Terminal**; neither stops the other).

The first time, click **Install Agent SDKs**. The app drives each harness through its official SDK (the Claude Agent SDK, the Codex SDK and the Pi SDK), which it installs once with `npm ci` into `~/Library/Application Support/Worktree Browser/agent-bridge` (about 1 GB). They use the logins you already have: sign in with `claude`, `codex` or `pi` first.

Then pick a harness, one of its models and a thinking level, type a task and press **Return** (Shift+Return starts a new line). While the agent works, its thinking and tool calls appear as one-line rows such as `Read src/main.rs` or `Run cargo test`; click a row to see the details. Send follow-ups at any time: one sent mid-turn is queued and goes in when the turn ends. **Stop** interrupts the turn, and **New Chat** starts over with the same harness and model.

Agents keep working while you look at other worktrees. The spark on the right of a managed worktree's sidebar row turns purple while its agent works and becomes a cyan dot when a turn finished out of view. Quitting the app stops agents, and conversations aren't restored on the next launch.

There is no permission prompt in the pane. Claude follows the permission mode in your Claude settings, and a tool call that would ask is declined with a note in the conversation. Codex runs in its `workspace-write` sandbox: it can change the worktree and commit, and nothing else. Pi runs its tools without asking, as it does in a terminal.

## Terminal preferences

On startup the app looks for your Alacritty TOML in `$XDG_CONFIG_HOME/alacritty/alacritty.toml`, `$XDG_CONFIG_HOME/alacritty.toml`, `~/.config/alacritty/alacritty.toml`, and `~/.alacritty.toml`, in that order. It follows imports and honors the normal font family and size, primary, ANSI, and cursor colors, `font.offset`, and supported `keyboard.bindings` that send literal `chars` (including modified Enter, Tab, and character keys). On macOS the embedded grid uses the same Core Text metrics as Alacritty, including half-point cells on Retina displays. The GUI uses the same font family at its own interface sizes. Restart the browser after editing the file; running tmux sessions are unaffected.

The embedded widget still paints text with iced rather than Alacritty’s renderer, so antialiasing and glyph placement can differ slightly. The embedded terminal intentionally has no window padding; the `window.padding` and `dynamic_padding` settings only apply in standalone Alacritty. Window decorations, selection styling, global Option-as-Alt for printable keys, and Alacritty-specific binding actions aren’t imported. The app does not modify your home config.

## Verify

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --features rust-worktree-gui/capture -- -D warnings
cargo test --locked                                     # app and vendored iced_term tests
cargo doc --locked --no-deps --document-private-items   # with RUSTDOCFLAGS="-D warnings"
cargo deny check                                        # advisories, licenses, sources
cargo llvm-cov --locked --ignore-filename-regex vendor/ # app coverage (needs llvm-tools)
(cd agent-bridge && node --test)                         # the agent bridge's event normalizers
```

GitHub Actions runs all of these on every pull request and push to `main` (`.github/workflows/ci.yml`). Pull requests that change how the app looks also get a comment showing screenshots of the app states that show their changes, hosted as assets of a `pr-screenshots` prerelease rather than in the repository (`.github/workflows/screenshots.yml`; the states and the files each one shows are in `script/capture/states.rb`). To capture them locally: `cargo build --locked --features capture && ruby script/capture/capture.rb` (PNGs in `target/screenshots`; it uses fixture repos and a private tmux server, never your own). Lint levels live in `Cargo.toml` under `[lints]`, and `deny.toml` holds the dependency policy.

Security checks (daily RustSec audit, dependency review, CodeQL, workflow audit) run in `.github/workflows/security.yml`, and a weekly report flags dependencies that are falling out of date or support. Dependabot proposes minor and major updates only; patch releases arrive only as security fixes. [SECURITY.md](SECURITY.md) explains the policy and how to report a vulnerability.

For a native smoke test, open a linked worktree and run `pwd`, export a shell variable, then switch away and back. Check the variable both after switching and after restarting the app. Resize the window or split a tmux pane to check that the terminal redraws correctly. To check the error for a missing binary, run with `WORKTREE_BROWSER_TMUX=/nonexistent/tmux`.

See [PLAN.md](PLAN.md) for the original external-window prototype.
