# Worktree Browser — Requirements Summary

*Last updated 2026-11-20. Full functional requirements, with IDs and test traceability, are in [docs/frd/](frd/README.md).*

## What it is

Worktree Browser is a small macOS app, written in Rust, for working across Git worktrees. You bookmark the repositories you care about. The app lists each repository's worktrees, and clicking one opens a terminal for that worktree right inside the window. The terminal is a persistent tmux session, so whatever you were running, such as a coding agent, is still there when you come back.

It was built for two reasons: to have a lightweight "agents workspace" (similar to VS Code's agent view) that doesn't depend on the Pi worktree extension, and to learn Rust UI development coming from React.

## What it must do

**Projects you choose**
- Add a repository by picking any folder inside it. Picking the same repository twice, from any checkout, doesn't create a duplicate.
- Bookmarks are remembered between launches. Forgetting a project removes only the bookmark, never files, Git data or running sessions.
- If the saved folder goes away, the project is shown as unavailable and you can point it at another checkout of the same repository.

**Worktrees straight from Git**
- The list of worktrees comes from Git alone. Worktrees created or deleted elsewhere appear or disappear when you click Refresh.
- Each worktree shows its branch (or "Detached HEAD"). Missing or prunable worktrees are flagged and can't open a terminal.
- The app never creates, removes or prunes worktrees.

**A terminal per worktree**
- Clicking a worktree opens an embedded terminal using Alacritty's engine, not a separate window.
- Each worktree has its own tmux session, which is resumed every time you come back. Switching worktrees, hiding the sidebar or quitting the app never stops your shell.
- Sessions use a private tmux socket but your own `~/.tmux.conf`. New shells are zsh.
- Session names are short (`wt-` plus 10 characters) and stay the same if you rename the branch.

**Feels like your terminal**
- The embedded terminal uses your Alacritty font, size, colors and literal-text key bindings, with the same cell size as standalone Alacritty.
- The app's own text uses the same font. Your Alacritty and tmux configs are read, never changed.

**Quick actions**
- An **Actions** menu, also available from a **⋮** button when you hover a worktree in the sidebar, opens the current directory in Finder or VS Code, or copies its path. If you've `cd`'d somewhere in tmux, it uses that directory.
- **Rename Worktree** gives a worktree a short display name. It doesn't affect Git, the folder or the tmux session. Clear the name to show the branch again.

**Pull requests**
- If the selected worktree's branch has an **open** GitHub pull request, a **PR #N** button appears in the header. Clicking it opens the PR in your browser. This uses your existing `gh` login, and the button stays hidden if anything goes wrong.

**Look and layout**
- Dark Dracula theme by default, 3-pixel rounded corners, translucent "glass" panels with macOS blur.
- Compact by design: a single-row header, a narrow sidebar with names that never wrap (hover for details), and a terminal with no padding.
- The sidebar can be hidden for a full-width terminal. While it's hidden, hovering the toggle shows the sidebar as a temporary overlay.

**Settings and theme**
- A gear at the top left opens a full-window Settings screen. Its first page, **Theme**, edits the app's eleven Dracula palette colors with a color wheel, a brightness slider and hex fields, and previews every change live.
- A preset menu offers Dracula, Alucard (light) and the Dracula Pro family (Pro, Blade, Buffy, Lincoln, Morbius, Van Helsing).
- **Save & Confirm** keeps the theme across launches; **Cancel** or Escape discards the changes. The terminal keeps your Alacritty colors.

## Not in scope (yet)

- Creating worktrees from the app.
- Automatically starting a coding agent (you start it in the shell).
- Packaging as a `.app`, automatic refresh or file watching, and sessions that survive a reboot.

## Requirements and verification

| Area | Requirements | Covered by automated tests |
| --- | --- | --- |
| Projects | 9 | 9 |
| Worktrees | 8 | 8 |
| Terminal sessions | 12 | 10 |
| Terminal preferences | 9 | 6 |
| Actions and rename | 8 | 8 |
| Pull requests | 6 | 6 |
| Interface | 11 | 5 |
| Constraints | 7 | 5 |
| Settings and theme | 10 | 8 |
| **Total** | **80** | **65** |

The 15 requirements without automated tests are about visual rendering or OS integration, such as blur, tooltips, font matching and Finder opening. They're checked manually; each FRD document marks them **Manual** with the check to perform.

## Needs

macOS, Git 2.41+, tmux 3.5+. Optional: GitHub CLI (`gh`) for the PR button, VS Code for "Open in VS Code".
