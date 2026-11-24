#!/usr/bin/env ruby
# frozen_string_literal: true

# Cuts a release: works out the next version from the changes since the last
# one (major, minor or patch; see version_bump.rb), commits the bump to
# Cargo.toml and Cargo.lock, tags it vX.Y.Z and pushes both to origin's default
# branch. Then it builds the release binary, wraps it in an ad-hoc signed
# "Worktree Browser.app", stops the running copy, swaps the bundle into
# /Applications and relaunches it. Standard library only, like the other
# helper scripts.
#
#   ruby .agents/skills/new-release/release.rb [--plan] [--bump LEVEL] [--co-author TEXT]
#                                              [--no-version [--skip-fetch] [--allow-behind]]
#                                              [--dest DIR] [--no-open]
#
# A versioned release is cut only from a clean checkout of origin's default
# branch, so unreviewed work never ships under a version number. `--no-version`
# installs whatever is checked out instead, without a bump, tag or push.
#
# Quitting the app never loses terminal state: every terminal is a tmux session
# on the `-L worktree-browser` socket, and the relaunched app reattaches to it.

require "fileutils"
require "open3"
require "optparse"
require "time"
require_relative "version_bump"

APP_NAME = "Worktree Browser"
EXECUTABLE = "worktree-browser"
BUNDLE_ID = "local.worktree-browser"
ICON_SIZES = [16, 32, 128, 256, 512].freeze
PACKAGE = "rust-worktree-gui"
DEFAULT_DEST = "/Applications"
# Where releases used to be installed. A bundle left there is moved to the
# Trash, when installing to DEFAULT_DEST, so only one copy of the app exists.
LEGACY_DEST = File.expand_path("~/Desktop")

def step(message) = puts("==> #{message}")

def fail!(message)
  warn "error: #{message}"
  exit 1
end

# Runs a command from an argument vector (never a shell string) and returns stdout.
def run!(*cmd, chdir: Dir.pwd)
  out, err, status = Open3.capture3(*cmd, chdir: chdir)
  fail!("#{cmd.join(' ')} failed:\n#{err}#{out}") unless status.success?
  out
end

def plist(version:, build:)
  <<~XML
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
    	<key>CFBundleDisplayName</key>
    	<string>#{APP_NAME}</string>
    	<key>CFBundleExecutable</key>
    	<string>#{EXECUTABLE}</string>
    	<key>CFBundleIconFile</key>
    	<string>AppIcon</string>
    	<key>CFBundleIdentifier</key>
    	<string>#{BUNDLE_ID}</string>
    	<key>CFBundleName</key>
    	<string>#{APP_NAME}</string>
    	<key>CFBundlePackageType</key>
    	<string>APPL</string>
    	<key>CFBundleShortVersionString</key>
    	<string>#{version.encode(xml: :text)}</string>
    	<key>CFBundleVersion</key>
    	<string>#{build.encode(xml: :text)}</string>
    	<key>LSMinimumSystemVersion</key>
    	<string>12.0</string>
    	<key>NSHighResolutionCapable</key>
    	<true/>
    </dict>
    </plist>
  XML
end

# Renders assets/icon.svg into AppIcon.icns. Falls back to the icon of the
# bundle being replaced when rsvg-convert (Homebrew `librsvg`) is missing.
def build_icon(root, resources, previous_app)
  svg = File.join(root, "assets", "icon.svg")
  target = File.join(resources, "AppIcon.icns")
  if File.exist?(svg) && system("which", "rsvg-convert", out: File::NULL, err: File::NULL)
    iconset = File.join(File.dirname(resources), "AppIcon.iconset")
    FileUtils.mkdir_p(iconset)
    ICON_SIZES.each do |size|
      { "" => size, "@2x" => size * 2 }.each do |suffix, px|
        run!("rsvg-convert", "-w", px.to_s, "-h", px.to_s, svg,
             "-o", File.join(iconset, "icon_#{size}x#{size}#{suffix}.png"))
      end
    end
    run!("iconutil", "-c", "icns", iconset, "-o", target)
    FileUtils.rm_rf(iconset)
  elsif File.exist?(old = File.join(previous_app, "Contents", "Resources", "AppIcon.icns"))
    warn "warning: rsvg-convert not found; reusing the previous bundle's icon"
    FileUtils.cp(old, target)
  else
    warn "warning: no icon (install librsvg for rsvg-convert)"
  end
end

# PIDs of the app running from this exact bundle path. A `cargo run` dev build
# or a copy elsewhere is left alone.
def running_pids(app)
  binary = File.join(app, "Contents", "MacOS", EXECUTABLE)
  run!("ps", "-axo", "pid=,command=").each_line.filter_map do |line|
    pid, command = line.strip.split(" ", 2)
    pid.to_i if command&.start_with?(binary)
  end
end

def wait_until(seconds)
  deadline = Time.now + seconds
  until yield
    return false if Time.now > deadline

    sleep 0.25
  end
  true
end

def stop_app(app)
  pids = running_pids(app)
  return step("#{APP_NAME} is not running") if pids.empty?

  step "Stopping #{APP_NAME} (pid #{pids.join(', ')})"
  # SIGTERM skips the quit confirmation dialog; tmux sessions keep running.
  pids.each { |pid| Process.kill("TERM", pid) rescue Errno::ESRCH }
  return if wait_until(5) { running_pids(app).empty? }

  warn "warning: still running after SIGTERM, sending SIGKILL"
  running_pids(app).each { |pid| Process.kill("KILL", pid) rescue Errno::ESRCH }
  wait_until(3) { running_pids(app).empty? } || fail!("could not stop #{APP_NAME}")
end

# True when `ancestor` is reachable from `descendant` (`git merge-base --is-ancestor`).
def ancestor?(ancestor, descendant, root)
  system("git", "merge-base", "--is-ancestor", ancestor, descendant, chdir: root)
end

# Fetches origin's default branch and makes sure the build includes it:
# up to date → continue; strictly behind with a clean tree → fast-forward
# (no commits can be lost); diverged or dirty → stop unless --allow-behind.
# Returns the default branch's name, e.g. "main".
def sync_with_origin(root, allow_behind)
  # Ask the remote for its default branch; the local refs/remotes/origin/HEAD
  # is only a cached guess and can point at a stale branch.
  symref, = Open3.capture2("git", "ls-remote", "--symref", "origin", "HEAD", chdir: root)
  remote_branch = symref[%r{^ref: refs/heads/(\S+)\s+HEAD}, 1] || "main"
  upstream = "origin/#{remote_branch}"

  step "Fetching #{upstream}"
  _, err, status = Open3.capture3("git", "fetch", "origin", "#{remote_branch}:refs/remotes/#{upstream}", chdir: root)
  fail!("git fetch failed (offline? rerun with --skip-fetch):\n#{err}") unless status.success?

  if ancestor?(upstream, "HEAD", root)
    step "Up to date with #{upstream}"
    return remote_branch
  end

  missing = run!("git", "log", "--oneline", "HEAD..#{upstream}", chdir: root)
  if ancestor?("HEAD", upstream, root) && clean?(root)
    step "Fast-forwarding to #{upstream}:\n#{missing}"
    run!("git", "merge", "--ff-only", upstream, chdir: root)
    return remote_branch
  end

  reason = clean?(root) ? "this branch has diverged from #{upstream}" : "there are uncommitted changes"
  message = "the checkout is behind #{upstream} and #{reason}, so it can't be fast-forwarded. " \
            "Missing commits:\n#{missing}"
  fail!("#{message}Merge or rebase, or rerun with --allow-behind to build it anyway.") unless allow_behind

  warn "warning: building without these #{upstream} commits (--allow-behind):\n#{missing}"
  remote_branch
end

# No uncommitted changes to tracked files (untracked files never reach a commit).
def clean?(root)
  run!("git", "status", "--porcelain", "--untracked-files=no", chdir: root).strip.empty?
end

def rev(name, root) = run!("git", "rev-parse", name, chdir: root).strip

def package_version(cargo_toml)
  cargo_toml[/\A\[package\].*?^version\s*=\s*"([^"]+)"/m, 1] || fail!("no [package] version in Cargo.toml")
end

def version_at(sha, root)
  out, _, status = Open3.capture3("git", "show", "#{sha}:Cargo.toml", chdir: root)
  status.success? ? package_version(out) : nil
end

# The commit the current version was released from: its vX.Y.Z tag, or, for
# versions from before releases were tagged, the main-line commit that set the
# version. That is the release PR's merge, not the release commit on its
# branch: PRs merged to main in between shipped in that release too.
def previous_release(version, root)
  tag = "v#{version}"
  return tag if system("git", "rev-parse", "-q", "--verify", "refs/tags/#{tag}",
                       chdir: root, out: File::NULL, err: File::NULL)

  candidates = run!("git", "log", "--first-parent", "--diff-merges=first-parent", "--no-patch", "--format=%H",
                    "-G", "^version *=", "--", "Cargo.toml", chdir: root).split
  candidates.find { |sha| version_at(sha, root) == version && version_at("#{sha}^", root) != version } ||
    fail!("can't find the commit that released #{version}; tag it as #{tag} and rerun")
end

def commits(range, root, first_parent:)
  args = ["git", "log", "--format=%H%x1f%s%x1f%b%x1e"]
  args << "--first-parent" if first_parent
  run!(*args, range, chdir: root).split("\x1e").filter_map do |record|
    sha, subject, body = record.strip.split("\x1f", 3)
    VersionBump::Commit.new(sha:, subject:, body: body.to_s) if sha
  end
end

# Sets the package version in Cargo.toml and in the package's Cargo.lock entry,
# so `cargo build --locked` accepts the lock file unchanged.
def write_version(root, from, to)
  toml = File.join(root, "Cargo.toml")
  File.write(toml, File.read(toml).sub(/\A(\[package\].*?^version\s*=\s*")#{Regexp.escape(from)}"/m, "\\1#{to}\""))
  lock = File.join(root, "Cargo.lock")
  pattern = /^(name = "#{PACKAGE}"\nversion = ")#{Regexp.escape(from)}"/
  File.read(lock).match?(pattern) || fail!("no #{PACKAGE} #{from} entry in Cargo.lock")
  File.write(lock, File.read(lock).sub(pattern, "\\1#{to}\""))
end

def describe(change)
  label = change.type || "unclassified"
  "#{change.level.to_s.ljust(5)} #{label.ljust(12)} #{change.commit.sha[0, 7]} #{change.commit.subject}"
end

# Moves a bundle to the user's Trash (recoverable), under a unique name.
def trash(app)
  trash_dir = File.expand_path("~/.Trash")
  target = File.join(trash_dir, File.basename(app))
  target = File.join(trash_dir, "#{APP_NAME} #{Time.now.strftime('%Y-%m-%d %H.%M.%S')}.app") if File.exist?(target)
  FileUtils.mv(app, target)
  target
end

dest = DEFAULT_DEST
open_after = true
fetch = true
allow_behind = false
versioned = true
plan_only = false
bump_override = nil
co_author = nil
OptionParser.new do |o|
  o.banner = "usage: release.rb [options]"
  o.on("--plan", "Show the next version and the changes behind it, then stop") { plan_only = true }
  o.on("--bump LEVEL", %w[major minor patch], "Override the computed bump (major, minor or patch)") do |l|
    bump_override = l.to_sym
  end
  o.on("--co-author TEXT", "Add a Co-Authored-By trailer to the release commit") { |t| co_author = t }
  o.on("--no-version", "Install the checkout as it is: no bump, tag or push") { versioned = false }
  o.on("--skip-fetch", "Don't fetch or compare with origin (needs --no-version)") { fetch = false }
  o.on("--allow-behind", "Build even if the checkout is missing origin's commits (needs --no-version)") do
    allow_behind = true
  end
  o.on("--dest DIR", "Folder to install the .app into (default /Applications)") { |d| dest = File.expand_path(d) }
  o.on("--no-open", "Install without relaunching") { open_after = false }
end.parse!

fail!("macOS only") unless RUBY_PLATFORM.include?("darwin")
if versioned && (!fetch || allow_behind)
  fail!("--skip-fetch and --allow-behind only apply to --no-version builds: a versioned release is cut from origin")
end
root = run!("git", "rev-parse", "--show-toplevel").strip
remote_branch = fetch ? sync_with_origin(root, allow_behind) : nil
cargo_version = package_version(File.read(File.join(root, "Cargo.toml")))
release = nil # [new version, changes] when this run cuts a new version

if versioned
  upstream = "origin/#{remote_branch}"
  unless clean?(root) && rev("HEAD", root) == rev(upstream, root)
    fail!("a versioned release is cut from a clean checkout of #{upstream}, and this checkout " \
          "#{clean?(root) ? 'has commits that are not on it' : 'has uncommitted changes'}. " \
          "Land the work on #{remote_branch} first, or pass --no-version to install this checkout unversioned.")
  end

  base = previous_release(cargo_version, root)
  changes, level = VersionBump.classify(commits("#{base}..HEAD", root, first_parent: true),
                                        commits("#{base}..HEAD", root, first_parent: false))
  if level.nil? && bump_override.nil?
    step "No changes since #{cargo_version} (#{base[0, 12]}); reinstalling it without a new version"
  else
    level = bump_override || level
    new_version = VersionBump.bump(cargo_version, level)
    step "Changes since #{cargo_version} (#{base[0, 12]}):\n#{changes.map { |c| "    #{describe(c)}" }.join("\n")}"
    step "Next version: #{cargo_version} -> #{new_version} (#{level}#{bump_override ? ', from --bump' : ''})"
    release = [new_version, changes]
  end
end
exit 0 if plan_only

if release
  new_version, changes = release
  tag = "v#{new_version}"
  local_tag = system("git", "rev-parse", "-q", "--verify", "refs/tags/#{tag}",
                     chdir: root, out: File::NULL, err: File::NULL)
  remote_tag = !run!("git", "ls-remote", "--tags", "origin", "refs/tags/#{tag}", chdir: root).strip.empty?
  fail!("tag #{tag} already exists#{remote_tag ? ' on origin' : ' locally'}") if local_tag || remote_tag

  step "Setting the version to #{new_version} in Cargo.toml and Cargo.lock"
  write_version(root, cargo_version, new_version)
  cargo_version = new_version
end

step "Building release binary in #{root}"
unless system("cargo", "build", "--release", "--locked", chdir: root)
  run!("git", "checkout", "--", "Cargo.toml", "Cargo.lock", chdir: root) if release
  fail!("cargo build failed")
end

if release
  # Commit, tag and push before installing: the installed bundle then names a
  # commit that exists on origin.
  original = rev("HEAD", root)
  notes = changes.map { |c| "- #{c.commit.subject} (#{c.commit.sha[0, 7]})" }.join("\n")
  message = "Release #{new_version}\n\n#{notes}\n"
  message += "\nCo-Authored-By: #{co_author}\n" if co_author
  step "Committing #{message.lines.first.strip} and tagging #{tag}"
  run!("git", "commit", "--quiet", "-m", message, "--", "Cargo.toml", "Cargo.lock", chdir: root)
  run!("git", "tag", "-a", tag, "-m", "#{APP_NAME} #{new_version}\n\n#{notes}", chdir: root)

  step "Pushing #{remote_branch} and #{tag} to origin"
  # --atomic: either both the branch and the tag land, or neither does.
  _, err, status = Open3.capture3("git", "push", "--atomic", "origin",
                                  "HEAD:refs/heads/#{remote_branch}", "refs/tags/#{tag}", chdir: root)
  unless status.success?
    # Undo the local commit and tag so a rerun starts from a clean slate.
    run!("git", "tag", "-d", tag, chdir: root)
    run!("git", "reset", "--keep", original, chdir: root)
    fail!("git push failed, so nothing was released (#{remote_branch} may have moved; rerun):\n#{err}")
  end

  # The window title comes from the commit and tags `build.rs` saw. The build
  # above ran before the release commit and tag existed, so rebuild: only this
  # crate recompiles, and the title then shows just the new version.
  step "Rebuilding #{tag} so the window title shows the release"
  run!("cargo", "build", "--release", "--locked", chdir: root)
end

sha = run!("git", "rev-parse", "--short", "HEAD", chdir: root).strip
branch = run!("git", "branch", "--show-current", chdir: root).strip
branch = "detached" if branch.empty?
build = clean?(root) ? sha : "#{sha}-dirty"
version = versioned ? "#{cargo_version} (#{build})" : "#{cargo_version} (#{branch} #{build})"
installed = File.join(dest, "#{APP_NAME}.app")
legacy = File.join(LEGACY_DEST, "#{APP_NAME}.app")
legacy = nil unless dest == DEFAULT_DEST && File.exist?(legacy)

stage_dir = File.join(root, "target", "release", "bundle")
staged = File.join(stage_dir, "#{APP_NAME}.app")
step "Assembling #{staged}"
FileUtils.rm_rf(stage_dir)
macos = File.join(staged, "Contents", "MacOS")
resources = File.join(staged, "Contents", "Resources")
FileUtils.mkdir_p([macos, resources])
FileUtils.cp(File.join(root, "target", "release", PACKAGE), File.join(macos, EXECUTABLE))
File.write(File.join(staged, "Contents", "Info.plist"), plist(version: version, build: build))
build_icon(root, resources, File.exist?(installed) ? installed : legacy.to_s)
run!("codesign", "--force", "--sign", "-", staged)
run!("codesign", "--verify", "--strict", staged)

stop_app(installed)
stop_app(legacy) if legacy

if File.exist?(installed)
  # Keep exactly one rollback copy, outside the destination folder.
  backup = File.join(root, "target", "release", "previous", "#{APP_NAME}.app")
  step "Backing up the old bundle to #{backup}"
  FileUtils.rm_rf(File.dirname(backup))
  FileUtils.mkdir_p(File.dirname(backup))
  FileUtils.mv(installed, backup)
end
step "Moved the old #{legacy} to #{trash(legacy)}" if legacy

step "Installing #{installed}"
FileUtils.mkdir_p(dest)
run!("ditto", staged, installed) # ditto preserves the code signature and xattrs

if open_after
  step "Launching #{APP_NAME}"
  run!("open", installed)
  wait_until(10) { running_pids(installed).any? } ||
    fail!("#{APP_NAME} did not start; try `open \"#{installed}\"` and check Console.app")
  step "Running as pid #{running_pids(installed).join(', ')}"
end

puts "Released #{tag} and pushed it with #{remote_branch} to origin" if release
puts "Installed #{APP_NAME} #{version}"
