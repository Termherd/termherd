#!/usr/bin/env bash
# Claude Code PostToolUse hook (Edit|Write): lint the Markdown file just
# written, with the repo's config, and hand the findings back to the agent.
#
# A file the config ignores (`.personal/`, `.local/`) reports `Linting: 0
# files`, which reads like a pass; such a file is linted as a copy instead.
# Exit 2 shows stderr to the agent; a missing linter is not an error.
set -uo pipefail

payload=$(cat)

if command -v jq >/dev/null 2>&1; then
  file=$(jq -r '.tool_input.file_path // empty' <<<"$payload")
else
  file=$(grep -oE '"file_path" *: *"[^"]*"' <<<"$payload" |
    sed -E 's/.*"([^"]*)"$/\1/')
fi

root=${CLAUDE_PROJECT_DIR:-$(pwd)}
case "$file" in
  "$root"/*.md) ;;
  *) exit 0 ;;
esac
command -v markdownlint-cli2 >/dev/null 2>&1 || exit 0

cd "$root" || exit 0
report=$(markdownlint-cli2 --no-globs "${file#"$root"/}" 2>&1)
status=$?

if grep -q '^Linting: 0 files' <<<"$report"; then
  copy=$(mktemp -d "${TMPDIR:-/tmp}/lint-md.XXXXXX")
  cp .markdownlint-cli2.jsonc "$copy/"
  cp "$file" "$copy/"
  report=$(cd "$copy" && markdownlint-cli2 --no-globs "$(basename "$file")" 2>&1)
  status=$?
  rm -rf "$copy"
fi

if [[ $status -ne 0 ]]; then
  grep -vE '^(markdownlint-cli2 v|Finding:|Linting:|Summary:)' <<<"$report" >&2
  exit 2
fi
exit 0
