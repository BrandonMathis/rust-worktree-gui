#!/usr/bin/env ruby
# frozen_string_literal: true

# Prints the comma-separated names of the screenshot states a pull request
# should capture (see states.rb), or nothing if none show its changes.
#
#   git diff --name-only -z BASE HEAD | ruby script/capture/select.rb [--summary FILE]
#   ruby script/capture/select.rb --all
#
# Changed paths are read from stdin, NUL- or newline-separated. --summary
# appends a Markdown note explaining the choice (for $GITHUB_STEP_SUMMARY).

require "optparse"
require_relative "states"

options = { all: false, summary: nil }
OptionParser.new do |opts|
  opts.banner = "Usage: select.rb [--all] [--summary FILE] < changed-paths"
  opts.on("--all", "Every state, ignoring stdin") { options[:all] = true }
  opts.on("--summary FILE", "Append a Markdown explanation to FILE") { |v| options[:summary] = v }
end.parse!

paths = options[:all] ? [] : $stdin.read.split(/[\0\n]/).reject(&:empty?)
states = options[:all] ? CaptureStates::STATES : CaptureStates.select(paths)

note =
  if options[:all]
    "Capturing every screenshot state."
  elsif states.empty?
    "No screenshot state shows the files this pull request changes, so none were captured. " \
      "If it changes how the app looks, add a state that shows it to `script/capture/states.rb`."
  else
    "Capturing the screenshot states that show this pull request's changes: " \
      "#{states.map { |s| "`#{s[:name]}`" }.join(', ')}."
  end
warn note
File.write(options[:summary], "#{note}\n", mode: "a") if options[:summary]

puts states.map { |state| state[:name] }.join(",")
