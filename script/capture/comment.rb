#!/usr/bin/env ruby
# frozen_string_literal: true

# Writes the Markdown for the PR screenshot comment to stdout.
#
#   ruby script/capture/comment.rb DIR IMAGE_URL_PREFIX HEAD_SHA RUN_URL
#
# DIR holds states.json and the PNGs, written by capture.rb. Each PNG is shown
# inline from IMAGE_URL_PREFIX followed by its file name (`01-workspace.png`):
# the workflow uploads the PNGs as release assets under that prefix, because
# GitHub shows an image in a comment only from a URL anyone can fetch, which a
# workflow artifact is not.

require "json"

# The workflow finds and updates its earlier comment by this marker.
MARKER = "<!-- worktree-browser-screenshots -->"

dir, url_prefix, sha, run_url = ARGV
abort "Usage: comment.rb DIR IMAGE_URL_PREFIX HEAD_SHA RUN_URL" unless run_url

states = JSON.parse(File.read(File.join(dir, "states.json")))

puts MARKER
puts "### Screenshots"
puts
puts "Worktree Browser at #{sha[0, 7]}, in the states that show this pull request's changes, " \
     "with fixture repositories and tmux sessions " \
     "([run](#{run_url}), `script/capture/capture.rb`)."
states.each do |state|
  file = "#{state['name']}.png"
  puts
  puts "#### #{state['title']} · `#{state['size']}`"
  puts
  # A state whose capture failed is still listed, without an image.
  puts File.exist?(File.join(dir, file)) ? "![#{state['title']}](#{url_prefix}#{file})" : "_Not captured; see the run._"
end
