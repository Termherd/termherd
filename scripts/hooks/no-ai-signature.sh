#!/usr/bin/env bash
# Claude Code PreToolUse hook (Bash): refuse a commit, pull request or issue
# whose text carries an AI attribution. The rule failed three times as prose,
# because a harness reminder asks for the trailer on every commit.
#
# Reads the hook payload on stdin. Exit 2 blocks the call and shows stderr to
# the agent; anything else lets it run.
set -uo pipefail

payload=$(cat)

if command -v jq >/dev/null 2>&1; then
  command=$(jq -r '.tool_input.command // empty' <<<"$payload")
else
  command=$payload
fi

case "$command" in
  *"git commit"* | *"gh pr create"* | *"gh pr edit"* | *"gh pr comment"* | \
    *"gh pr merge"* | *"gh issue create"* | *"gh issue edit"* | \
    *"gh issue comment"*) ;;
  *) exit 0 ;;
esac

text=$command
# Bodies passed by file: `git commit -F f`, `gh … --body-file f`.
while IFS= read -r file; do
  file=${file#[\"\']}
  file=${file%[\"\']}
  if [[ -f $file ]]; then
    text+=$'\n'$(cat "$file")
  fi
done < <(grep -oE -- '(--body-file|--file|-F)[= ]+[^ ]+' <<<"$command" |
  sed -E 's/^(--body-file|--file|-F)[= ]+//')

pattern='co-authored-by:.*(claude|anthropic)|generated (with|by) \[?claude'
if grep -qiE "$pattern" <<<"$text"; then
  echo "Blocked: the message carries an AI attribution (a Co-Authored-By:" \
    "Claude trailer or a 'Generated with Claude Code' footer). termherd" \
    "forbids AI signatures; remove it and retry." >&2
  exit 2
fi
exit 0
