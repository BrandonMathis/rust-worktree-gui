# frozen_string_literal: true

# Unit tests for the keystroke latency comparison rules. No cargo or app
# build needed:
#   ruby scripts/test/latency_test.rb

require "minitest/autorun"
require_relative "../latency"

class LatencyTest < Minitest::Test
  # Normally distributed samples, reproducible from `seed`.
  def samples(mean, sd, seed, count = 60)
    random = Random.new(seed)
    Array.new(count) do
      # Box–Muller transform: two uniform numbers make one normal one.
      z = Math.sqrt(-2 * Math.log(1 - random.rand)) * Math.cos(2 * Math::PI * random.rand)
      mean + (sd * z)
    end
  end

  # A report with one scenario whose `total_ms` and `app_ms` follow
  # (mean, sd); the phase series are copies so every metric is present.
  def report(total:, app: [0.8, 0.02], seed:, renderer: "wgpu")
    total_ms = samples(*total, seed)
    app_ms = samples(*app, seed + 1000)
    series = Latency::METRICS.keys.to_h { |metric| [metric, app_ms] }
    {
      "schema" => Latency::SCHEMA,
      "renderer" => renderer,
      "window" => { "width" => 1120, "height" => 720, "scale" => 2 },
      "samples_per_scenario" => 60,
      "scenarios" => [series.merge("name" => "type_character", "total_ms" => total_ms, "app_ms" => app_ms)]
    }
  end

  # Three rounds of each build, with distinct seeds per round.
  def rounds(seed, **options)
    Array.new(3) { |round| report(seed: seed + round, **options) }
  end

  def finding(base, head, metric = "total_ms")
    Latency.compare(base, head).find { |f| f.metric == metric }
  end

  def test_run_to_run_noise_passes
    base = rounds(10, total: [20, 4])
    head = rounds(20, total: [20, 4])
    assert(Latency.compare(base, head).none?(&:regression?))
  end

  def test_a_slowdown_beyond_the_percentage_budget_fails
    # Noisy samples (σ = 3 ms), so the 5 ms slowdown is under 3σ, but it is 25%.
    f = finding(rounds(10, total: [20, 3]), rounds(20, total: [25, 3]))
    assert_predicate f, :regression?
    assert_match(/% > 15%/, f.reasons.join)
    refute_match(/σ >/, f.reasons.join)
  end

  def test_a_shift_beyond_the_sigma_budget_fails_even_under_the_percentage_budget
    # Tight samples (σ = 0.2 ms): 1.5 ms slower is only 7.5%, but 7.5σ.
    f = finding(rounds(10, total: [20, 0.2]), rounds(20, total: [21.5, 0.2]))
    assert_predicate f, :regression?
    assert_match(/σ > 3σ/, f.reasons.join)
    refute_match(/% > 15%/, f.reasons.join)
  end

  def test_the_app_work_metric_is_gated_too
    f = finding(rounds(10, total: [20, 4], app: [0.8, 0.02]),
                rounds(20, total: [20, 4], app: [1.4, 0.02]), "app_ms")
    assert_predicate f, :regression?
  end

  def test_a_slowdown_below_the_absolute_floor_never_fails
    # 40% and many σ slower, but only 0.2 ms: under the 1 ms floor for total.
    f = finding(rounds(10, total: [0.5, 0.01]), rounds(20, total: [0.7, 0.01]))
    refute_predicate f, :regression?
  end

  def test_one_stalled_round_does_not_move_the_result
    base = rounds(10, total: [20, 0.5])
    head = rounds(20, total: [20, 0.5])
    head[1] = report(seed: 99, total: [40, 0.5])
    assert_equal :ok, finding(base, head).status
  end

  def test_a_slowdown_in_only_some_rounds_is_a_warning_not_a_failure
    base = rounds(10, total: [20, 0.5])
    head = rounds(20, total: [25, 0.5])
    # The last round matches base: the slowdown came and went with the runner.
    head[2] = report(seed: 99, total: [19.9, 0.5])
    f = finding(base, head)
    assert_equal :warning, f.status
    assert_includes f.reasons, "not slower in every round"
  end

  def test_phase_metrics_are_reported_but_never_fail
    base = rounds(10, total: [20, 4])
    head = rounds(20, total: [20, 4])
    head.each { |r| r["scenarios"][0]["raster_ms"] = r["scenarios"][0]["raster_ms"].map { |v| v * 3 } }
    f = finding(base, head, "raster_ms")
    assert_operator f.change_pct, :>, 100
    assert_equal :ok, f.status
  end

  def test_a_faster_head_passes
    f = finding(rounds(10, total: [20, 1]), rounds(20, total: [12, 1]))
    assert_equal :ok, f.status
    assert_operator f.change_pct, :<, -30
  end

  def test_runs_with_different_renderers_are_not_compared
    error = assert_raises(ArgumentError) do
      Latency.compare(rounds(10, total: [20, 4]), rounds(20, total: [20, 4], renderer: "tiny-skia"))
    end
    assert_match(/different renderers/, error.message)
  end

  def test_mann_whitney_finds_a_shift_and_not_a_tie
    assert_operator Latency.mann_whitney_p(samples(11, 1, 1), samples(10, 1, 2)), :<, 0.001
    assert_operator Latency.mann_whitney_p(samples(10, 1, 1), samples(11, 1, 2)), :>, 0.99
    same = [5.0] * 40
    assert_in_delta 0.5, Latency.mann_whitney_p(same, same), 0.01
  end

  def test_robust_sigma_ignores_outliers_and_matches_a_normal_spread
    values = samples(20, 2, 7, 2000)
    assert_in_delta 2.0, Latency.robust_sigma(values), 0.15
    assert_in_delta 2.0, Latency.robust_sigma(values + [500.0] * 20), 0.2
  end

  def test_only_checkouts_that_declare_the_perf_feature_have_a_baseline
    assert Latency.harness_in?(%(capture = []\nperf = ["dep:iced_runtime"]\n))
    refute Latency.harness_in?(%(capture = []\n))
  end

  def test_the_report_flags_regressions_and_explains_the_rules
    base = rounds(10, total: [20, 3])
    head = rounds(20, total: [26, 3])
    text = Latency.markdown(Latency.compare(base, head), base_runs: base, head_runs: head,
                                                         base_label: "Base `main`", head_label: "head")
    assert_match(/\*\*1 regression\.\*\*/, text)
    assert_match(/\| type_character \| key → pixels \|.*❌/, text)
    assert_match(/Mann–Whitney p < 0.01/, text)
  end
end
