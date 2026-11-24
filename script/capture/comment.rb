#!/usr/bin/env ruby
# frozen_string_literal: true

# Writes the Markdown for the PR screenshot comment to stdout.
#
#   ruby script/capture/comment.rb DIR ARTIFACTS_TSV HEAD_SHA RUN_URL
#
# DIR holds states.json, written by capture.rb. ARTIFACTS_TSV lists the workflow
# run's artifacts, one "name<TAB>id" per line; each screenshot is the artifact
# named after its file (`01-workspace.png`). The comment links to those
# artifacts, so no image is stored in the repository.

require "json"

# The workflow finds and updates its earlier comment by this marker.
MARKER = "<!-- worktree-browser-screenshots -->"

dir, artifacts_tsv, sha, run_url = ARGV
abort "Usage: comment.rb DIR ARTIFACTS_TSV HEAD_SHA RUN_URL" unless run_url

states = JSON.parse(File.read(File.join(dir, "states.json")))
artifact_ids = File.readlines(artifacts_tsv, chomp: true).to_h { |line| line.split("\t", 2) }

puts MARKER
puts "### Screenshots"
puts
puts "Worktree Browser at #{sha[0, 7]}, in the states that show this pull request's changes, " \
     "with fixture repositories and tmux sessions " \
     "([run](#{run_url}), `script/capture/capture.rb`)."
puts
states.each do |state|
  id = artifact_ids["#{state['name']}.png"]
  # A state whose upload failed is still listed, without a link.
  title = id ? "[#{state['title']}](#{run_url}/artifacts/#{id})" : state["title"]
  puts "- **#{title}** · `#{state['size']}`"
end
puts
puts "The links open workflow artifacts: they need a GitHub sign-in and stop working when the artifacts expire."
