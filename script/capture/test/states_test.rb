# frozen_string_literal: true

# Tests for the rule that picks a pull request's screenshot states. No app
# build, tmux or macOS needed:
#   ruby script/capture/test/states_test.rb

require "minitest/autorun"
require_relative "../states"

class StatesTest < Minitest::Test
  REPO = File.expand_path("../../..", __dir__)

  def selected(*paths)
    CaptureStates.select(paths).map { |state| state[:name] }
  end

  def all_states
    CaptureStates::STATES.map { |state| state[:name] }
  end

  def test_dependency_updates_get_no_screenshots
    assert_empty selected("Cargo.lock")
    assert_empty selected("Cargo.toml", "Cargo.lock", "vendor/iced_term/Cargo.toml")
  end

  def test_changes_the_app_does_not_draw_get_no_screenshots
    assert_empty selected("src/git.rs", "src/app/update.rs", "src/app/tests.rs", "README.md")
  end

  def test_a_settings_change_gets_only_the_settings_screenshot
    assert_equal ["04-settings"], selected("src/settings.rs", "docs/frd/FRD-09-settings.md")
  end

  def test_a_terminal_rendering_change_gets_the_terminal_screenshots
    assert_equal %w[01-workspace 02-review], selected("vendor/iced_term/src/view.rs")
  end

  def test_a_sidebar_change_gets_the_default_and_minimum_window_screenshots
    assert_equal %w[01-workspace 03-compact], selected("src/app/sidebar.rs", "src/app/update.rs")
  end

  def test_a_style_change_gets_every_screenshot
    assert_equal all_states, selected("src/style.rs")
  end

  def test_a_change_to_only_the_capture_tooling_gets_every_screenshot
    assert_equal all_states, selected("script/capture/capture.rb")
    assert_equal all_states, selected(".github/workflows/screenshots.yml", "src/app/mod.rs")
  end

  def test_a_feature_with_its_own_state_gets_only_that_state_even_when_it_adds_the_state
    assert_equal ["04-settings"], selected("script/capture/states.rb", "src/color_wheel.rs")
  end

  # A renamed or deleted file would otherwise drop out of every state silently.
  def test_every_covers_glob_matches_a_file_in_the_repository
    globs = CaptureStates::STATES.flat_map { |state| state[:covers] } + CaptureStates::SHARED +
            CaptureStates::TOOLING
    globs.uniq.each do |glob|
      refute_empty Dir.glob(glob, File::FNM_DOTMATCH, base: REPO), "#{glob} matches no file"
    end
  end
end
