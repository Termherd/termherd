#!/usr/bin/env python3
"""Report where the project board and the issues have drifted apart.

The board (AGENTS.md § How we track work) is canonical for priority and order,
held as `Horizon` / `Class` / `Effort` / `Severity`. An issue the board never
classified is invisible to every view — filed, then lost. This reports the
three ways that happens:

  * an open issue absent from the board altogether
  * an open issue missing a field its native type owes
  * a board item whose `Status` and `Horizon` disagree about having shipped

The first is a backstop rather than the common case: since 2026-08-02 the
project's *Auto-add to project* workflow puts every new issue on the board by
itself, so absence now means either that the issue predates that date or that
the workflow has since been turned off — both worth knowing. What escapes
routinely is the second finding: an item present but unclassified.

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

The third check exists because closing an issue only moves *half* the board:
the workflow flips `Status` to Done, and `Horizon` keeps whatever it had. A
shipped feature then still reads as Next in the By-horizon view. Unlike the
other two it runs over **every** board item, not just the open issues — the
drift it catches appears at the moment an issue closes, i.e. exactly when the
issue stops being open.

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
  board_check.py --check     # 0 clean · 1 drift found · 2 could not check
  board_check.py --selftest  # offline guards, no network

Needs `gh` on PATH and authenticated, with the `project` scope for the board.
A dimension that cannot be fetched is reported as *unchecked*, never as clean:
under `--check` that is exit 2, distinct from the 1 a real finding earns.
"""

from __future__ import annotations

import json
import subprocess
import sys

OWNER = "Termherd"
REPO = "Termherd/termherd"
PROJECT = "1"

# Page sizes. Both are checked for truncation rather than trusted: a partial
# fetch reads exactly like a board that lost items.
ISSUE_LIMIT = 500
BOARD_LIMIT = 1000

# Per-call wall clock. `gh` retries and can hang on a wedged proxy; without a
# bound the script would hang a planning pass instead of reporting a cause.
GH_TIMEOUT = 60

FLAGS = {"--check", "--selftest"}

# A malformed payload is a fetch failure, not a crash: `gh` shapes are stable
# but not contractual, and a traceback tells a reader less than a named cause.
FETCH_ERRORS = (
    subprocess.CalledProcessError,
    subprocess.TimeoutExpired,
    json.JSONDecodeError,
    OSError,
    AttributeError,
    TypeError,
    KeyError,
    ValueError,
)


class Truncated(Exception):
    """A page cap cut the fetch short.

    Distinct from an unreachable `gh`: that is an environment the script can
    only skip, this is a constant in this file that stopped being big enough.
    Skipping it would report a healthy board as broken, so it fails loudly
    instead of degrading quietly.
    """


def gh_json(args: list[str]) -> dict | list:
    out = subprocess.run(["gh", *args], capture_output=True, text=True,
                         check=True, timeout=GH_TIMEOUT)
    return json.loads(out.stdout)


def reason(exc: Exception) -> str:
    """A short, honest cause — so an unchecked dimension names what actually
    failed instead of always blaming a missing scope.

    Every stderr line is scanned, not just the last: `gh` puts the diagnosis on
    one line and the remedy on the next ("error: your authentication token is
    missing required scopes [read:project]" / "To request it, run: gh auth
    refresh -s read:project"), so reading the tail alone reports the remedy as
    if it were the cause.
    """
    if isinstance(exc, FileNotFoundError):
        return "`gh` not found on PATH"
    if isinstance(exc, subprocess.TimeoutExpired):
        return f"`gh` exceeded {GH_TIMEOUT}s"
    if isinstance(exc, subprocess.CalledProcessError):
        lines = (exc.stderr or "").strip().splitlines()
        blob = "\n".join(lines).lower()
        if "scope" in blob:
            return "token lacks the `project` scope"
        if any(k in blob for k in ("tls", "certificate", "dial tcp", "connection")):
            return f"network blocked `gh` ({lines[0]})"
        return lines[-1] if lines else "`gh` exited non-zero"
    if isinstance(exc, json.JSONDecodeError):
        return "`gh` returned non-JSON output"
    if isinstance(exc, (AttributeError, TypeError, KeyError, ValueError)):
        return f"unexpected `gh` payload shape ({type(exc).__name__}: {exc})"
    return str(exc)


def open_issues() -> tuple[dict[int, tuple[str, str]] | None, str]:
    """{number: (native issue type, title)}, or (None, reason).

    The type has to come from the issue: `gh project item-list` reports the
    content *kind* ("Issue"), never `Feature` / `Bug` / `Task`.
    """
    try:
        data = gh_json(["issue", "list", "--repo", REPO, "--state", "open",
                        "--limit", str(ISSUE_LIMIT), "--json",
                        "number,title,issueType"])
        # `gh issue list` reports no total, so a full page is the only
        # truncation signal there is. Refusing to check beats checking a slice.
        if len(data) >= ISSUE_LIMIT:
            raise Truncated(f"more than {ISSUE_LIMIT} open issues — "
                            "raise ISSUE_LIMIT")
        return {
            int(i["number"]): ((i.get("issueType") or {}).get("name") or "",
                               i["title"])
            for i in data
        }, ""
    except FETCH_ERRORS as exc:
        return None, reason(exc)


def board() -> tuple[dict[int, dict] | None, str]:
    """{issue number: item}, or (None, reason).

    `gh project item-list` lower-cases custom field names in its JSON output.

    The board accumulates every closed issue too, so it outgrows the open-issue
    count without bound. A silent truncation here is the worst failure the
    script has: items past the cap vanish, their issues get reported as "not on
    the board", and `--check` fails a healthy board. `totalCount` catches it.
    """
    try:
        data = gh_json(["project", "item-list", PROJECT, "--owner", OWNER,
                        "--format", "json", "--limit", str(BOARD_LIMIT)])
        items = data.get("items", [])
        total = data.get("totalCount")
        if total is not None and len(items) < total:
            raise Truncated(f"board truncated at {len(items)} of {total} items"
                            " — raise BOARD_LIMIT")
        out: dict[int, dict] = {}
        for item in items:
            num = (item.get("content") or {}).get("number")
            if num is not None:
                out[int(num)] = item
        return out, ""
    except FETCH_ERRORS as exc:
        return None, reason(exc)


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


def shipped_disagreement(item: dict) -> str | None:
    """The one predicate for "these two fields tell the same story".

    `Status` (the workflow's word, Done on close) and `Horizon` (the planning
    word, Shipped when it landed) must agree — expressed once, as an equality
    between the two readings, so neither side can be edited into disagreeing
    with the other. Both blank agrees: an item nobody has classified yet is
    caught by `missing_fields`, not here.
    """
    status = item.get("status") or ""
    horizon = item.get("horizon") or ""
    if (status == "Done") == (horizon == "Shipped"):
        return None
    return f"Status={status or '(blank)'} / Horizon={horizon or '(blank)'}"


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

    agree_cases = [
        # (item, should it be reported?)
        ({"status": "Done", "horizon": "Shipped"}, False),
        ({"status": "Todo", "horizon": "Later"}, False),
        ({"status": "In Progress", "horizon": "Now"}, False),
        # the drift this check exists for: closing flipped Status, not Horizon
        ({"status": "Done", "horizon": "Next"}, True),
        ({"status": "Done", "horizon": ""}, True),
        # and the mirror: marked as landed while still open on the board
        ({"status": "Todo", "horizon": "Shipped"}, True),
        # nobody has touched this item at all — missing_fields' business
        ({}, False),
    ]
    for item, want in agree_cases:
        got = shipped_disagreement(item) is not None
        if got != want:
            fails.append(f"shipped_disagreement({item!r}) reported={got}, want {want}")

    def cpe(stderr: str) -> subprocess.CalledProcessError:
        exc = subprocess.CalledProcessError(1, ["gh"])
        exc.stderr = stderr
        return exc

    reason_cases = [
        (FileNotFoundError(), "not found"),
        (cpe("error: missing required scopes: project"), "scope"),
        (cpe('Post "https://api.github.com/graphql": tls: bad certificate'), "network"),
        # the shapes `gh` actually emits: the diagnosis is followed by the
        # remedy, so reading only the last line names the wrong cause
        (cpe("error: your authentication token is missing required scopes "
             "[read:project]\nTo request it, run: gh auth refresh -s read:project"),
         "scope"),
        (cpe('Get "https://api.github.com": dial tcp: lookup api.github.com: '
             "no such host\ntry again later"), "network"),
        (subprocess.TimeoutExpired(["gh"], GH_TIMEOUT), str(GH_TIMEOUT)),
        (KeyError("title"), "payload shape"),
    ]
    for exc, want_sub in reason_cases:
        if want_sub not in reason(exc):
            fails.append(f"reason({exc!r}) = {reason(exc)!r}, missing {want_sub!r}")

    # `gh` stubbed out: truncation must refuse to report, and a payload of the
    # wrong shape must come back as a named cause rather than a traceback
    real_gh_json = globals()["gh_json"]
    try:
        globals()["gh_json"] = lambda _args: [
            {"number": n, "title": "t", "issueType": None} for n in range(ISSUE_LIMIT)
        ]
        try:
            open_issues()
            fails.append("open_issues accepted a full page instead of raising")
        except Truncated:
            pass

        globals()["gh_json"] = lambda _args: {"items": [{}], "totalCount": 7}
        try:
            board()
            fails.append("board accepted a short page instead of raising")
        except Truncated:
            pass

        globals()["gh_json"] = lambda _args: {"items": [], "totalCount": 0}
        if board() != ({}, ""):
            fails.append("board rejected an honestly empty page")

        globals()["gh_json"] = lambda _args: [{"nope": 1}]
        got, why = open_issues()
        if got is not None or "payload shape" not in why:
            fails.append(f"open_issues on a bad shape = {got!r}, {why!r}")

        globals()["gh_json"] = lambda _args: "a string, not a board"
        got, why = board()
        if got is not None or "payload shape" not in why:
            fails.append(f"board on a bad shape = {got!r}, {why!r}")
    finally:
        globals()["gh_json"] = real_gh_json

    for f in fails:
        print("selftest FAIL:", f)
    print("selftest OK" if not fails else f"selftest: {len(fails)} failure(s)")
    return 1 if fails else 0


def report(issues, issues_err, items, board_err) -> tuple[list[str], list[str]]:
    """(findings, unchecked) — each dimension contributes to whichever it can.

    A dimension that failed to fetch lands in `unchecked` and the others still
    run: the board alone answers the Status/Horizon question, so an issue-list
    failure must not silence it.
    """
    findings: list[str] = []
    unchecked: list[str] = []

    if items is None:
        unchecked.append(f"board unreachable ({board_err})")
    else:
        for num, item in sorted(items.items()):
            why = shipped_disagreement(item)
            if why:
                findings.append(f"⚠ #{num} Status and Horizon disagree — {why}")

    if issues is None:
        unchecked.append(f"issues unreachable ({issues_err})")
    elif items is not None:
        for num in sorted(set(issues) - set(items)):
            findings.append(f"⚠ #{num} is not on the board — {issues[num][1]}")
        for num in sorted(set(issues) & set(items)):
            itype, title = issues[num]
            gaps = missing_fields(items[num], itype)
            if gaps:
                findings.append(f"⚠ #{num} ({itype or 'no type'}) missing "
                                f"{', '.join(gaps)} — {title}")

    return findings, unchecked


def main() -> int:
    # stdout is redirected on Windows more often than not, and the default
    # locale codec there cannot encode the markers below.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")

    argv = sys.argv[1:]
    unknown = [a for a in argv if a not in FLAGS]
    if unknown:
        print(f"unknown argument(s): {' '.join(unknown)}\n"
              f"usage: board_check.py [{' | '.join(sorted(FLAGS))}]",
              file=sys.stderr)
        return 2
    if "--selftest" in argv:
        return selftest()
    check = "--check" in argv

    try:
        issues, issues_err = open_issues()
        items, board_err = board()
    except Truncated as exc:
        # Not a degradation: the fetch cap in this file stopped being big
        # enough, and a partial view reports classified issues as missing.
        print(f"⚠ {exc}")
        return 2 if check else 0

    findings, unchecked = report(issues, issues_err, items, board_err)

    for line in findings:
        print(line)
    for line in unchecked:
        print(f"· {line} — not checked.")
    if not findings and not unchecked:
        print(f"✓ all {len(issues or {})} open issues are on the board, "
              "classified, and agree with their Status.")

    if unchecked:
        return 2 if check else 0
    return 1 if (findings and check) else 0


if __name__ == "__main__":
    raise SystemExit(main())
