#!/usr/bin/env ruby
# frozen_string_literal: true

# Builds a release binary, wraps it in an ad-hoc signed "Worktree Browser.app",
# stops the copy running from the destination folder, swaps the bundle in and
# relaunches it. Standard library only, like the other helper scripts.
#
#   ruby .claude/skills/new-release/release.rb [--dest DIR] [--no-open] [--skip-fetch] [--allow-behind]
#
# Quitting the app never loses terminal state: every terminal is a tmux session
# on the `-L worktree-browser` socket, and the relaunched app reattaches to it.

require "fileutils"
require "open3"
require "optparse"
require "time"

APP_NAME = "Worktree Browser"
EXECUTABLE = "worktree-browser"
BUNDLE_ID = "local.worktree-browser"
ICON_SIZES = [16, 32, 128, 256, 512].freeze

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
def sync_with_origin(root, allow_behind)
  # Ask the remote for its default branch; the local refs/remotes/origin/HEAD
  # is only a cached guess and can point at a stale branch.
  symref, = Open3.capture2("git", "ls-remote", "--symref", "origin", "HEAD", chdir: root)
  remote_branch = symref[%r{^ref: refs/heads/(\S+)\s+HEAD}, 1] || "main"
  upstream = "origin/#{remote_branch}"

  step "Fetching #{upstream}"
  _, err, status = Open3.capture3("git", "fetch", "origin", "#{remote_branch}:refs/remotes/#{upstream}", chdir: root)
  fail!("git fetch failed (offline? rerun with --skip-fetch):\n#{err}") unless status.success?

  return step("Up to date with #{upstream}") if ancestor?(upstream, "HEAD", root)

  missing = run!("git", "log", "--oneline", "HEAD..#{upstream}", chdir: root)
  clean = run!("git", "status", "--porcelain", "--untracked-files=no", chdir: root).strip.empty?
  if ancestor?("HEAD", upstream, root) && clean
    step "Fast-forwarding to #{upstream}:\n#{missing}"
    run!("git", "merge", "--ff-only", upstream, chdir: root)
    return
  end

  reason = clean ? "this branch has diverged from #{upstream}" : "there are uncommitted changes"
  message = "the checkout is behind #{upstream} and #{reason}, so it can't be fast-forwarded. " \
            "Missing commits:\n#{missing}"
  fail!("#{message}Merge or rebase, or rerun with --allow-behind to build it anyway.") unless allow_behind

  warn "warning: building without these #{upstream} commits (--allow-behind):\n#{missing}"
end

dest = File.expand_path("~/Desktop")
open_after = true
fetch = true
allow_behind = false
OptionParser.new do |o|
  o.banner = "usage: release.rb [--dest DIR] [--no-open] [--skip-fetch] [--allow-behind]"
  o.on("--dest DIR", "Folder to install the .app into (default ~/Desktop)") { |d| dest = File.expand_path(d) }
  o.on("--no-open", "Install without relaunching") { open_after = false }
  o.on("--skip-fetch", "Don't fetch or compare with origin (offline builds)") { fetch = false }
  o.on("--allow-behind", "Build even if the checkout is missing origin's commits") { allow_behind = true }
end.parse!

fail!("macOS only") unless RUBY_PLATFORM.include?("darwin")
root = run!("git", "rev-parse", "--show-toplevel").strip
sync_with_origin(root, allow_behind) if fetch
cargo_version = File.read(File.join(root, "Cargo.toml"))[/^version\s*=\s*"([^"]+)"/, 1] || "0.0.0"
sha = run!("git", "rev-parse", "--short", "HEAD", chdir: root).strip
branch = run!("git", "branch", "--show-current", chdir: root).strip
branch = "detached" if branch.empty?
dirty = !run!("git", "status", "--porcelain", chdir: root).strip.empty?
build = dirty ? "#{sha}-dirty" : sha
version = "#{cargo_version} (#{branch} #{build})"
installed = File.join(dest, "#{APP_NAME}.app")

step "Building release binary in #{root}"
system("cargo", "build", "--release", "--locked", chdir: root) || fail!("cargo build failed")

stage_dir = File.join(root, "target", "release", "bundle")
staged = File.join(stage_dir, "#{APP_NAME}.app")
step "Assembling #{staged}"
FileUtils.rm_rf(stage_dir)
macos = File.join(staged, "Contents", "MacOS")
resources = File.join(staged, "Contents", "Resources")
FileUtils.mkdir_p([macos, resources])
FileUtils.cp(File.join(root, "target", "release", "rust-worktree-gui"), File.join(macos, EXECUTABLE))
File.write(File.join(staged, "Contents", "Info.plist"), plist(version: version, build: build))
build_icon(root, resources, installed)
run!("codesign", "--force", "--sign", "-", staged)
run!("codesign", "--verify", "--strict", staged)

stop_app(installed)

if File.exist?(installed)
  # Keep exactly one rollback copy, outside the destination folder.
  backup = File.join(root, "target", "release", "previous", "#{APP_NAME}.app")
  step "Backing up the old bundle to #{backup}"
  FileUtils.rm_rf(File.dirname(backup))
  FileUtils.mkdir_p(File.dirname(backup))
  FileUtils.mv(installed, backup)
end

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

puts "Installed #{APP_NAME} #{version}"
