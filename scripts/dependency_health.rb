#!/usr/bin/env ruby
# frozen_string_literal: true

# Dependency health report: flags direct dependencies that are falling far
# behind or look out of support, and a Rust toolchain pin that has aged out.
#
# Dependabot (.github/dependabot.yml) proposes individual upgrades. This
# report answers a different question: "is anything getting dangerously
# old?" It runs weekly in .github/workflows/dependency-health.yml, which
# opens (or updates) a GitHub issue while there are findings.
#
#   ruby scripts/dependency_health.rb                 # print the report
#   ruby scripts/dependency_health.rb --output r.md   # also write it to a file
#
# Needs only the Ruby standard library, `cargo` on PATH and network access to
# crates.io and static.rust-lang.org. When GITHUB_OUTPUT is set (GitHub
# Actions), it writes `findings=<count>` there.

require "date"
require "json"
require "net/http"
require "open3"
require "optparse"
require "time"
require "uri"

module DependencyHealth
  # A dependency is "majorly out of date" when it is this many breaking
  # release lines behind (1.x -> 3.x, or 0.14 -> 0.16 for pre-1.0 crates)...
  MAJOR_LAG_LIMIT = 2
  # ...or one breaking line behind and that newer line has been out this long.
  STALE_MAJOR_DAYS = 180
  # No release of any kind for this long suggests the crate is unmaintained.
  # Small, finished crates (raw-window-handle, directories) can be quiet for a
  # while, so a shorter silence only puts the crate on the report's watch list.
  ABANDONED_DAYS = 1095 # 3 years
  QUIET_DAYS = 548 # 18 months
  # Rust has no LTS: only the latest stable gets fixes. Flag the pin once it
  # is this many 6-weekly releases behind (~6 months).
  TOOLCHAIN_LAG_LIMIT = 4

  # crates.io asks automated clients to stay at or under one request a second
  # and to send a descriptive User-Agent.
  CRATES_IO = "https://crates.io/api/v1/crates/"
  RUST_STABLE_MANIFEST = "https://static.rust-lang.org/dist/channel-rust-stable.toml"
  USER_AGENT = "dependency-health-check/1.0 (ruby net/http)"
  REQUEST_INTERVAL = 1.0

  # A parsed SemVer version. Comparable gives <, >, == etc. from <=>.
  Version = Struct.new(:major, :minor, :patch, :pre) do
    include Comparable

    def self.parse(text)
      match = /\A(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?\z/.match(text.to_s.strip)
      return nil unless match

      new(match[1].to_i, match[2].to_i, match[3].to_i, match[4])
    end

    def prerelease? = !pre.nil?

    # Cargo's compatibility rule: the leftmost non-zero component is the
    # "major" one. 1.2.3 and 1.9.0 are compatible; 0.14.1 and 0.15.0 are not;
    # 0.0.3 and 0.0.4 are not. Two versions with the same line are
    # semver-compatible, so a new line means a breaking release.
    def compat_line
      if major.positive? then [major, 0, 0]
      elsif minor.positive? then [0, minor, 0]
      else [0, 0, patch]
      end
    end

    def <=>(other)
      [major, minor, patch, prerelease? ? 0 : 1] <=> [other.major, other.minor, other.patch, other.prerelease? ? 0 : 1]
    end

    def to_s = "#{major}.#{minor}.#{patch}#{"-#{pre}" if pre}"
  end

  # One published version of a crate, as crates.io reports it.
  Release = Struct.new(:version, :created_at, :yanked)

  # What we know about one direct dependency.
  Dependency = Struct.new(:name, :current, :vendored, :releases, keyword_init: true) do
    def stable_releases
      releases.reject { |r| r.yanked || r.version.prerelease? }
    end

    def latest = stable_releases.max_by(&:version)

    def current_release = releases.find { |r| r.version == current }

    # Breaking release lines newer than the one in Cargo.lock.
    def newer_lines
      stable_releases
        .select { |r| (r.version.compat_line <=> current.compat_line).positive? }
        .group_by { |r| r.version.compat_line }
    end

    def lines_behind = newer_lines.size

    # When the oldest newer breaking line first shipped, i.e. how long an
    # upgrade has been available.
    def next_line_since
      first_line = newer_lines.keys.min
      first_line && newer_lines[first_line].map(&:created_at).min
    end

    def last_release_at = releases.map(&:created_at).max
  end

  Finding = Struct.new(:subject, :message)

  module_function

  # Returns an array of Findings for one dependency. `now` is injectable so
  # tests don't depend on the clock.
  def assess(dep, now: Time.now.utc)
    findings = []
    days = ->(time) { ((now - time) / 86_400).floor }

    if dep.current_release&.yanked
      findings << Finding.new(dep.name, "#{dep.current} is yanked on crates.io. Upgrade or pin a non-yanked release.")
    end

    behind = dep.lines_behind
    since = dep.next_line_since
    if behind >= MAJOR_LAG_LIMIT
      findings << Finding.new(dep.name, "#{behind} breaking releases behind (#{dep.current} -> #{dep.latest.version}).")
    elsif behind == 1 && since && days.call(since) >= STALE_MAJOR_DAYS
      findings << Finding.new(dep.name, "a breaking release behind for #{days.call(since)} days (#{dep.current} -> #{dep.latest.version}).")
    end

    last = dep.last_release_at
    if last && days.call(last) >= ABANDONED_DAYS
      findings << Finding.new(dep.name, "no release for #{days.call(last)} days (last #{last.to_date}). It is probably unmaintained; check for a RustSec notice or a successor.")
    end

    findings
  end

  # Returns a Finding when the pinned toolchain is too far behind stable.
  def assess_toolchain(pinned, stable)
    return nil unless pinned && stable

    behind = stable.major == pinned.major ? stable.minor - pinned.minor : TOOLCHAIN_LAG_LIMIT
    return nil if behind < TOOLCHAIN_LAG_LIMIT

    Finding.new("Rust toolchain",
                "pinned #{pinned} is #{behind} releases behind stable #{stable}. Only the latest stable " \
                "receives fixes. Update `.tool-versions`, `rust-version` in Cargo.toml and `RUST_VERSION` in CI.")
  end

  # --- Data collection (I/O) -------------------------------------------------

  # Direct dependencies of the root package with their locked versions.
  # Path dependencies (the vendored iced_term) are kept and marked, so the
  # report still shows how far the vendored copy is behind upstream.
  def direct_dependencies(manifest_dir)
    out, err, status = Open3.capture3("cargo", "metadata", "--format-version", "1", "--locked", chdir: manifest_dir)
    raise "cargo metadata failed: #{err.strip}" unless status.success?

    meta = JSON.parse(out)
    packages = meta.fetch("packages").to_h { |pkg| [pkg.fetch("id"), pkg] }
    root = meta.dig("resolve", "root") or raise "cargo metadata has no root package"
    node = meta.dig("resolve", "nodes").find { |n| n.fetch("id") == root }

    node.fetch("deps").map { |dep| packages.fetch(dep.fetch("pkg")) }.uniq { |pkg| pkg.fetch("name") }.map do |pkg|
      { name: pkg.fetch("name"), version: Version.parse(pkg.fetch("version")), vendored: pkg["source"].nil? }
    end
  end

  # GET with a few retries on network errors, rate limiting and 5xx answers.
  def http_get(url, attempts: 3)
    uri = URI(url)
    attempts.times do |attempt|
      last = attempt == attempts - 1
      sleep(REQUEST_INTERVAL * 2 * attempt) if attempt.positive?
      begin
        response = Net::HTTP.start(uri.host, uri.port, use_ssl: true, open_timeout: 10, read_timeout: 30) do |http|
          http.get(uri.request_uri, "User-Agent" => USER_AGENT, "Accept" => "application/json")
        end
      rescue Net::OpenTimeout, Net::ReadTimeout, SocketError, SystemCallError, OpenSSL::SSL::SSLError
        raise if last

        next
      end
      return response.body if response.is_a?(Net::HTTPSuccess)

      retryable = response.code == "429" || response.code.start_with?("5")
      raise "GET #{url}: HTTP #{response.code}" if last || !retryable
    end
  end

  def crate_releases(name)
    body = JSON.parse(http_get("#{CRATES_IO}#{URI.encode_www_form_component(name)}"))
    body.fetch("versions").filter_map do |v|
      version = Version.parse(v.fetch("num"))
      version && Release.new(version, Time.parse(v.fetch("created_at")), v.fetch("yanked"))
    end
  end

  def pinned_toolchain(manifest_dir)
    path = File.join(manifest_dir, ".tool-versions")
    return nil unless File.exist?(path)

    line = File.readlines(path).find { |l| l.start_with?("rust ") }
    line && Version.parse(line.split[1])
  end

  def stable_toolchain
    manifest = http_get(RUST_STABLE_MANIFEST)
    version = manifest[/^\[pkg\.rustc\]\s*\nversion = "(\d+\.\d+\.\d+)/, 1]
    version && Version.parse(version)
  end

  # Advisories deliberately ignored in deny.toml. They pass CI, so list them
  # here to keep them visible until the reason no longer holds.
  def accepted_advisories(manifest_dir)
    path = File.join(manifest_dir, "deny.toml")
    return [] unless File.exist?(path)

    File.read(path).scan(/id\s*=\s*"(RUSTSEC-[\d-]+)"\s*,\s*reason\s*=\s*"([^"]*)"/)
  end

  # --- Report ----------------------------------------------------------------

  def render(findings:, deps:, pinned:, stable:, advisories:, errors:, now: Time.now.utc)
    lines = ["# Dependency health report", ""]
    lines << "Generated by `scripts/dependency_health.rb`. Thresholds: #{MAJOR_LAG_LIMIT}+ breaking releases behind, " \
             "1 behind for #{STALE_MAJOR_DAYS}+ days, no release for #{ABANDONED_DAYS}+ days, " \
             "toolchain #{TOOLCHAIN_LAG_LIMIT}+ releases behind stable."
    lines << ""

    if findings.empty?
      lines << "## Needs attention" << "" << "Nothing. Every direct dependency and the toolchain are within limits." << ""
    else
      lines << "## Needs attention (#{findings.size})" << ""
      findings.each { |f| lines << "- **#{f.subject}**: #{f.message}" }
      lines << ""
    end

    unless errors.empty?
      lines << "## Could not check" << ""
      errors.each { |e| lines << "- #{e}" }
      lines << ""
    end

    quiet = deps.select do |dep|
      age = dep.last_release_at && ((now - dep.last_release_at) / 86_400).floor
      age && age >= QUIET_DAYS && age < ABANDONED_DAYS
    end
    unless quiet.empty?
      lines << "## Watch list" << ""
      lines << "No release for #{QUIET_DAYS}+ days. Often fine for small, finished crates; it becomes a finding at #{ABANDONED_DAYS} days." << ""
      quiet.sort_by(&:name).each { |dep| lines << "- #{dep.name}: last release #{dep.last_release_at.to_date}" }
      lines << ""
    end

    lines << "## Rust toolchain" << ""
    lines << "Pinned `#{pinned || "unknown"}`, latest stable `#{stable || "unknown"}`." << ""

    unless advisories.empty?
      lines << "## Accepted advisories (deny.toml)" << ""
      lines << "These are ignored by `cargo deny`. Recheck each reason when its dependency chain changes." << ""
      advisories.each { |id, reason| lines << "- [#{id}](https://rustsec.org/advisories/#{id}.html): #{reason}" }
      lines << ""
    end

    lines << "## Direct dependencies" << ""
    lines << "| Crate | Locked | Latest stable | Breaking releases behind | Last release |"
    lines << "| --- | --- | --- | --- | --- |"
    deps.sort_by(&:name).each do |dep|
      name = dep.vendored ? "#{dep.name} (vendored)" : dep.name
      lines << "| #{name} | #{dep.current} | #{dep.latest&.version || "?"} | #{dep.lines_behind} | #{dep.last_release_at&.to_date || "?"} |"
    end
    lines << ""
    lines.join("\n")
  end

  def run(argv)
    options = { manifest_dir: Dir.pwd, output: nil }
    OptionParser.new do |o|
      o.banner = "Usage: dependency_health.rb [--manifest-dir DIR] [--output FILE]"
      o.on("--manifest-dir DIR", "Directory holding Cargo.toml (default: cwd)") { |v| options[:manifest_dir] = v }
      o.on("--output FILE", "Also write the Markdown report to FILE") { |v| options[:output] = v }
    end.parse!(argv)

    dir = options[:manifest_dir]
    errors = []
    deps = direct_dependencies(dir).filter_map do |dep|
      sleep(REQUEST_INTERVAL)
      Dependency.new(name: dep[:name], current: dep[:version], vendored: dep[:vendored], releases: crate_releases(dep[:name]))
    rescue StandardError => e
      errors << "#{dep[:name]}: #{e.message}"
      nil
    end

    pinned = pinned_toolchain(dir)
    stable = begin
      stable_toolchain
    rescue StandardError => e
      errors << "Rust stable version: #{e.message}"
      nil
    end

    findings = deps.flat_map { |dep| assess(dep) }
    findings << assess_toolchain(pinned, stable)
    findings.compact!

    report = render(findings:, deps:, pinned:, stable:, advisories: accepted_advisories(dir), errors:)
    puts report
    File.write(options[:output], report) if options[:output]
    if (gh_output = ENV.fetch("GITHUB_OUTPUT", nil))
      File.open(gh_output, "a") { |f| f.puts "findings=#{findings.size}", "errors=#{errors.size}" }
    end
    0
  rescue StandardError => e
    warn "dependency_health: #{e.message}"
    2
  end
end

exit DependencyHealth.run(ARGV) if $PROGRAM_NAME == __FILE__
