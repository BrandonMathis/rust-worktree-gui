#!/usr/bin/env ruby
# frozen_string_literal: true

# Captures screenshots of Worktree Browser in a few key states, using fixture
# repositories and a private tmux server. The PR screenshot workflow
# (.github/workflows/screenshots.yml) runs it; it also works locally:
#
#   cargo build --locked --features capture
#   ruby script/capture/capture.rb [--bin PATH] [--out DIR] [--only NAME,...]
#
# The states are listed in states.rb. The workflow passes --only with the
# states that show the PR's changed files (select.rb).
#
# Isolation: the app runs with HOME pointed at a scratch directory (bookmarks,
# Alacritty, zsh, vim and git config all come from script/capture/home), and
# WORKTREE_BROWSER_TMUX points at a wrapper that moves the app's tmux socket to
# a scratch server. Your real bookmarks, tmux sessions and dotfiles are never
# read or changed. A window does briefly appear on screen for each state, and
# the terminal running this script needs the Screen Recording permission.

require "fileutils"
require "json"
require "open3"
require "optparse"
require "timeout"
require_relative "states"

HERE = File.expand_path(__dir__)
REPO = File.expand_path("../..", HERE)
MARKER = ".worktree-browser-capture"
# Seconds to wait after the panes are ready, before capturing.
SETTLE = Float(ENV.fetch("CAPTURE_SETTLE", "1.5"))
# CAPTURE_REFRESH=1 makes tmux resend the whole screen before each capture.
# Off by default, so the PNGs show exactly what the app drew, including any
# terminal update it failed to paint.
REFRESH = ENV["CAPTURE_REFRESH"] == "1"

options = {
  bin: File.join(REPO, "target/debug/rust-worktree-gui"),
  out: File.join(REPO, "target/screenshots"),
  root: ENV.fetch("CAPTURE_ROOT", "/tmp/wtb-demo"),
  only: nil
}
OptionParser.new do |opts|
  opts.banner = "Usage: capture.rb [options]"
  opts.on("--bin PATH", "App binary built with --features capture") { |v| options[:bin] = File.expand_path(v) }
  opts.on("--out DIR", "Where to write the PNGs") { |v| options[:out] = File.expand_path(v) }
  opts.on("--root DIR", "Scratch directory for fixtures (recreated)") { |v| options[:root] = v }
  opts.on("--only NAMES", Array, "Capture only these states, e.g. 02-review,04-settings") { |v| options[:only] = v }
end.parse!

def log(message)
  warn "[capture] #{message}"
end

# Runs a command from an argument vector (never a shell string) and returns
# stdout. Raises with the command's output if it fails.
def run(*args, env: {}, chdir: nil)
  opts = chdir ? { chdir: chdir } : {}
  out, status = Open3.capture2e(env, *args, **opts)
  raise "#{args.join(' ')} failed:\n#{out}" unless status.success?

  out
end

# Polls until the block returns a truthy value, or raises after `seconds`.
def wait_for(what, seconds: 30, interval: 0.25)
  deadline = Time.now + seconds
  loop do
    value = yield
    return value if value
    raise "Timed out after #{seconds}s waiting for #{what}" if Time.now > deadline

    sleep interval
  end
end

def find_executable(name)
  ENV.fetch("PATH", "").split(File::PATH_SEPARATOR).map { |dir| File.join(dir, name) }
     .find { |path| File.file?(path) && File.executable?(path) }
end

# --- Fixture repositories --------------------------------------------------

class Fixtures
  attr_reader :root, :home, :projects

  def initialize(root)
    @root = root
    @home = File.join(root, "home")
    @clock = Time.utc(2026, 11, 2, 9, 30)
    @projects = []
  end

  def build
    FileUtils.cp_r(File.join(HERE, "home/."), home)
    # Login shells run macOS path_helper after .zshenv, so put the fake gh and
    # cargo first here, in .zshrc, which runs last.
    File.write(File.join(home, ".zshrc"), "path=(#{File.join(HERE, "bin")} $path)\n", mode: "a")
    build_acme_api
    build_design_system
    build_docs_site
    write_bookmarks
  end

  # Canonical path, as Git reports it (/tmp is a symlink to /private/tmp).
  def path(*parts)
    File.realpath(File.join(root, *parts))
  end

  private

  def git_env
    # Fixed author dates keep commit hashes and `git log` output stable.
    @clock += 3 * 3600 + 17 * 60
    stamp = @clock.strftime("%Y-%m-%dT%H:%M:%S+0000")
    {
      "HOME" => home, "GIT_CONFIG_NOSYSTEM" => "1",
      "GIT_AUTHOR_DATE" => stamp, "GIT_COMMITTER_DATE" => stamp
    }
  end

  def git(dir, *args)
    run("git", "-C", dir, *args, env: git_env)
  end

  def write(dir, file, content)
    target = File.join(dir, file)
    FileUtils.mkdir_p(File.dirname(target))
    File.write(target, content)
  end

  def commit(dir, message, files = {})
    files.each { |file, content| write(dir, file, content) }
    git(dir, "add", "-A")
    git(dir, "commit", "-q", "-m", message)
  end

  def new_repo(name)
    dir = File.join(root, name)
    FileUtils.mkdir_p(dir)
    git(dir, "init", "-q", "-b", "main")
    dir
  end

  def build_acme_api
    repo = new_repo("acme-api")
    commit(repo, "Initial service skeleton", "Cargo.toml" => CARGO_TOML, "src/main.rs" => MAIN_RS,
                                              "README.md" => "# acme-api\n\nPublic API for Acme.\n")
    commit(repo, "Add health and metrics routes", "src/routes.rs" => ROUTES_RS)
    commit(repo, "Add a token bucket rate limiter", "src/limits.rs" => LIMITS_RS)
    commit(repo, "Issue signed session tokens", "src/auth/tokens.rs" => TOKENS_RS_V1)
    git(repo, "branch", "release/2.4")

    tokens = File.join(root, "acme-api-session-tokens")
    git(repo, "worktree", "add", "-q", "-b", "feature/session-tokens", tokens)
    commit(tokens, "Reject expired session tokens", "src/auth/tokens.rs" => TOKENS_RS_V2)
    commit(tokens, "Rotate signing secrets on refresh", "src/auth/tokens.rs" => TOKENS_RS)
    commit(tokens, "Test token expiry and tampering", "src/auth/mod.rs" => "pub mod tokens;\n")
    # Uncommitted work, so the shell panes have something to show.
    write(tokens, "src/routes.rs", ROUTES_RS.sub("// TODO: auth", "auth::require(&request)?;"))

    limiter = File.join(root, "acme-api-rate-limiter")
    git(repo, "worktree", "add", "-q", "-b", "fix/rate-limiter", limiter)
    write(limiter, "src/limits.rs", LIMITS_RS_FIXED)

    git(repo, "worktree", "add", "-q", File.join(root, "acme-api-release"), "release/2.4")

    # A worktree whose folder was deleted outside Git shows as unavailable.
    spike = File.join(root, "acme-api-grpc-spike")
    git(repo, "worktree", "add", "-q", "-b", "spike/grpc", spike)
    FileUtils.rm_rf(spike)

    @projects << { root: path("acme-api"), names: { path("acme-api-session-tokens") => "Session tokens" } }
  end

  def build_design_system
    repo = new_repo("design-system")
    commit(repo, "Scaffold the component library", "package.json" => PACKAGE_JSON,
                                                    "src/components/Button.tsx" => BUTTON_TSX)
    commit(repo, "Add light and dark themes", "src/tokens/light.ts" => LIGHT_TS, "src/tokens/dark.ts" => DARK_TS_V1)
    commit(repo, "Document the button variants", "docs/button.md" => "# Button\n\nPrimary, ghost and danger.\n")

    color = File.join(root, "design-system-color-tokens")
    git(repo, "worktree", "add", "-q", "-b", "feat/color-tokens", color)
    commit(color, "Use Dracula hues for the dark theme", "src/tokens/dark.ts" => DARK_TS)
    write(color, "src/components/Badge.tsx", BADGE_TSX)

    git(repo, "worktree", "add", "-q", "-b", "chore/drop-ie11", File.join(root, "design-system-ie11"))
    git(repo, "worktree", "add", "-q", "--detach", File.join(root, "design-system-v1"), "HEAD~1")

    @projects << { root: path("design-system"), names: {} }
  end

  def build_docs_site
    repo = new_repo("docs-site")
    commit(repo, "Start the docs site", "index.md" => "# Acme docs\n")
    @projects << { root: path("docs-site"), names: {} }
  end

  # Same format as src/config.rs.
  def write_bookmarks
    bookmarks = projects.map do |project|
      {
        anchor: project[:root],
        common_dir: File.join(project[:root], ".git"),
        worktree_names: project[:names].map { |p, name| { path: p, name: name } }
      }
    end
    dir = File.join(home, "Library/Application Support/Worktree Browser")
    FileUtils.mkdir_p(dir)
    File.write(File.join(dir, "projects.json"), JSON.pretty_generate(bookmarks))
  end
end

# --- tmux ------------------------------------------------------------------

class Tmux
  def initialize(executable, socket, home)
    @executable = executable
    @socket = socket
    @home = home
  end

  # The app runs `tmux -L worktree-browser ...`. This wrapper drops `-L` and
  # uses the scratch socket and config instead.
  def write_wrapper(path)
    File.write(path, <<~SH)
      #!/bin/sh
      [ "$1" = "-L" ] && shift 2
      exec #{sh_quote(@executable)} -S #{sh_quote(@socket)} -f #{sh_quote(File.join(HERE, 'tmux.conf'))} "$@"
    SH
    File.chmod(0o755, path)
  end

  def call(*args)
    run(@executable, "-S", @socket, *args, env: { "HOME" => @home })
  end

  def try(*args)
    _out, status = Open3.capture2e(@executable, "-S", @socket, *args)
    status.success?
  end

  def redraw_clients
    call("list-clients", "-F", %q(#{client_name})).split.each do |client|
      call("refresh-client", "-t", client)
    end
  end

  # Prints every pane, to debug a state that timed out.
  def dump
    call("list-panes", "-a", "-F", %q(#{pane_id})).split.each do |pane|
      warn "----- pane #{pane} -----", text(pane)
    end
  rescue StandardError => e
    warn "Could not read tmux panes: #{e.message}"
  end

  def kill_server
    try("kill-server")
  end

  # The pane id of the session the app created for this state.
  def first_pane
    wait_for("the app to start its tmux session") do
      try("has-session") && call("list-panes", "-a", "-F", '#{pane_id}').lines.first&.strip
    end
  end

  def split(target, direction, percent)
    # New panes start in the worktree, like the pane they split.
    call("split-window", direction, "-l", "#{percent}%", "-c", '#{pane_current_path}',
         "-t", target, "-P", "-F", '#{pane_id}').strip
  end

  def text(pane)
    call("capture-pane", "-p", "-J", "-t", pane)
  end

  # Types a command once the shell has drawn its prompt. Typing earlier makes
  # zsh echo the line twice.
  def type(pane, command)
    wait_for("a shell prompt in #{pane}") { text(pane).include?("❯") }
    call("send-keys", "-t", pane, "-l", command)
    call("send-keys", "-t", pane, "Enter")
  end

  def wait_for_text(pane, needle)
    wait_for("#{needle.inspect} in pane #{pane}") { text(pane).include?(needle) }
  end

  private

  def sh_quote(value)
    "'#{value.gsub("'", %q('\\''))}'"
  end
end

# --- Capture ---------------------------------------------------------------

def window_id(pid)
  wait_for("the app window", seconds: 60) do
    id = run("osascript", "-l", "JavaScript", File.join(HERE, "window_id.js"), pid.to_s).strip
    id.empty? ? nil : id
  end
end

def capture_state(state, fixtures, app_env, bin, tmux, out)
  log "#{state[:name]}: #{state[:title]}"
  env = app_env.merge("WORKTREE_BROWSER_CAPTURE_SIZE" => state[:size])
  if state[:worktree]
    env["WORKTREE_BROWSER_CAPTURE_PROJECT"] = state[:project].to_s
    env["WORKTREE_BROWSER_CAPTURE_WORKTREE"] = fixtures.path(state[:worktree])
  end
  env["WORKTREE_BROWSER_CAPTURE_OPEN"] = state[:open] if state[:open]
  pid = Process.spawn(env, bin, out: File::NULL, err: File.join(out, "#{state[:name]}.log"))
  begin
    id = window_id(pid)
    # States without a layout show no terminal, so there is no tmux to wait for.
    if state[:layout]
      pane = tmux.first_pane
      state[:layout].call(tmux, pane)
      tmux.redraw_clients if REFRESH
    end
    sleep SETTLE # Let the app draw the last tmux frame.
    file = File.join(out, "#{state[:name]}.png")
    run("screencapture", "-x", "-o", "-l#{id}", file)
    log "  wrote #{file}"
  rescue StandardError
    tmux.dump
    raise
  ensure
    Process.kill("TERM", pid)
    Process.wait(pid)
    tmux.kill_server
  end
end

# --- Main ------------------------------------------------------------------

abort "Build the app first: cargo build --locked --features capture" unless File.executable?(options[:bin])
real_tmux = find_executable("tmux") or abort "tmux is required (brew install tmux)"

root = options[:root]
if File.exist?(root)
  abort "Refusing to replace #{root}: it was not created by this script" unless File.exist?(File.join(root, MARKER))
  FileUtils.rm_rf(root)
end
FileUtils.mkdir_p(root)
File.write(File.join(root, MARKER), "")
FileUtils.mkdir_p(options[:out])

fixtures = Fixtures.new(root)
fixtures.build
log "fixtures in #{fixtures.root}"

tmux = Tmux.new(real_tmux, File.join(fixtures.root, "tmux.sock"), fixtures.home)
tmux.kill_server
wrapper = File.join(root, "tmux")
tmux.write_wrapper(wrapper)

app_env = {
  "HOME" => fixtures.home,
  "PATH" => "#{File.join(HERE, 'bin')}:#{ENV.fetch('PATH', '')}",
  "LANG" => "en_US.UTF-8",
  "WORKTREE_BROWSER_TMUX" => wrapper,
  # Keep the user's own config and any outer tmux or Git state out of the app.
  "XDG_CONFIG_HOME" => nil, "TMUX" => nil, "TMUX_PANE" => nil,
  "GIT_DIR" => nil, "GIT_WORK_TREE" => nil, "GIT_CONFIG_NOSYSTEM" => "1"
}

selected = CaptureStates::STATES.select { |s| options[:only].nil? || options[:only].include?(s[:name]) }
unknown = Array(options[:only]) - selected.map { |s| s[:name] }
abort "No state named #{unknown.join(', ')}" unless unknown.empty?
selected.each { |state| capture_state(state, fixtures, app_env, options[:bin], tmux, options[:out]) }

File.write(File.join(options[:out], "states.json"),
           JSON.pretty_generate(selected.map { |s| { name: s[:name], title: s[:title], size: s[:size] } }))
log "done"

# --- Fixture file contents ---------------------------------------------------

BEGIN {
  CARGO_TOML = <<~TOML
    [package]
    name = "acme-api"
    version = "2.4.0"
    edition = "2024"

    [dependencies]
    hmac = "0.12"
    sha2 = "0.10"
    time = "0.3"
  TOML

  MAIN_RS = <<~RUST
    mod auth;
    mod limits;
    mod routes;

    fn main() {
        let limiter = limits::TokenBucket::new(100, 10.0);
        routes::serve("0.0.0.0:8080", limiter);
    }
  RUST

  ROUTES_RS = <<~RUST
    use crate::limits::TokenBucket;

    pub fn serve(address: &str, mut limiter: TokenBucket) {
        println!("listening on {address}");
        // TODO: auth
        while limiter.take() {}
    }
  RUST

  LIMITS_RS = <<~RUST
    use std::time::Instant;

    /// Allows short bursts while holding a steady average request rate.
    pub struct TokenBucket {
        capacity: u32,
        tokens: f64,
        refill_per_sec: f64,
        last: Instant,
    }

    impl TokenBucket {
        pub fn new(capacity: u32, refill_per_sec: f64) -> Self {
            Self { capacity, tokens: capacity as f64, refill_per_sec, last: Instant::now() }
        }

        pub fn take(&mut self) -> bool {
            let now = Instant::now();
            let elapsed = now - self.last;
            self.tokens += elapsed.as_secs_f64() * self.refill_per_sec;
            self.last = now;
            if self.tokens >= 1.0 {
                self.tokens -= 1.0;
                true
            } else {
                false
            }
        }
    }
  RUST

  LIMITS_RS_FIXED = LIMITS_RS.sub(
    "self.tokens += elapsed.as_secs_f64() * self.refill_per_sec;",
    "// Never refill past capacity, or an idle client gets an unbounded burst.\n" \
    "        let refill = elapsed.as_secs_f64() * self.refill_per_sec;\n" \
    "        self.tokens = (self.tokens + refill).min(self.capacity as f64);"
  )

  TOKENS_RS_V1 = <<~RUST
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    pub struct Token {
        pub user_id: u64,
        pub signature: Vec<u8>,
    }

    pub fn issue(user_id: u64, secret: &[u8]) -> Token {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("any key length");
        mac.update(&user_id.to_be_bytes());
        Token { user_id, signature: mac.finalize().into_bytes().to_vec() }
    }
  RUST

  TOKENS_RS_V2 = TOKENS_RS_V1.sub("pub signature", "pub expires_at: OffsetDateTime,\n    pub signature")

  TOKENS_RS = <<~RUST
    //! Signed, short-lived session tokens.
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use time::{Duration, OffsetDateTime};

    const LIFETIME: Duration = Duration::minutes(15);

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Token {
        pub user_id: u64,
        pub expires_at: OffsetDateTime,
        pub signature: Vec<u8>,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub enum TokenError {
        Expired,
        BadSignature,
    }

    pub fn issue(user_id: u64, secret: &[u8], now: OffsetDateTime) -> Token {
        let expires_at = now + LIFETIME;
        let signature = sign(user_id, expires_at, secret);
        Token { user_id, expires_at, signature }
    }

    pub fn verify(token: &Token, secret: &[u8], now: OffsetDateTime) -> Result<u64, TokenError> {
        if now >= token.expires_at {
            return Err(TokenError::Expired);
        }
        if sign(token.user_id, token.expires_at, secret) != token.signature {
            return Err(TokenError::BadSignature);
        }
        Ok(token.user_id)
    }

    /// Issues a fresh token signed with the newest secret.
    pub fn refresh(token: &Token, secrets: &[&[u8]], now: OffsetDateTime) -> Option<Token> {
        let user = secrets.iter().find_map(|s| verify(token, s, now).ok())?;
        Some(issue(user, secrets[0], now))
    }

    fn sign(user_id: u64, expires_at: OffsetDateTime, secret: &[u8]) -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("any key length");
        mac.update(&user_id.to_be_bytes());
        mac.update(&expires_at.unix_timestamp().to_be_bytes());
        mac.finalize().into_bytes().to_vec()
    }
  RUST

  PACKAGE_JSON = <<~JSON
    {
      "name": "@acme/design-system",
      "version": "1.8.0",
      "type": "module",
      "scripts": { "build": "tsc -p .", "test": "vitest" }
    }
  JSON

  BUTTON_TSX = <<~TSX
    import { theme } from "../tokens/dark";

    type Variant = "primary" | "ghost" | "danger";

    export function Button({ variant = "primary", children }: { variant?: Variant; children: string }) {
      return <button style={{ background: theme[variant], borderRadius: 3 }}>{children}</button>;
    }
  TSX

  BADGE_TSX = <<~TSX
    import { theme } from "../tokens/dark";

    export function Badge({ label }: { label: string }) {
      return <span style={{ color: theme.accent }}>{label}</span>;
    }
  TSX

  LIGHT_TS = <<~TS
    // Light theme: high contrast on white surfaces.
    export const theme = {
      background: "#ffffff",
      surface: "#f4f4f5",
      foreground: "#18181b",
      muted: "#71717a",
      primary: "#7c3aed",
      accent: "#0891b2",
      ghost: "transparent",
      danger: "#dc2626",
      success: "#16a34a",
      warning: "#ca8a04",
      radius: 3,
    } as const;

    export type Theme = typeof theme;
  TS

  DARK_TS_V1 = LIGHT_TS.sub("// Light theme: high contrast on white surfaces.", "// Dark theme.")
                       .sub('"#ffffff"', '"#000000"').sub('"#18181b"', '"#fafafa"')

  DARK_TS = <<~TS
    // Dark theme: Dracula hues on a deep background.
    export const theme = {
      background: "#282a36",
      surface: "#44475a",
      foreground: "#f8f8f2",
      muted: "#6272a4",
      primary: "#bd93f9",
      accent: "#8be9fd",
      ghost: "transparent",
      danger: "#ff5555",
      success: "#50fa7b",
      warning: "#f1fa8c",
      radius: 3,
    } as const;

    export type Theme = typeof theme;
  TS
}
