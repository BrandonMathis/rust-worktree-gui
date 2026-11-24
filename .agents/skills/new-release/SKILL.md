---
name: new-release
description: Build a release of Worktree Browser, package it as "Worktree Browser.app", stop the running copy, replace the bundle on the Desktop and relaunch it. Use when asked to rebuild/install/ship/refresh the release build, update the Desktop app, or "make/create a new release". This is the project's release process: not a version bump PR, tag or GitHub release.
---

# New release (local install)

Builds the current checkout in release mode and installs it as
`~/Desktop/Worktree Browser.app`, replacing and relaunching any running copy.
This is a local install for the owner, not a GitHub release or tag.

## Run it

From the root of the checkout to build (the main checkout or any worktree; it
builds whatever is checked out there):

```sh
ruby .agents/skills/new-release/release.rb
```

The skill is committed with the project, so every checkout and worktree has
its own copy. It builds the checkout it's run from.

Options: `--dest DIR` installs somewhere other than `~/Desktop`; `--no-open`
installs without relaunching; `--skip-fetch` skips the origin check (offline);
`--allow-behind` builds even when the checkout is missing origin's commits.
The build takes a few minutes the first time, so give the command a long
timeout (10 minutes).

## What the script does

1. Fetches origin's default branch (`origin/main`) and compares it with HEAD:
   - up to date (HEAD contains it): continue;
   - strictly behind, with no uncommitted changes to tracked files:
     fast-forward (`git merge --ff-only`) and print the commits pulled in;
   - diverged, or behind with uncommitted changes: **stop** and list the
     missing commits. Ask the user whether to merge/rebase first or rebuild
     with `--allow-behind`; don't decide for them.
   It never touches the local `main` branch ref, which may be checked out
   in another worktree.
2. `cargo build --release --locked`.
3. Assembles `target/release/bundle/Worktree Browser.app`: the binary is
   renamed `worktree-browser`, `Info.plist` is generated (bundle id
   `local.worktree-browser`, version `<Cargo version> (<branch> <sha>[-dirty])`),
   and `AppIcon.icns` is rendered from `assets/icon.svg` with `rsvg-convert`
   and `iconutil` (falls back to the old bundle's icon).
4. Ad-hoc signs the bundle (`codesign --sign -`) and verifies it.
5. Stops only the copy running from the destination path, with SIGTERM
   (escalating to SIGKILL after 5s). This skips the quit confirmation dialog
   and loses nothing: terminals live in tmux on the `-L worktree-browser`
   socket and reattach on relaunch. A `cargo run` dev instance is untouched.
6. Moves the old bundle to `target/release/previous/` (one rollback copy),
   copies the new one into place with `ditto`, opens it and waits for the
   process to appear.

## Before running

- Mention it if `git status` shows uncommitted changes: the build includes
  them and the version is marked `-dirty`. Go ahead unless the user objects.
- Don't run the full check suite unless asked; the user wants the build. If
  the build fails, report the compiler error rather than installing anything.

## After running

Report the installed version line, the new pid, and whether the checkout was
already up to date with `origin/main` or was fast-forwarded (say which commit
was built; don't claim "latest main" without the script's confirmation). To roll back:

```sh
rm -rf ~/Desktop/"Worktree Browser.app"
mv target/release/previous/"Worktree Browser.app" ~/Desktop/
open ~/Desktop/"Worktree Browser.app"
```
