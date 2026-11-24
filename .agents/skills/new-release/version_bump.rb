# frozen_string_literal: true

# Decides whether a release is a major, minor or patch bump from the commits
# since the previous release. Standard library only; tested by
# .agents/skills/new-release/test/version_bump_test.rb.
#
# The type of a change comes from, in order:
#   - a GitHub merge commit's branch prefix ("Merge pull request #12 from
#     owner/feat/reorder-projects" is a `feat`), which is how most work lands;
#   - a Conventional Commits subject ("fix(tmux): ...", "ci: ...").
# `feat` is a minor bump; every other type, or no type, is a patch.
# A `type!:` subject or PR title, or a `BREAKING CHANGE:` footer in any commit
# in the range (merged branches included), is a major bump.
module VersionBump
  LEVELS = %i[patch minor major].freeze
  MINOR_TYPES = %w[feat feature].freeze
  CONVENTIONAL = /\A(\w+)(?:\([^)]*\))?(!)?:/
  MERGED_PR = %r{\AMerge pull request #\d+ from [^/\s]+/(\S+)}
  RELEASE_SUBJECT = /\ARelease \d+\.\d+\.\d+\z/

  Commit = Data.define(:sha, :subject, :body)
  Change = Data.define(:commit, :type, :level)

  module_function

  # The change type of one commit, or nil when it has none.
  def type_of(commit)
    if (branch = commit.subject[MERGED_PR, 1])
      prefix = branch.split("/", 2).first if branch.include?("/")
      return prefix&.delete_suffix("!")&.downcase
    end
    commit.subject[CONVENTIONAL, 1]&.downcase
  end

  def breaking?(commit)
    pr_title = commit.body.lines.first.to_s
    [commit.subject, pr_title].any? { |line| line.match(CONVENTIONAL)&.[](2) } ||
      commit.subject[MERGED_PR, 1].to_s.match?(%r{\A\w+!/}) ||
      commit.body.match?(/^BREAKING[ -]CHANGE:/)
  end

  # Release commits and merges of release branches are bookkeeping, not changes.
  def release?(commit)
    commit.subject.match?(RELEASE_SUBJECT) || type_of(commit) == "release"
  end

  # `first_parent` is the main-line history (one entry per merged PR or direct
  # commit); `all` also includes the merged branches' own commits, which are
  # only searched for breaking-change markers.
  def classify(first_parent, all)
    changes = first_parent.reject { |c| release?(c) }.map do |commit|
      type = type_of(commit)
      level = if breaking?(commit) then :major
              elsif MINOR_TYPES.include?(type) then :minor
              else :patch
              end
      Change.new(commit:, type:, level:)
    end
    level = changes.map(&:level).max_by { |l| LEVELS.index(l) }
    level = :major if level && all.any? { |c| !release?(c) && breaking?(c) }
    [changes, level]
  end

  def bump(version, level)
    major, minor, patch = version.split(".").map(&:to_i)
    case level
    when :major then "#{major + 1}.0.0"
    when :minor then "#{major}.#{minor + 1}.0"
    when :patch then "#{major}.#{minor}.#{patch + 1}"
    else raise ArgumentError, "unknown bump level #{level.inspect}"
    end
  end
end
