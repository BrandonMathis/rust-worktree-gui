#!/usr/bin/env ruby
# frozen_string_literal: true

# Keystroke latency check (NFR-11): how long a key press takes to change the
# pixels on screen, for this checkout ("head") against a base revision.
#
# The measuring is done by the app itself, built with `--features perf`
# (src/perf/): it drives the real app headlessly, types into the embedded
# terminal and the Rename field, and times every keystroke until the frame
# changes. This script builds both revisions, runs them in alternating rounds
# on the same machine (so drift and runner differences cancel out), and fails
# when head is significantly slower. .github/workflows/latency.yml runs it on
# every pull request.
#
#   ruby scripts/latency.rb                         # HEAD against origin/main
#   ruby scripts/latency.rb --base-ref main~3       # against another revision
#   ruby scripts/latency.rb --head-only             # just measure this checkout
#   ruby scripts/latency.rb --base-bin a --head-bin b   # prebuilt binaries
#
# Results go to target/latency/ (JSON per run, report.md). Exit status: 0 when
# nothing regressed, 1 on a regression, 2 when the check could not run. With
# GITHUB_STEP_SUMMARY set (GitHub Actions) the report is appended there.
#
# Only the Ruby standard library, git and cargo are needed.

require "fileutils"
require "json"
require "open3"
require "optparse"
require "tmpdir"

module Latency
  SCHEMA = 1

  # When a metric counts as regressed. A slowdown must be real (statistically
  # significant, seen in every round, bigger than the floor below) and large:
  # more than MAX_INCREASE_PCT slower, or shifted by more than MAX_SIGMA of
  # the base build's own spread.
  Thresholds = Struct.new(:max_increase_pct, :max_sigma, :alpha, keyword_init: true)
  DEFAULT_THRESHOLDS = Thresholds.new(max_increase_pct: 15.0, max_sigma: 3.0, alpha: 0.01)

  # The metrics that can fail the check, with the smallest slowdown (in ms)
  # that may: below it, a change is timer and scheduler noise whatever the
  # percentage says. `app_ms` is about 0.5 ms a keystroke, so its floor is
  # low: drawing every terminal glyph twice adds only ~0.15 ms there (+30%).
  GATED = {
    "total_ms" => 0.5, # key press to changed pixels
    "app_ms" => 0.05   # the app's own work: update + view + draw
  }.freeze

  # Every metric in a report, in display order (the phases add up to total).
  METRICS = {
    "total_ms" => "key → pixels",
    "app_ms" => "app (update+view+draw)",
    "update_ms" => "update",
    "view_ms" => "view",
    "draw_ms" => "draw",
    "raster_ms" => "render + readback",
    "wait_ms" => "PTY wait"
  }.freeze

  module_function

  def median(values) = percentile(values, 0.5)

  # Linear-interpolated quantile, `fraction` in 0..1.
  def percentile(values, fraction)
    sorted = values.sort
    return Float::NAN if sorted.empty?

    position = (sorted.size - 1) * fraction
    low = sorted[position.floor]
    high = sorted[position.ceil]
    low + ((high - low) * (position - position.floor))
  end

  # A standard deviation that a few outliers (a GC pause, a context switch)
  # can't inflate: 1.4826 × the median absolute deviation, which equals the
  # standard deviation for normally distributed samples. Falls back to the
  # plain standard deviation when more than half the samples are identical.
  def robust_sigma(values)
    center = median(values)
    mad = median(values.map { |v| (v - center).abs })
    return 1.4826 * mad if mad.positive?

    mean = values.sum / values.size.to_f
    Math.sqrt(values.sum { |v| (v - mean)**2 } / [values.size - 1, 1].max)
  end

  # One-sided Mann–Whitney U test: the p-value for "samples in `slower` tend
  # to be larger than those in `faster`". It compares ranks, not means, so it
  # suits skewed latency distributions. Normal approximation with tie and
  # continuity corrections (fine for the 30+ samples a run has).
  def mann_whitney_p(slower, faster)
    n1 = slower.size
    n2 = faster.size
    return 1.0 if n1.zero? || n2.zero?

    tagged = slower.map { |v| [v, 0] } + faster.map { |v| [v, 1] }
    tagged.sort_by!(&:first)
    n = tagged.size
    rank_sum = 0.0
    ties = 0.0
    i = 0
    while i < n
      j = i
      j += 1 while j + 1 < n && tagged[j + 1][0] == tagged[i][0]
      average_rank = ((i + j) / 2.0) + 1
      count = j - i + 1
      ties += (count**3) - count
      (i..j).each { |k| rank_sum += average_rank if tagged[k][1].zero? }
      i = j + 1
    end
    u = rank_sum - (n1 * (n1 + 1) / 2.0)
    variance = n1 * n2 / 12.0 * ((n + 1) - (ties / (n * (n - 1))))
    return 0.5 unless variance.positive?

    z = (u - (n1 * n2 / 2.0) - 0.5) / Math.sqrt(variance)
    0.5 * Math.erfc(z / Math.sqrt(2))
  end

  # The comparison of one metric of one scenario.
  Finding = Struct.new(
    :scenario, :metric, :base_median, :head_median, :change_pct, :sigmas,
    :p_value, :consistent, :status, :reasons, keyword_init: true
  ) do
    def gated? = GATED.key?(metric)
    def regression? = status == :regression
  end

  def load_report(path)
    report = JSON.parse(File.read(path))
    unless report["schema"] == SCHEMA
      raise ArgumentError, "#{path}: report schema #{report["schema"].inspect}, expected #{SCHEMA}"
    end

    report
  end

  # Compares paired rounds of base and head reports (round i of each ran back
  # to back). Returns a Finding for every metric of every scenario both have.
  def compare(base_runs, head_runs, thresholds = DEFAULT_THRESHOLDS)
    renderers = (base_runs + head_runs).map { |r| r["renderer"] }.uniq
    if renderers.size > 1
      raise ArgumentError, "runs used different renderers (#{renderers.join(", ")}); they can't be compared"
    end

    names = head_runs.first["scenarios"].map { |s| s["name"] }
    names.select! { |name| base_runs.first["scenarios"].any? { |s| s["name"] == name } }
    names.flat_map do |name|
      rounds = base_runs.zip(head_runs).map do |base, head|
        [scenario(base, name), scenario(head, name)]
      end
      METRICS.keys.map { |metric| assess(name, metric, rounds, thresholds) }
    end
  end

  def scenario(report, name)
    report["scenarios"].find { |s| s["name"] == name } or
      raise ArgumentError, "a run is missing scenario #{name}"
  end

  def assess(name, metric, rounds, thresholds)
    base = rounds.flat_map { |b, _| b.fetch(metric) }
    head = rounds.flat_map { |_, h| h.fetch(metric) }
    base_median = median(base)
    head_median = median(head)
    delta = head_median - base_median
    change_pct = base_median.positive? ? delta / base_median * 100 : 0.0
    sigma = robust_sigma(base)
    sigmas = sigma.positive? ? delta / sigma : 0.0
    p_value = mann_whitney_p(head, base)
    consistent = rounds.all? { |b, h| median(h.fetch(metric)) > median(b.fetch(metric)) }

    reasons = []
    reasons << format("%+.1f%% > %.0f%%", change_pct, thresholds.max_increase_pct) if change_pct > thresholds.max_increase_pct
    reasons << format("%+.1fσ > %.0fσ", sigmas, thresholds.max_sigma) if sigmas > thresholds.max_sigma
    real = p_value < thresholds.alpha && delta > GATED.fetch(metric, 0.0)
    status =
      if !GATED.key?(metric) || reasons.empty? || !real
        :ok
      elsif consistent
        :regression
      else
        # Large and significant overall, but not slower in every round: a
        # transient slowdown on the runner, more likely than a real one.
        reasons << "not slower in every round"
        :warning
      end
    Finding.new(scenario: name, metric:, base_median:, head_median:, change_pct:, sigmas:,
                p_value:, consistent:, status:, reasons:)
  end

  def ms(value) = format("%.2f ms", value)

  def verdict(finding)
    case finding.status
    when :regression then "❌ #{finding.reasons.join(", ")}"
    when :warning then "⚠️ #{finding.reasons.join(", ")}"
    else "✅"
    end
  end

  def markdown(findings, base_runs:, head_runs:, base_label:, head_label:, thresholds: DEFAULT_THRESHOLDS)
    head = head_runs.first
    window = head["window"]
    lines = ["## Keystroke latency", ""]
    regressions = findings.count(&:regression?)
    lines << (regressions.zero? ? "No regressions." : "**#{regressions} regression#{"s" unless regressions == 1}.**")
    lines << ""
    lines << "#{base_label} → #{head_label}. Renderer #{head["renderer"]}, " \
             "window #{window["width"].to_i}×#{window["height"].to_i} @#{window["scale"]}x, " \
             "#{head_runs.size} rounds × #{head["samples_per_scenario"]} keystrokes per scenario per build, base and head taking turns."
    lines << ""
    lines << "| Scenario | Metric | Base median | Head median | Change | Shift | p | |"
    lines << "| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |"
    findings.select(&:gated?).each do |f|
      lines << "| #{f.scenario} | #{METRICS[f.metric]} | #{ms(f.base_median)} | #{ms(f.head_median)} | " \
               "#{format("%+.1f%%", f.change_pct)} | #{format("%+.1fσ", f.sigmas)} | " \
               "#{format("%.3f", f.p_value)} | #{verdict(f)} |"
    end
    lines << ""
    lines << "A metric fails when head is slower by more than #{thresholds.max_increase_pct.round}% " \
             "or #{thresholds.max_sigma.round}σ of base's spread (σ = 1.4826 × MAD), and the slowdown is real: " \
             "Mann–Whitney p < #{thresholds.alpha}, slower in every round, and at least " \
             "#{GATED.map { |m, floor| "#{floor} ms (#{METRICS[m]})" }.join(" / ")}."
    lines << ""
    lines << "<details><summary>Median of every phase</summary>"
    lines << ""
    lines << "| Scenario | #{METRICS.values.join(" | ")} |"
    lines << "| --- |#{" ---: |" * METRICS.size}"
    findings.group_by(&:scenario).each do |name, rows|
      cells = rows.map { |f| "#{format("%.2f", f.base_median)} → #{format("%.2f", f.head_median)}" }
      lines << "| #{name} | #{cells.join(" | ")} |"
    end
    lines << ""
    lines << "</details>"
    lines.join("\n") + "\n"
  end

  # A report of one build alone: when there is no base to compare with.
  def head_only_markdown(head_runs, note)
    runs = head_runs
    head = runs.first
    lines = ["## Keystroke latency", "", note, "",
             "Renderer #{head["renderer"]}, #{runs.size} round#{"s" unless runs.size == 1} × " \
             "#{head["samples_per_scenario"]} keystrokes per scenario.", "",
             "| Scenario | Key → pixels median | p95 | App median |",
             "| --- | ---: | ---: | ---: |"]
    head["scenarios"].each do |s|
      total = runs.flat_map { |r| scenario(r, s["name"])["total_ms"] }
      app = runs.flat_map { |r| scenario(r, s["name"])["app_ms"] }
      lines << "| #{s["name"]} | #{ms(median(total))} | #{ms(percentile(total, 0.95))} | #{ms(median(app))} |"
    end
    lines.join("\n") + "\n"
  end

  # Whether a checkout has the harness: its Cargo.toml declares the feature.
  def harness_in?(cargo_toml) = cargo_toml.match?(/^perf\s*=/)

  # ----- Building and running -------------------------------------------------

  ROOT = File.expand_path("..", __dir__)
  BINARY = "rust-worktree-gui"

  def sh!(*argv, chdir: ROOT, env: {})
    puts "$ #{argv.join(" ")}"
    system(env, *argv, chdir:, exception: false) or raise "command failed: #{argv.join(" ")}"
  end

  # Builds `source` (a checkout) in release mode with the harness and copies
  # the binary to `destination`. One shared target directory means the second
  # build reuses the first one's dependencies.
  def build(source, destination)
    sh!("cargo", "build", "--release", "--locked", "--features", "perf",
        "--manifest-path", File.join(source, "Cargo.toml"),
        env: { "CARGO_TARGET_DIR" => File.join(ROOT, "target") })
    FileUtils.cp(File.join(ROOT, "target", "release", BINARY), destination)
  end

  def measure(binary, out, args)
    sh!(binary, "--perf-latency", "--out", out, *args)
    load_report(out)
  end

  # Re-runs the comparison on the per-scenario JSON files a run saved
  # (`<side>-<round>-<scenario>.json`).
  def report_saved(dir, options)
    runs = Hash.new { |h, k| h[k] = Hash.new { |r, n| r[n] = [] } }
    Dir.glob(File.join(dir, "{base,head}-*-*.json")).sort.each do |path|
      side, round = File.basename(path, ".json").split("-", 3)
      runs[side][round.to_i] << load_report(path)
    end
    raise ArgumentError, "no saved base and head runs in #{dir}" unless runs.key?("base") && runs.key?("head")

    base_runs, head_runs = %w[base head].map { |side| runs[side].sort.map { |_, reports| merge(reports) } }
    findings = compare(base_runs, head_runs, options[:thresholds])
    puts markdown(findings, base_runs:, head_runs:, thresholds: options[:thresholds],
                            base_label: "Base (saved)", head_label: "head")
    findings.any?(&:regression?) ? 1 : 0
  end

  # The scenarios the head build knows (`--list`), or the `--only` ones.
  def scenario_names(binary, only)
    listed, status = Open3.capture2(binary, "--perf-latency", "--list")
    raise "#{binary} --perf-latency --list failed" unless status.success?

    names = listed.lines.map { |line| line.split.first }.compact
    return names unless only

    wanted = only.split(",")
    unknown = wanted - names
    raise ArgumentError, "unknown scenario #{unknown.join(", ")}" if unknown.any?

    wanted
  end

  # One report holding the scenarios of several single-scenario runs.
  def merge(reports)
    reports.first.merge("scenarios" => reports.flat_map { |r| r["scenarios"] })
  end

  def run(argv)
    options = { base_ref: "origin/main", rounds: 5, samples: 40, warmup: 10,
                out: File.join(ROOT, "target", "latency"), thresholds: DEFAULT_THRESHOLDS.dup }
    OptionParser.new do |o|
      o.banner = "usage: ruby scripts/latency.rb [options]"
      o.on("--base-ref REF", "revision to compare against (default origin/main)") { |v| options[:base_ref] = v }
      o.on("--base-bin PATH", "prebuilt base binary (skips building base)") { |v| options[:base_bin] = v }
      o.on("--head-bin PATH", "prebuilt head binary (skips building head)") { |v| options[:head_bin] = v }
      o.on("--head-only", "measure head without comparing") { options[:head_only] = true }
      o.on("--from DIR", "compare the runs saved in DIR again, without measuring") { |v| options[:from] = v }
      o.on("--rounds N", Integer, "rounds; each runs every scenario on both builds (default 5)") { |v| options[:rounds] = v }
      o.on("--samples N", Integer, "keystrokes per scenario per round (default 40)") { |v| options[:samples] = v }
      o.on("--only LIST", "comma-separated scenarios") { |v| options[:only] = v }
      o.on("--out DIR", "results directory (default target/latency)") { |v| options[:out] = v }
      o.on("--max-increase PCT", Float, "fail above this % slowdown (default 15)") do |v|
        options[:thresholds].max_increase_pct = v
      end
      o.on("--max-sigma N", Float, "fail above this many σ of shift (default 3)") do |v|
        options[:thresholds].max_sigma = v
      end
    end.parse!(argv)

    return report_saved(File.expand_path(options[:from]), options) if options[:from]

    out = File.expand_path(options[:out])
    FileUtils.mkdir_p(out)
    Dir.glob(File.join(out, "*.json")).each { |f| File.delete(f) }
    head_bin = options[:head_bin] || File.join(out, "head-bin")
    build(ROOT, head_bin) unless options[:head_bin]

    base_bin, note = base_binary(options, out)

    # Pick the renderer once so both builds use the same one. Without a GPU,
    # tiny-skia draws on the CPU, ~10× slower at text: measure at 1× scale,
    # in fewer rounds of fewer keystrokes, so the check still takes minutes.
    probe = measure(head_bin, File.join(out, "probe.json"),
                    %w[--only type_character --samples 1 --warmup 0])
    args = ["--samples", options[:samples].to_s, "--warmup", options[:warmup].to_s,
            "--renderer", probe["renderer"]]
    if probe["renderer"] == "tiny-skia"
      warn "latency: no GPU available; measuring with tiny-skia at 1× scale"
      args = ["--samples", [options[:samples], 20].min.to_s, "--warmup", "3",
              "--renderer", "tiny-skia", "--scale", "1"]
      options[:rounds] = [options[:rounds], 3].min
    end
    names = scenario_names(head_bin, options[:only])

    # Each run measures one scenario, and base and head take turns per
    # scenario, a few seconds apart, alternating which goes first. Load from
    # other programs (a build, a backup, another app on the GPU) comes and
    # goes over tens of seconds, so this way it lands on both builds alike.
    base_runs = []
    head_runs = []
    turn = 0
    options[:rounds].times do |round|
      base_round = []
      head_round = []
      names.each do |name|
        sides = turn.even? ? %i[base head] : %i[head base]
        turn += 1
        sides.each do |side|
          binary, results = side == :head ? [head_bin, head_round] : [base_bin, base_round]
          next unless binary

          path = File.join(out, "#{side}-#{round + 1}-#{name}.json")
          results << measure(binary, path, args + ["--only", name])
        end
      end
      base_runs << merge(base_round) unless base_round.empty?
      head_runs << merge(head_round)
    end

    if base_runs.empty?
      report = head_only_markdown(head_runs, note)
      status = 0
    else
      findings = compare(base_runs, head_runs, options[:thresholds])
      report = markdown(findings, base_runs:, head_runs:, thresholds: options[:thresholds],
                                  base_label: "Base `#{options[:base_bin] ? "binary" : options[:base_ref]}`",
                                  head_label: "head")
      status = findings.any?(&:regression?) ? 1 : 0
    end
    File.write(File.join(out, "report.md"), report)
    File.write(ENV["GITHUB_STEP_SUMMARY"], report, mode: "a") if ENV["GITHUB_STEP_SUMMARY"]
    puts "", report
    status
  rescue RuntimeError, ArgumentError, JSON::ParserError, OptionParser::ParseError, SystemCallError => e
    warn "latency: #{e.message}"
    2
  end

  # The base binary: prebuilt, or built from a temporary worktree of the base
  # revision. Returns [path or nil, note explaining a missing base].
  def base_binary(options, out)
    return [nil, "Head only (`--head-only`)."] if options[:head_only]
    return [options[:base_bin], nil] if options[:base_bin]

    ref = options[:base_ref]
    sha, status = Open3.capture2("git", "-C", ROOT, "rev-parse", "--verify", "#{ref}^{commit}")
    raise "unknown base revision #{ref}" unless status.success?

    Dir.mktmpdir("latency-base") do |dir|
      checkout = File.join(dir, "base")
      sh!("git", "-C", ROOT, "worktree", "add", "--detach", checkout, sha.strip)
      begin
        unless harness_in?(File.read(File.join(checkout, "Cargo.toml")))
          return [nil, "No baseline: `#{ref}` predates the latency harness, so this run only records head's numbers."]
        end

        destination = File.join(out, "base-bin")
        build(checkout, destination)
        [destination, nil]
      ensure
        sh!("git", "-C", ROOT, "worktree", "remove", "--force", checkout)
      end
    end
  end
end

exit Latency.run(ARGV) if $PROGRAM_NAME == __FILE__
