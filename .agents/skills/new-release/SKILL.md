---
name: new-release
description: Cut a release of Worktree Browser — pick the next major/minor/patch version from the changes since the last release, commit the bump, tag it vX.Y.Z, push to main — then build "Worktree Browser.app", install it in /Applications, and relaunch it. Use when asked to rebuild/install/ship/refresh the release build, update the installed app, or "make/create a new release".
---

# New release

Cuts a version and installs it on this Mac:

1. works out the next version from the changes since the last release;
2. commits the bump to `Cargo.toml` and `Cargo.lock` as `Release X.Y.Z`, tags
   it `vX.Y.Z` and pushes the commit and the tag to `origin/main`;
3. builds that commit and installs it as `/Applications/Worktree Browser.app`,
   replacing and relaunching the running copy.

There is no GitHub Release; the tag is the record.

## Run it

From the root of the checkout (the main checkout or any worktree; it releases
whatever is checked out there, which must be `origin/main`). Check the plan
first, then cut the release:

```sh
ruby .agents/skills/new-release/release.rb --plan
ruby .agents/skills/new-release/release.rb --co-author "<your Co-Authored-By value>"
```

`--plan` fetches, prints each change since the last release with the bump it
implies and the next version, and stops. Read the list before releasing: if
the heuristic got a change wrong (say, a feature merged from a branch without
a `feat/` prefix), rerun with `--bump major|minor|patch`. Pass your commit
attribution with `--co-author` (it becomes the release commit's
`Co-Authored-By:` trailer); a person running the script can leave it out.
The build takes a few minutes the first time, so give the command a long
timeout (10 minutes).

Other options: `--dest DIR` installs somewhere other than `/Applications`;
`--no-open` installs without relaunching. `--no-version` installs the checkout
as it is, with no bump, tag or push: use it only when the user asks to
install a branch that isn't on main, and with it `--skip-fetch` (offline) and
`--allow-behind` (build without origin's newest commits) are allowed.

## How the version is chosen

`version_bump.rb` looks at the main-line commits since the last release: the
`vX.Y.Z` tag of the current `Cargo.toml` version, or, for versions released
before tagging began, the main-line commit that set it.

- A merged PR's type is its branch prefix (`feat/…`, `fix/…`, `ci/…`,
  `chore/…`, `dependabot/…`); a direct commit's is its Conventional Commits
  prefix (`fix: …`).
- `feat` is a **minor** bump; any other type, or none, is a **patch**.
- `type!:` in a subject or PR title, a `feat!/…` branch, or a
  `BREAKING CHANGE:` footer in any commit in the range is a **major** bump.
- Release commits and `release/…` merges are ignored. With no changes since
  the last release, the script reinstalls the current version without a tag.

Plain semver is applied as is, so a breaking change takes 0.x to 1.0.0.
Tests: `ruby .agents/skills/new-release/test/version_bump_test.rb`.

## What the script does

1. Fetches origin's default branch and fast-forwards the checkout if it's
   strictly behind and clean. A versioned release then requires that HEAD
   **is** `origin/main` with no uncommitted changes to tracked files; otherwise
   it stops. Land the work on main first (or, if the user asks, use
   `--no-version`). It never touches the local `main` branch ref, which may be
   checked out in another worktree.
2. Refuses if the tag already exists, writes the new version, and runs
   `cargo build --release --locked` (the build fails, and the version edit is
   reverted, if the lock file doesn't match).
3. Commits `Release X.Y.Z` (the changes listed in the body), creates the
   annotated tag, and pushes both with `git push --atomic origin
   HEAD:refs/heads/main refs/tags/vX.Y.Z`. If the push fails (main moved), it
   deletes the local tag and resets to where it started, so a rerun is clean.
   Then it rebuilds, so `build.rs` sees the new tag and the window title
   shows just `vX.Y.Z` rather than a branch and commit.
4. Assembles `target/release/bundle/Worktree Browser.app`: the binary is
   renamed `worktree-browser`, `Info.plist` is generated (bundle id
   `local.worktree-browser`, version `X.Y.Z (<sha>)`), and `AppIcon.icns` is
   rendered from `assets/icon.svg` with `rsvg-convert` and `iconutil` (falls
   back to the old bundle's icon). It's ad-hoc signed (`codesign --sign -`)
   and verified.
5. Stops only the copies running from the install path and from the old
   `~/Desktop` location, with SIGTERM (escalating to SIGKILL after 5s). This
   skips the quit confirmation dialog and loses nothing: terminals live in
   tmux on the `-L worktree-browser` socket and reattach on relaunch. A
   `cargo run` dev instance is untouched.
6. Moves the old `/Applications` bundle to `target/release/previous/` (one
   rollback copy), moves any bundle left on the Desktop by earlier releases to
   the Trash, copies the new one into place with `ditto`, opens it and waits
   for the process to appear.

## Before running

- Don't run the full check suite unless asked; CI runs it on every PR before merge. If
  the build fails, report the compiler error.
- If the checkout isn't `origin/main` (a feature branch, local commits,
  uncommitted changes), don't work around it: ask the user whether to land
  the work first or install it unversioned with `--no-version`.

## After running

Report the version change and bump level (with the reason), the tag and that
it was pushed, the installed version line, and the new pid. If a Desktop copy
was moved to the Trash, say so. To roll back the install (the tag and commit
stay; they're history):

```sh
rm -rf /Applications/"Worktree Browser.app"
mv target/release/previous/"Worktree Browser.app" /Applications/
open /Applications/"Worktree Browser.app"
```
