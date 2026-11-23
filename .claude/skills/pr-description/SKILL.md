---
name: pr-description
description: Write or rewrite a pull request title and description in this project's preferred format, with a "What changed" section and a "Why" section and nothing else. Use whenever creating a PR (`gh pr create`), editing a PR body (`gh pr edit`), or when asked to draft, write, update or tidy a PR description.
---

# Pull request descriptions

A PR description tells a reviewer what the change does and why it was made.
It isn't a work log. The diff, the commits and CI already show how the work
was done and checked, so the description doesn't repeat them.

## Format

Two sections, in this order, and no others:

```markdown
## What changed

- <one bullet per user-visible behavior or structural change>

## Why

<the problem or goal behind the change, in a few sentences>
```

Then any attribution footer the environment requires (for example the
"Generated with Claude Code" line), after a blank line.

## Title

- Describe the change as an outcome, in sentence case, with no trailing period,
  under about 70 characters: "Keep the Settings gear visible so a second click
  closes Settings".
- Use a short lowercase prefix only where the repository already does
  (`deps:`, `ci:`).
- One change per title. If it needs "and", "plus" or a list of items, the PR is
  probably several PRs. Say so to the user rather than writing a long title.

## What changed

- Lead with behavior a user or reviewer would notice: what the app now does,
  what it no longer does, and what changed in CI, dependencies or the build.
- Name the requirement IDs a change touches (`WT-06`, `SET-03`) when the change
  adds or alters behavior in `docs/frd/`. Give the ID and the behavior, not the
  edit to the doc.
- Mention a structural change (a module split, a new shared helper) in one
  bullet when a reviewer needs it to read the diff. Don't list files.
- Call out anything with lasting effects on the user's machine: changed paths
  or file formats, tmux session naming (migrations), new required tools.
- If the reviewer must decide or check something (an unverified value, a
  trade-off you picked), say so in one bullet at the end, starting with
  "Please check:".
- For a visible UI change, a screenshot can go here. It shows what changed.
- Write plain bullets. Bold only the name of a UI element or a setting, not
  whole phrases.

## Why

- Give the motivation: the bug and its symptom, the request, or the
  limitation. Write a short paragraph, not bullets, unless there are several
  independent reasons.
- Link the issue, review item (`docs/CODE_REVIEW.md` IDs such as `M1`) or
  earlier PR the change answers.
- If you chose between approaches, say in one sentence why this one.

## Leave out

These add length without helping a reviewer decide. Don't include them, even
in a short form:

- Verification steps, a "Test plan" or "How to test" section, or checklists
  of commands run (`cargo fmt`, `clippy`, `cargo test`, "63 passed").
- Notes about tests: lists of test names added, "added tests for X", or
  coverage numbers. The tests are in the diff and the FRD cites them.
- CI or lint status, and what you couldn't run locally.
- File-by-file change lists, and line counts.
- Explanations of Rust or iced patterns used. Those belong in code comments
  (see `AGENTS.md`), not the PR.
- The history of how you got there: dead ends, earlier attempts, reverted
  ideas.
- Filler such as "This PR…" openers, summaries of the summary, and closing
  remarks.

## Example

```markdown
## What changed

- A failed refresh keeps the last good worktree list instead of replacing it
  with the error, so the selection and the terminal stay put (WT-06).
- Expanding a project in the sidebar only toggles the row. It no longer
  selects the project, unless nothing is selected yet (UI-12).

## Why

Both made the worktree you were working in disappear from under you. After a
failed refresh the selected worktree looked deleted, so the selection jumped
to Overview and the terminal detached. Expanding a project selected it,
which detached the terminal the same way. Both are listed in `docs/CODE_REVIEW.md` (M1, M2).
```

## Writing it

- Read the full diff against the base branch (`git diff origin/main...HEAD`)
  and the commit messages before writing. Describe the PR as a whole, not
  commit by commit.
- Pass the body with `--body-file` or a quoted heredoc so backticks and `$`
  survive the shell.
- When updating an existing PR, rewrite the body to this format rather than
  appending to the old one.
