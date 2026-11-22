#!/usr/bin/env ruby
# frozen_string_literal: true

# Writes the Markdown for the PR screenshot comment to stdout.
#
#   ruby script/capture/comment.rb DIR IMAGE_BASE_URL HEAD_SHA RUN_URL
#
# DIR holds the PNGs and states.json written by capture.rb. IMAGE_BASE_URL is
# where those PNGs are published (no trailing slash).

require "json"

# The workflow finds and updates its earlier comment by this marker.
MARKER = "<!-- worktree-browser-screenshots -->"

dir, base_url, sha, run_url = ARGV
abort "Usage: comment.rb DIR IMAGE_BASE_URL HEAD_SHA RUN_URL" unless run_url

states = JSON.parse(File.read(File.join(dir, "states.json")))

puts MARKER
puts "### Screenshots"
puts
puts "Worktree Browser at #{sha[0, 7]}, with fixture repositories and tmux sessions " \
     "([run](#{run_url}), `script/capture/capture.rb`)."
states.each do |state|
  puts
  puts "**#{state['title']}** · `#{state['size']}`"
  puts
  puts "<img src=\"#{base_url}/#{state['name']}.png\" alt=\"#{state['title']}\" width=\"#{state['size'].split('x').first}\">"
end
