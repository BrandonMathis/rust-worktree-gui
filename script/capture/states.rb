# frozen_string_literal: true

# The app states the PR screenshot workflow can capture, and the rule for which
# of them a pull request gets. capture.rb takes the screenshots; select.rb runs
# the rule on a PR's changed files.
#
# A PR gets screenshots of the states whose `covers` globs match a file it
# changes, so reviewers see the feature it adds or modifies, not a fixed tour of
# the app. A PR that adds a UI feature no state shows should add a state here
# (and, if the app needs a new startup hook to reach it, a variant in
# src/capture.rs) with `covers` naming the feature's files.
module CaptureStates
  GIT_LOG = "git lg"

  # Files that change how every state looks.
  SHARED = %w[
    src/main.rs
    src/backdrop.rs
    src/style.rs
    src/theme.rs
    src/icons.rs
    assets/**/*
  ].freeze

  # The capture tooling itself. A PR that changes only these gets every state,
  # so its effect on the screenshots is visible.
  TOOLING = %w[
    script/capture/**/*
    src/capture.rs
    .github/workflows/screenshots.yml
  ].freeze

  # Each state opens one worktree (or a view such as Settings) at one window
  # size. Keys:
  #
  # - name, title, size: the PNG's file name, its caption, and the window size.
  # - covers: globs (relative to the repository root) of the files whose
  #   changes this state shows. SHARED is added to every state.
  # - project, worktree: the bookmark index and fixture directory to open.
  # - open: a view from src/capture.rs to show at startup, such as "settings".
  # - layout: lays out the worktree's tmux window, given the tmux helper and
  #   the first pane. Omit it for states that show no terminal.
  # - prompt: for a managed worktree, a prompt to send at startup, so the
  #   screenshot shows a conversation (answered by script/capture/agent-bridge).
  # - settle: seconds to wait before capturing, when the default is too short.
  STATES = [
    {
      name: "01-workspace",
      title: "Workspace — sidebar with PR badges, vim and shell splits",
      size: "1120x720",
      covers: %w[
        src/app/view.rs src/app/sidebar.rs src/app/widgets.rs src/app/pull_requests.rs
        src/app/sessions.rs src/app/terminal_pane.rs src/app/project_list.rs src/row_menu.rs
        src/terminal.rs src/terminal_config.rs vendor/iced_term/src/**/*
      ],
      project: 0, worktree: "acme-api-session-tokens",
      layout: lambda do |tmux, pane|
        right = tmux.split(pane, "-h", 42)
        bottom = tmux.split(right, "-v", 55)
        tmux.type(right, GIT_LOG)
        tmux.type(bottom, "git status --short && cargo test")
        tmux.type(pane, "vim src/auth/tokens.rs")
        tmux.wait_for_text(bottom, "test result")
        tmux.wait_for_text(pane, "pub struct Token")
      end
    },
    {
      name: "02-review",
      title: "Review — vim diff over shell panes",
      size: "1120x720",
      covers: %w[src/terminal.rs src/terminal_config.rs vendor/iced_term/src/**/*],
      project: 1, worktree: "design-system-color-tokens",
      layout: lambda do |tmux, pane|
        bottom = tmux.split(pane, "-v", 34)
        right = tmux.split(bottom, "-h", 50)
        tmux.type(bottom, "git status -sb && git diff --stat main")
        tmux.type(right, "git branch -vv")
        tmux.type(pane, "vim -d src/tokens/light.ts src/tokens/dark.ts")
        tmux.wait_for_text(right, "chore/drop-ie11")
        tmux.wait_for_text(pane, "export const")
      end
    },
    {
      name: "03-compact",
      title: "Minimum window (820×440) — vim beside a colored diff",
      size: "820x440",
      covers: %w[
        src/app/view.rs src/app/sidebar.rs src/app/widgets.rs src/app/pull_requests.rs
        src/app/terminal_pane.rs
      ],
      project: 0, worktree: "acme-api-rate-limiter",
      layout: lambda do |tmux, pane|
        right = tmux.split(pane, "-h", 50)
        tmux.type(right, "git diff")
        tmux.type(pane, "vim +12 src/limits.rs")
        tmux.wait_for_text(right, "@@")
        tmux.wait_for_text(pane, "struct TokenBucket")
      end
    },
    {
      name: "04-settings",
      title: "Settings — theme colors and the color wheel",
      size: "1120x720",
      covers: %w[src/settings.rs src/color_wheel.rs],
      open: "settings"
    },
    {
      name: "05-long-list",
      title: "Long worktree list — the first 10, the selected one, and Show N more",
      size: "1120x720",
      covers: %w[src/app/sidebar.rs],
      # The twelfth worktree: past the limit, so it is listed after the first ten.
      project: 2, worktree: "docs-site-12-faq",
      layout: lambda do |tmux, pane|
        tmux.type(pane, GIT_LOG)
        tmux.wait_for_text(pane, "Start the docs site")
      end
    },
    {
      name: "06-unavailable",
      title: "Unavailable worktrees — the list from the project menu",
      size: "1120x720",
      covers: %w[src/app/unavailable.rs],
      open: "unavailable"
    },
    {
      name: "07-managed",
      title: "Managed worktree — choose a harness, model and thinking level, then prompt",
      size: "1120x720",
      covers: %w[src/app/managed.rs src/agent.rs],
      project: 0, worktree: "acme-api-audit-log",
      settle: 4
    },
    {
      name: "08-managed-chat",
      title: "Managed worktree — a turn with collapsed thinking and tool calls",
      size: "1120x720",
      covers: %w[src/app/managed.rs src/agent.rs],
      project: 0, worktree: "acme-api-audit-log",
      prompt: "Add an audit log entry for every API request, with tests.",
      settle: 5
    },
    {
      name: "09-earlier-chats",
      title: "Managed worktree — New Chat keeps the last conversation under Earlier chats, to resume",
      size: "1120x720",
      covers: %w[src/app/managed.rs],
      project: 0, worktree: "acme-api-audit-log",
      open: "earlier-chats",
      prompt: "Add an audit log entry for every API request, with tests.",
      settle: 4
    }
  ].freeze

  module_function

  # The states a PR that changes `paths` should capture, in catalog order:
  # those that show a changed file; failing that, every state if the capture
  # tooling changed; otherwise none (the PR changes nothing a state shows).
  def select(paths, states = STATES)
    shown = states.select { |state| paths.any? { |path| covers?(state, path) } }
    return shown unless shown.empty?

    paths.any? { |path| match?(TOOLING, path) } ? states : []
  end

  def covers?(state, path)
    match?(state[:covers] + SHARED, path)
  end

  def match?(globs, path)
    # FNM_PATHNAME keeps `*` within one directory; `**/` spans directories.
    globs.any? { |glob| File.fnmatch?(glob, path, File::FNM_PATHNAME | File::FNM_DOTMATCH) }
  end
end
