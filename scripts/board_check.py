#!/usr/bin/env python3
"""Report open issues the project board has not classified.

The board (AGENTS.md § How we track work) is canonical for priority and order,
held as `Horizon` / `Class` / `Effort` / `Severity`. An issue that never gets
those fields is invisible to every board view — filed, then lost. This reports
the two ways that happens:

  * an open issue absent from the board altogether
  * an open issue carrying none of the fields its native type owes

What each type owes:

  | type      | Horizon | Class | Severity | Effort |
  |-----------|---------|-------|----------|--------|
  | `Feature` | yes     | yes   | --       | yes    |
  | `Bug`     | yes     | --    | yes      | yes    |
  | `Task`    | yes     | --    | --       | yes    |

A `Bug` carries no Class because it restores a contract rather than adding
leverage; a `Task` (packaging, tooling) carries neither. An issue labelled
`needs-design` owes only a Horizon — the feature-torture pass is what produces
the rest, so demanding them before it has run is backwards.

**Why there is no roadmap dimension here.** faceto's `sync_roadmap.py` also
reconciles ROADMAP.md, because its roadmap is a table with a per-row `Horizon`
that says which rows are committed and therefore owe a tracking issue.
termherd's is a MoSCoW checkbox list with no such column, and both directions
of the obvious check were tried and dropped for firing on correct states:
"every open issue is cited by an entry" flags refinements that were never
features (auto-scroll during a drag-selection); "every unticked entry cites an
issue" flags the design-first items AGENTS.md explicitly says live in the
roadmap alone (`F-activity-stats`, `F-scheduled-tasks`). Reconciling the
roadmap stays a human read, until entries carry a horizon of their own.

Usage:
  board_check.py             # report; always exit 0
  board_check.py --check     # exit 1 if anything is unclassified
  board_check.py --selftest  # offline guards, no network

Needs `gh` on PATH and authenticated, with the `project` scope for the board.
An unreachable dimension is skipped with its cause, never fatal.
"""

from __future__ import annotations

import json
import subprocess
import sys

OWNER = "Termherd"
REPO = "Termherd/termherd"
PROJECT = "1"


def gh_json(args: list[str]) -> dict | list:
    out = subprocess.run(["gh", *args], capture_output=True, text=True, check=True)
    return json.loads(out.stdout)


def reason(exc: Exception) -> str:
    """A short, honest cause — so a skipped dimension names what actually
    failed instead of always blaming a missing scope."""
    if isinstance(exc, FileNotFoundError):
        return "`gh` not found on PATH"
    if isinstance(exc, subprocess.CalledProcessError):
        tail = (exc.stderr or "").strip().splitlines()
        line = tail[-1] if tail else ""
        low = line.lower()
        if "scope" in low:
            return "token lacks the `project` scope"
        if any(k in low for k in ("tls", "certificate", "dial tcp", "connection")):
            return f"network blocked `gh` ({line})"
        return line or "`gh` exited non-zero"
    if isinstance(exc, json.JSONDecodeError):
        return "`gh` returned non-JSON output"
    return str(exc)


def open_issues() -> tuple[dict[int, tuple[str, str]] | None, str]:
    """{number: (native issue type, title)}, or (None, reason).

    The type has to come from the issue: `gh project item-list` reports the
    content *kind* ("Issue"), never `Feature` / `Bug` / `Task`.
    """
    try:
        data = gh_json(["issue", "list", "--repo", REPO, "--state", "open",
                        "--limit", "500", "--json", "number,title,issueType"])
    except (subprocess.CalledProcessError, json.JSONDecodeError, OSError) as exc:
        return None, reason(exc)
    return {
        int(i["number"]): ((i.get("issueType") or {}).get("name") or "", i["title"])
        for i in data
    }, ""


def board() -> tuple[dict[int, dict] | None, str]:
    """{issue number: item}, or (None, reason).

    `gh project item-list` lower-cases custom field names in its JSON output.
    """
    try:
        data = gh_json(["project", "item-list", PROJECT, "--owner", OWNER,
                        "--format", "json", "--limit", "300"])
    except (subprocess.CalledProcessError, json.JSONDecodeError, OSError) as exc:
        return None, reason(exc)
    out: dict[int, dict] = {}
    for item in data.get("items", []):
        num = (item.get("content") or {}).get("number")
        if num is not None:
            out[int(num)] = item
    return out, ""


def missing_fields(item: dict, issue_type: str) -> list[str]:
    """Which board fields this item owes for its type, and doesn't carry."""
    gaps = []
    if not item.get("horizon"):
        gaps.append("Horizon")
    if "needs-design" in (item.get("labels") or []):
        return gaps
    if issue_type == "Feature" and not item.get("class"):
        gaps.append("Class")
    if issue_type == "Bug" and not item.get("severity"):
        gaps.append("Severity")
    if not item.get("effort"):
        gaps.append("Effort")
    return gaps


def selftest() -> int:
    fails = []

    field_cases = [
        # (item, issue type, expected gaps)
        ({}, "Feature", ["Horizon", "Class", "Effort"]),
        ({"horizon": "Now", "class": "✨ Polish", "effort": "S"}, "Feature", []),
        ({"horizon": "Now", "severity": "🟠 Major", "effort": "M"}, "Bug", []),
        # a Task owes neither Class nor Severity — packaging restores or
        # maintains, it does not add leverage
        ({"horizon": "Parked", "effort": "M"}, "Task", []),
        ({"horizon": "Now", "effort": "M"}, "Bug", ["Severity"]),
        ({"horizon": "Now", "class": "🔑 Enabler"}, "Feature", ["Effort"]),
        # needs-design owes only a Horizon: torture is what yields the rest
        ({"horizon": "Later", "labels": ["needs-design"]}, "Feature", []),
        ({"labels": ["needs-design"]}, "Feature", ["Horizon"]),
        # an untyped issue still owes a Horizon and an Effort
        ({}, "", ["Horizon", "Effort"]),
    ]
    for item, itype, want in field_cases:
        got = missing_fields(item, itype)
        if got != want:
            fails.append(f"missing_fields({item!r}, {itype!r}) = {got!r}, want {want!r}")

    def cpe(stderr: str) -> subprocess.CalledProcessError:
        exc = subprocess.CalledProcessError(1, ["gh"])
        exc.stderr = stderr
        return exc

    for exc, want_sub in [
        (FileNotFoundError(), "not found"),
        (cpe("error: missing required scopes: project"), "scope"),
        (cpe('Post "https://api.github.com/graphql": tls: bad certificate'), "network"),
    ]:
        if want_sub not in reason(exc):
            fails.append(f"reason({exc!r}) missing {want_sub!r}")

    for f in fails:
        print("selftest FAIL:", f)
    print("selftest OK" if not fails else f"selftest: {len(fails)} failure(s)")
    return 1 if fails else 0


def main() -> int:
    argv = sys.argv[1:]
    if "--selftest" in argv:
        return selftest()

    issues, issues_err = open_issues()
    items, board_err = board()

    if issues is None:
        print(f"· issues unreachable ({issues_err}) — nothing to check.")
        return 0
    if items is None:
        print(f"· board unreachable ({board_err}) — nothing to check.")
        return 0

    absent = sorted(set(issues) - set(items))
    gaps = {n: missing_fields(items[n], issues[n][0]) for n in sorted(issues) if n in items}
    gaps = {n: g for n, g in gaps.items() if g}

    for n in absent:
        print(f"⚠ #{n} is not on the board — {issues[n][1]}")
    for n, g in gaps.items():
        print(f"⚠ #{n} ({issues[n][0] or 'no type'}) missing {', '.join(g)} — {issues[n][1]}")
    if not absent and not gaps:
        print(f"✓ all {len(issues)} open issues are on the board and classified.")

    return 1 if ((absent or gaps) and "--check" in argv) else 0


if __name__ == "__main__":
    raise SystemExit(main())
