# frozen_string_literal: true

# Unit tests for the dependency health rules. No network or cargo needed:
#   ruby scripts/test/dependency_health_test.rb

require "minitest/autorun"
require_relative "../dependency_health"

class DependencyHealthTest < Minitest::Test
  # Don't `include DependencyHealth`: its module functions (like `run`) would
  # shadow Minitest's own methods.
  Release = DependencyHealth::Release
  Version = DependencyHealth::Version
  Dependency = DependencyHealth::Dependency

  NOW = Time.utc(2026, 11, 22)

  def release(version, days_ago, yanked: false)
    Release.new(Version.parse(version), NOW - (days_ago * 86_400), yanked)
  end

  def dep(current, releases, name: "demo")
    Dependency.new(name:, current: Version.parse(current), vendored: false, releases:)
  end

  def messages(dependency)
    DependencyHealth.assess(dependency, now: NOW).map(&:message)
  end

  def test_pre_1_0_minor_releases_count_as_breaking_lines
    d = dep("0.14.0", [release("0.14.0", 400), release("0.15.0", 300), release("0.16.2", 10)])
    assert_equal 2, d.lines_behind
    assert_match(/2 breaking releases behind \(0\.14\.0 -> 0\.16\.2\)/, messages(d).join)
  end

  def test_compatible_releases_never_count_as_behind
    d = dep("1.0.200", [release("1.0.200", 400), release("1.0.229", 10), release("1.4.0", 5)])
    assert_equal 0, d.lines_behind
    assert_empty messages(d)
  end

  def test_one_breaking_release_behind_is_flagged_only_after_the_grace_period
    fresh = dep("0.10.9", [release("0.10.9", 400), release("0.11.0", 30)])
    stale = dep("0.10.9", [release("0.10.9", 400), release("0.11.0", 200), release("0.11.1", 30)])
    assert_empty messages(fresh)
    assert_match(/a breaking release behind for 200 days/, messages(stale).join)
  end

  def test_prereleases_and_yanked_releases_are_not_upgrade_targets
    d = dep("1.2.0", [release("1.2.0", 400), release("3.0.0-rc.1", 300), release("2.0.0", 300, yanked: true)])
    assert_equal 0, d.lines_behind
    assert_empty messages(d)
  end

  def test_a_yanked_locked_version_is_flagged
    d = dep("1.2.0", [release("1.2.0", 100, yanked: true), release("1.2.1", 90)])
    assert_match(/1\.2\.0 is yanked/, messages(d).join)
  end

  def test_years_without_a_release_is_flagged_but_a_quiet_stable_crate_is_not
    quiet = dep("0.6.2", [release("0.6.2", 900)])
    abandoned = dep("0.6.2", [release("0.6.2", 1200)])
    assert_empty messages(quiet)
    assert_match(/no release for 1200 days/, messages(abandoned).join)
  end

  def test_toolchain_is_flagged_once_it_falls_the_limit_behind_stable
    stable = Version.parse("1.100.0")
    assert_nil DependencyHealth.assess_toolchain(Version.parse("1.97.0"), stable)
    assert_match(/10 releases behind/, DependencyHealth.assess_toolchain(Version.parse("1.90.0"), stable).message)
  end
end
