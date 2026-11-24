# frozen_string_literal: true

# Tests for how a release picks its version bump. No git needed:
#   ruby .agents/skills/new-release/test/version_bump_test.rb

require "minitest/autorun"
require_relative "../version_bump"

class VersionBumpTest < Minitest::Test
  Commit = VersionBump::Commit

  def commit(subject, body = "")
    Commit.new(sha: "0" * 40, subject:, body:)
  end

  def merged(branch, title = "A pull request")
    commit("Merge pull request #1 from BrandonMathis/#{branch}", title)
  end

  def level(first_parent, all = first_parent)
    VersionBump.classify(first_parent, all).last
  end

  def test_fixes_ci_and_dependency_updates_are_a_patch
    assert_equal :patch, level([
      merged("fix/terminal-freeze-deadlock"),
      merged("ci/screenshots-as-artifacts"),
      merged("dependabot/cargo/toml-1.2.1spec-1.1.0"),
      commit("Keep project skills in .agents/skills")
    ])
  end

  def test_a_merged_feature_branch_is_a_minor_bump
    assert_equal :minor, level([merged("fix/corners"), merged("feat/reorder-projects")])
  end

  def test_a_conventional_feat_commit_is_a_minor_bump
    assert_equal :minor, level([commit("feat(sidebar): drag to reorder")])
  end

  def test_breaking_markers_are_a_major_bump
    assert_equal :major, level([commit("feat!: rename tmux sessions")])
    assert_equal :major, level([merged("feat/sessions", "feat!: rename tmux sessions")])
    assert_equal :major, level([merged("fix/x", "Fix x\n\nBREAKING CHANGE: drops the old socket")])
  end

  def test_a_breaking_commit_inside_a_merged_branch_is_a_major_bump
    main_line = [merged("fix/sessions")]
    inside = commit("Rename sessions", "BREAKING CHANGE: running sessions are orphaned")
    assert_equal :major, level(main_line, main_line + [inside])
  end

  def test_release_commits_and_release_branches_are_not_changes
    assert_nil level([commit("Release 0.3.0"), merged("release/0.3.0")])
  end

  def test_bumps_follow_semver
    assert_equal "1.0.0", VersionBump.bump("0.3.4", :major)
    assert_equal "0.4.0", VersionBump.bump("0.3.4", :minor)
    assert_equal "0.3.5", VersionBump.bump("0.3.4", :patch)
  end
end
