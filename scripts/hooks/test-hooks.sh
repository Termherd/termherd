#!/usr/bin/env bash
# Exercise the Claude Code hooks beside this script with real payloads; print
# PASS/FAIL per case and exit non-zero on any failure. Needs jq and
# markdownlint-cli2.
set -u
hooks=$(dirname "$0")
tmp=$(mktemp -d "${TMPDIR:-/tmp}/t.XXXXXX")
fail=0

expect() { # name, want-exit, hook, payload
  local got
  printf '%s' "$4" | "$hooks/$3" >/dev/null 2>"$tmp/err"
  got=$?
  if [[ $got -eq $2 ]]; then echo "PASS $1 ($got)"; else echo "FAIL $1: want $2 got $got"; cat "$tmp/err"; fail=1; fi
}

trailer='Co-Authored-By: Claude Opus <noreply@anthropic.com>'
printf 'fix: x\n\n%s\n' "$trailer" > "$tmp/msg.txt"
printf 'Body\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n' > "$tmp/body.md"
printf 'fix: clean\n' > "$tmp/clean.txt"

j() { jq -cn --arg c "$1" '{tool_input:{command:$c}}'; }
expect "commit -m trailer blocks" 2 no-ai-signature.sh "$(j "git commit -m \"fix: x

$trailer\"")"
expect "lowercase trailer blocks" 2 no-ai-signature.sh "$(j "git commit -m 'x co-authored-by: claude'")"
expect "commit -F file blocks" 2 no-ai-signature.sh "$(j "git commit -F $tmp/msg.txt")"
expect "pr --body-file footer blocks" 2 no-ai-signature.sh "$(j "gh pr create --title t --body-file $tmp/body.md")"
expect "clean commit passes" 0 no-ai-signature.sh "$(j "git commit -F $tmp/clean.txt")"
expect "human co-author passes" 0 no-ai-signature.sh "$(j "git commit -m 'x Co-Authored-By: Ada <a@b.c>'")"
expect "unrelated command passes" 0 no-ai-signature.sh "$(j "grep -i 'generated with claude' notes.md")"

root=$(mktemp -d "${TMPDIR:-/tmp}/t.XXXXXX")
cp .markdownlint-cli2.jsonc "$root/"
printf '# Ok\n\nShort line.\n' > "$root/ok.md"
printf '# Bad\nno blank line after heading\n' > "$root/bad.md"
mkdir -p "$root/.personal"
printf '# Bad\nno blank line after heading\n' > "$root/.personal/bad.md"
printf 'x\n' > "$root/code.rs"
w() { jq -cn --arg f "$1" '{tool_input:{file_path:$f}}'; }
export CLAUDE_PROJECT_DIR=$root
expect "clean md passes" 0 lint-markdown.sh "$(w "$root/ok.md")"
expect "bad md reports" 2 lint-markdown.sh "$(w "$root/bad.md")"
expect "ignored .personal md still linted" 2 lint-markdown.sh "$(w "$root/.personal/bad.md")"
expect "non-md skipped" 0 lint-markdown.sh "$(w "$root/code.rs")"
expect "md outside project skipped" 0 lint-markdown.sh "$(w "$tmp/body.md")"
rm -rf "$tmp" "$root"
exit $fail
