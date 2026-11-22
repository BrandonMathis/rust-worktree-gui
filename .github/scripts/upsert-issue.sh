#!/usr/bin/env bash
# Keeps one open GitHub issue per scheduled check, identified by a label.
#
#   upsert-issue.sh open    <label> <title> <body-file>   # open, or update if the body changed
#   upsert-issue.sh resolve <label>                       # close the open issue, if any
#
# Needs `gh` with GH_TOKEN and GH_REPO set (the workflow provides both) and
# RUN_URL pointing at the workflow run, which is linked in each comment.
# Updating an issue's body sends no notification, so a comment is added
# whenever the report changes: that comment is what reaches watchers.
set -euo pipefail

mode=$1
label=$2

gh label create "$label" --color B60205 --description "Opened by a scheduled check" >/dev/null 2>&1 || true
existing=$(gh issue list --label "$label" --state open --limit 1 --json number --jq '.[0].number // empty')

case "$mode" in
  open)
    title=$3
    body_file=$4
    if [[ -z "$existing" ]]; then
      gh issue create --title "$title" --label "$label" --body-file "$body_file"
    elif [[ "$(gh issue view "$existing" --json body --jq .body)" != "$(cat "$body_file")" ]]; then
      gh issue edit "$existing" --body-file "$body_file"
      gh issue comment "$existing" --body "The report changed. Run: ${RUN_URL}"
    else
      echo "Issue #$existing is already up to date."
    fi
    ;;
  resolve)
    if [[ -n "$existing" ]]; then
      gh issue close "$existing" --comment "Resolved: the check passed. Run: ${RUN_URL}"
    fi
    ;;
  *)
    echo "usage: $0 open|resolve <label> [title body-file]" >&2
    exit 64
    ;;
esac
