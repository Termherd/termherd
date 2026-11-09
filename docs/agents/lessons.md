# Lessons behind the rules

Each section is the incident behind a one-line rule in `AGENTS.md` or
`crates/app/AGENTS.md`. Read it when a rule seems arbitrary or when you
are about to relax one. A new lesson gets its rule line there and its
story here, never both in full.

## No global mutable state, the environment included

**No global mutable state.** No `static mut`, no `lazy_static`, no
require-time singletons. Construct dependencies in `main()` and inject.
The environment counts: a store reading `$HOME` let 28 tests rewrite the
user's `~/.termherd` on each `cargo test` (#407). `paths::DataDir::user()`
and `paths::ClaudeDir::user()` are `main()`'s alone (the second since #432).
The guard `no_test_writes_the_data_dir_of_the_home_it_runs_in` (`.termherd` and
`.claude`) covers the app's unit tests only, not `crates/app/tests/` or
other crates.

## A cfg-gated API is not compiled by the PR gate

**A `cfg`-gated API is not compiled by the PR gate — cross-check it
yourself.** The `cross-os` job does not run on pull requests, so
`cargo check --target x86_64-pc-windows-msvc` is the *only* thing standing
between a Unix-only call and a broken packaging build. `#236` called
`MasterPty::process_group_leader`, which `portable-pty` declares under
`#[cfg(unix)]`, from an ungated file: green on every PR check, and it would
not compile for Windows. Worse, the code's own doc-comment asserted the
opposite ("`portable_pty` reports none on Windows") — true of the *value*,
but the method is absent, so there was nothing to return none. Whenever a
diff touches a dependency's platform-conditional surface, add the target once
(`rustup target add x86_64-pc-windows-msvc`) and check both ways: the change
compiles, and reverting it fails.
**A test that needs an OS-only API belongs in `tests/`, not in the
containment allow-list.** `check-os-cfg-containment.sh` scans `*/src/**`
only, so an integration test may use `std::os::unix::fs` freely — whereas
adding the adapter's own source file to the allow-list to accommodate one
test would licence OS-conditional *production* code there unnoticed, which is
what the quarantine exists to prevent. Budget for one surprise on the way:
`clippy.toml`'s `allow-expect-in-tests` recognises `#[cfg(test)]` items, not
a `tests/` binary, so such a file needs its own `#![allow(clippy::…)]` with a
reason.

## Code that runs differently per host

**Cross-compiling is not the whole hazard — code that *runs* differently per
host is worse, because no `cargo check` catches it.** `#239`'s shell
integration built the replayed startup path with `Path::join`, which writes
the host's separator into what is a line of **POSIX shell**: correct on Unix,
a backslash escape on Windows, so the user's own rc was silently never
sourced. It compiled everywhere and three tests asserted the Unix spelling.
Because `cross-os` skips PRs *and* is path-filtered on `rust`, two docs-only
merges went by before the next `.rs` push to `main` surfaced it — the break
was attributed to that push, not to the one that caused it. The rule:
**a string destined for another grammar takes that grammar's separators, not
`std::path`'s.** When a `Path` is being rendered into a script, a URL, or a
wire format, join by hand. And when `main` goes red on a Rust push, check
whether the previous Rust push is the real author before reading the diff in
front of you.

## No spawn_blocking inside an iced Task

**Async work inside an `iced::Task` is not on tokio — `spawn_blocking` there
is fatal, and silently so.** `iced` is built here without its `tokio`
feature, so `Task::perform` polls on the `futures` thread pool, and `main`
hosts the MCP runtime with `block_on` rather than entering it. A
`tokio::task::spawn_blocking` in that future calls `Handle::current()` and
panics. The panic is invisible twice over: the effect it belonged to simply
never answers, *and* the unwind takes a pool worker with it — the pool does
not respawn one, so after roughly `num_cpus` occurrences **every** `Task` in
the app stops completing (scans, captures, the PNG encode, the bridge). The
clickable-path resolution (#252) was written this way and reached review with
839 tests green, none of which could have caught it: no test polls a returned
`Task`. Do blocking work directly in the `async` block, the way
`scanner.scan()`, `docs::discover` and `capture::write_png` already do; reach
for a `std::thread` + channel when it must outlive the task. The rule is
about *where the future runs*, not about which file it lives in: a `tokio::`
call is fine in code the runtime spawned (`shell::bridge`, `app::mcp`) and in
a `#[tokio::test]`, and wrong in anything handed to `Task::perform`.

## A tab index is a copy core may move

Parking (#353) let `core` reorder the strip on its own, after a drag or a
parked session asking for attention, and the shell's `closing`, `tab_drag`
and `split_drag` kept the old slot: an armed close confirmation would have
closed the neighbour. A first fix re-anchored them by session after each
message (`TabAnchors`); #454 then made each hold a session of its tab, as the
tab rename already did, and resolve the position with `Workspace::tab_of`
when it reads it. A pane closing re-anchors all four through one `follow` in
`after_pane_vanished`. New shell state names a tab the same way.

## An invariant expressed twice drifts

**An invariant expressed twice will drift — extract the predicate.** Two
call sites deciding "has this settled?" with hand-written conditions is a
bug waiting on the first edit that touches one of them. It bit the
`wait_for_status` rung: one site treated a session exit as settling a wait,
the other only compared against the requested statuses, so a wait placed
after a crash parked until the caller's timeout. Both now go through one
`settles()` predicate. A doc-comment asserting the rule is *not* enforcement
— the comment describing the correct behaviour sat directly above the code
that broke it.

## A flag breaks whoever depended on its side effects

**Putting a shipped behaviour behind a flag breaks whoever depended on its
side effects, not on its call site.** Grep for what it *wrote*, not for who
called it. Making copy-on-select configurable gated the two places that copy
a selection — and both were the only writers of the shell's last-copied
cache, which is all the copy chord ever read. Off by default, a mouse
selection could then be copied by no gesture at all: the flag disabled a
feature nobody had flagged. Every test passed, because each covered its own
side of a seam that no longer met. The first fix was a *reordering* rather
than only a fallback — the chord read the live selection first and kept the
cache behind it, for the one case the screen could not answer: a selection
scrolled out of the viewport carries no visible spans. Putting the live read
first also killed an older bug, where a stale cache outranked a fresh
highlight. A cache named after what filled it (`selection`) that actually
holds what was last *copied* is how the gap stayed invisible; name a cache
for its contents. The cache kept "behind" the live read was not harmless
either: once a program owns the mouse (#155) the terminal *never* has a
visible selection in that pane, so the fallback fired on every `copy` there
and overwrote the clipboard the program had filled itself (#316). The cache
is gone: a `Screen` now says whether the terminal holds a selection at all
(`has_selection`, true even when it scrolled out of view), and the chord
asks the terminal or refuses. **A fallback that fires whenever a
precondition is absent becomes a bug the day a feature makes that
precondition permanently absent** — the day the mouse became the child's,
"no visible selection" stopped meaning "look somewhere else".

## Cleanup that only runs at teardown does not run

A crash skips teardown, and so does every OS-initiated exit — on macOS the
whole AppKit `terminate:` family: Dock → Quit, logout, restart. The case
nobody pictures is the run that *opts out*: it has no teardown of its own, so
it silently inherits whatever the last run left on disk. The attach rung's
published `bridge.json` had all three holes, and the third was permanent — an
endpoint advertised forever, with a token a foreign process on the recycled
port could be handed (#267). Put the reconciliation at **startup**,
unconditionally and before the decision to publish: the one moment that always
happens and the one moment you know what the file should say. Keep the
teardown as well, for the clean exit; never let it be the only one.

## stdout belongs to the product

`tracing` initialises to stdout by default, which is invisible in a GUI and
fatal the day the binary grows a subcommand whose output is meant to be piped:
`termherd mcp-config > .mcp.json` would capture a warn line in the middle of
the JSON. The app installs a stderr writer; `2>&1` recovers the old behaviour
when reading logs. A release build on Windows writes to
`~/.termherd/termherd.log` instead (#405), so there `2>&1` recovers nothing:
read the file.

## A guard is unreachable or tested

**A guard is unreachable and goes, or reachable and gets a test — there is
no third state.** Defensive arithmetic nobody can trigger is not free: it
reads as a live case to the next reader, and no test can pin it. Mutation
testing finds these by construction, because a mutant of dead code changes
nothing observable and survives. In `image::resample_box` the clamps were
provably unreachable under its shrink-only precondition and went, while
`count > 0` — all that stands between a box with no readable pixel and a
divide by zero — was reachable and earned a truncated-buffer test. Both
survivors looked like missing assertions and were really design smells.

## A value that reaches a command line

**A value that reaches a command line is not a setting like the others — and
a charset allowlist does not make it one.** Two lessons from the same seam.
The resume-id fix validated the stem to `[A-Za-z0-9_-]+` and still admitted
`--help.jsonl`: a *flag-shaped* token passes the charset, and
`claude --resume --help` parses it as a flag, not a value. Validate at the
seam where the string becomes an `argv`, reject a leading `-` (or pass `--`)
on top of any allowlist. The wider form bit the stdio MCP server, which
listed `shell.program` / `shell.args` as writable beside `theme`: the value
is what the next launch *executes*, so an unattended agent could pick it.
Now `OptionSpec` carries `writable`, the exec-carrying options are read-only
over MCP, and a shape mismatch is refused rather than warned-and-written.
When an option is added to a catalogue, ask what its value *reaches*, not
only what type it is.

## A check that can pass without exercising anything

**A check that can pass without exercising anything is not a check.** The
cheap probe and the real call are rarely the same call, and the cheap one is
the one that gets written. Probing whether macOS would let an agent drive the
app, `osascript -e 'keystroke ""'` **succeeds without the Accessibility
permission** — an empty string never reaches the TCC check — so a poll built
on it reported the permission granted while every real keystroke was still
refused, and the pass was believed for a full turn. Same shape as a linter
that exits 0 having linted nothing, and as `mutants.out/outcomes.json` read
mid-run (see `.wrap.md`): success and vacuity are indistinguishable from the
outside. Probe with the operation you actually intend to perform, and if that
is destructive, assert on something the operation *must* have changed.

## Poll for the value you assert

**Poll for the value you assert, not for a sign it is coming.** The argv
spawn test waited for the copy's target to exist, then read it. On Windows
the target existed but was still locked by `copy`, so the read failed. Filed
twice with two guessed causes (#280, #437), while 200 reruns passed; a 300 MB
payload that slowed the copy made it fail at once. Force a suspected race by
widening its window, not by rerunning.

## Proptest seed files follow the source path

**A proptest seed file is keyed by the test's source path, so moving the
test orphans it without a word.** proptest's default persistence reads
`proptest-regressions/<path of the .rs file>.txt`, derived from `file!()`.
When the scroll-drift property left `shell/terminal.rs` (for
`shell/terminal/mod.rs`, then `shell/terminal/canvas.rs`), its two committed
seeds stopped replaying from 2026-07-12 to 2026-09-25, and nothing reported
it: an unread seed file gives no signal either way. When a refactor moves a
file holding a `proptest!`, `git mv` its regressions file in the same commit.
The file is test input to CI as well: the `rust` path filter now watches
`proptest-regressions/**`, since the PR that moved it came up green with
`test` and `portable` both skipped.

## An unfinished check is not a passing check

**An unfinished check is not a passing check, and reporting it once is not
reading it.** `portable` is the only gate that runs a Windows *behaviour*
before merge, and it takes minutes while every other job takes seconds — so
the natural moment to look at the checks is the moment it is still pending.
A repo-add push was reported that way ("three jobs pending"), nobody went
back, and the same Windows failure survived **three** consecutive pushes,
each of which read as green in the summary that had been looked at. Poll
until the run is `completed`, then read the conclusion; a job list with a
`pending` in it says nothing about the branch.

## An expectation defined by the other side

**An expectation defined as "whatever the other side said" cannot tell a
wrong answer from a wrong question.** Comparing a declaration's key against
the walk's own output is the right *shape* — it is what stops the two rules
drifting — but it holds only if the fixture the walk answered about is the
one the test meant to ask about. Every fixture wrote its transcript as
`abc.jsonl`, so the session id was ambiguous across them; the scan keeps both
records rather than merging them, both satisfied the lookup, and `find`
returned whichever the filesystem enumerated first. A test about one
directory was answered about another: correct on APFS, a neighbour's answer
on ext4 and on Windows. Two habits close it — make a fixture's identifier
unique, and spell the expected value out *beside* the cross-check instead of
only comparing the two sides, so a wrong fixture fails on any platform.

The same holds when both sides are one table read in two directions. The
chord name table maps a name, the key iced reports and the terminal key, and
the round trips read it name → iced key → name. Swap the iced keys of the
`f5` and `f6` rows and every test in `crates/app` still passed: each lookup
lands on the other's row, and the two wrong turns cancel. Even the
test spelling every chord's *bytes* could not fail, because a synthesised
press reaches the bytes through the same swapped row; only a real F6 would
have sent F5's sequence. A round trip proves the table agrees with itself,
not with the keyboard. Each column a round trip crosses needs a list written
independently of the table, plus a check derived from the table that the list
matches it row for row. Found while reworking #321:
`a_real_named_key_resolves_to_its_spelled_name_and_terminal_key` is that
list, and the same swap now fails it and the row-for-row check.

## A test that claims to be exhaustive

**A test that claims to be exhaustive is worse than no test when it is not.**
A hand-written list asserting "this *set* is the contract" reads as a
guarantee, so nobody re-derives it — where an absent test at least leaves the
reader suspicious. `every_action_that_can_refuse_reports_which_kind_of_nothing`
said so in its own comment and listed five of the seven actions that refuse;
the two it missed (`copy` with nothing selected, `close-focused` on an empty
workspace) went on reporting success at doing nothing, which is the exact
failure the test existed to kill. Either derive the list from the same source
the code uses, or make each case fail loudly on its own — never assert
completeness in prose above an enumeration a human typed.
Deriving the list is necessary and not sufficient, because an **exemption**
is where a derived list quietly becomes a typed one again.
`every_tool_answers_its_structured_content_as_an_object` reads its tools from
the router and still excused one by hand: `screenshot`, on a comment claiming
it answers no structured content — untrue, and it was also the one tool
assigning `structured_content` outside the shared guard, so the only tool
bypassing the rule was the only one the sweep skipped. Verify the claim that
justifies an exemption, or drop the exemption and write the case a fixture.
(Review caught this inside #302 and it was squashed away, so `git log` on
`main` will not show it — the PR is where the evidence is.)

## Two green pull requests can merge red

**Two pull requests can each be green and merge red together.** Branch
protection here is not strict, so a PR whose checks ran against an older
`main` merges without re-running them. #369 and #359 both extended a sweep
over `KeyboardOwner::ALL`: #359 asserted that no prompt lets a notification
click switch tabs, and #369's shared `arm_overlay` opened the tab menu on tab
0, which focuses that tab before the click. Each PR passed alone, and `main`
failed the moment both had landed (fixed in #376). When two open PRs touch
the same sweep or the same shared test helper, update the second with `main`
and wait for its checks before merging it. And a helper that arms a state
for a sweep must not change what the sweep then asserts on.

## Claude Code's bytes drift

**Claude Code's screen is matched on bytes, and its bytes drift.** Three
shapes termherd read off Claude's terminal broke in one day, each passing
tests built on what we assumed Claude draws: the `◐` / `◑` busy glyphs of
Claude Code 2.1 (#371), a no-break space after the `❯` prompt marker, and
the session name written into the rule above the prompt (#374). A
screenshot cannot show the no-break space. Build the fixture from a capture
of the live pane instead: `⌘⇧S` writes the visible text to
`~/.termherd/captures/`, and `xxd` on that row shows the bytes.
Its **input** is read on bytes too, and not as a shell reads it: `\n` is
Ctrl+J, a line break in the prompt, and only `\r` is Enter. A burst of
about 64 bytes or more is taken as a paste, inside which even `\r` is a
line break, so Enter must arrive as a write of its own, after a gap
(#426, fixed in #430). `claude_command` was the last caller writing its
Enter with no gap (#439); #467 gave it one, and that was not enough. A
**control byte** inside the burst is stripped as well: Claude Code shows
`Removed 1 invisible character · review and press Enter to send` and holds
the line, so the Enter after the gap only acknowledges the review. The
command's Ctrl+U now goes alone, then the line and Enter as paced writes
(#496). What settled it was sending the three byte sequences by hand over
the attach bridge before changing any code.

## The book drifts from the binary

The *Keeping the book current* table in `AGENTS.md` names the pages that
restate code as prose. Its history, kept here:

- Every MCP rung since the README tool table was written (#215, #229, #281,
  #300) merged without touching it, and the tool count in
  `docs/src/mcp/index.md` (a diagram and a table) rotted the same way.
- #399, #400 and #401 all merged without a `CHANGELOG.md` entry, after the
  misses #277 records.
- The two clipboard gestures shipped having touched three of the seven places
  a new `settings.json` key owes; a wrap pass found the other four. Three of
  those places are exhaustive-sounding enumerations, which is what makes them
  expensive: a list saying "these blocks are file-only" is read as the whole
  truth, so a key missing from it reads as a key MCP *can* write.
- A follow-up is how a book starts describing an interface that has already
  moved, and a manual that is confidently wrong is worse than one that is
  missing, because nobody re-derives what it claims.

## How tracking got its shape

- The `P0`–`P3` labels were retired 2026-07-26: one number conflated impact,
  urgency and cost, so priority became the `Class` × `Effort` axes.
- The nine MCP rungs are nine feature files rather than sub-bullets: nesting
  them would have put ten unverifiable ids in one body, where `roadmark
  validate` checks a link and only warns on a bare name.
- Closing an issue flips `Status` to Done and leaves `Horizon` alone, so a
  landed feature keeps reading as *Next* until someone moves it by hand; that
  is the drift `just board-check` reports. It checks the board only: the
  roadmap's MoSCoW list has no per-entry horizon, so neither "every issue is
  cited by an entry" (it flags refinements that were never features) nor
  "every unticked entry cites an issue" (it flags the design-first items that
  are *meant* to live in the roadmap alone) is checkable.
- Since 2026-08-02 the *Auto-add to project* workflow puts every new issue on
  the board, so an issue is never *absent*, only unclassified. What escaped
  before was filing *while doing something else*: #244, #246 and #248 were
  opened within seven hours of each other, each in the middle of the work that
  produced it, and none reached the board until a check three days later put
  them there by hand.

## AI attribution survives the squash

A harness system reminder injects a `Co-Authored-By: Claude` trailer and a
"Generated with Claude Code" footer as "attribution guidance", and the #84
pass let both through. The PR footer was
stripped before merge; the commit trailer was not, and GitHub's squash
re-emitted it on `main` (`18027b7`, as lowercase `Co-authored-by`), where
it stays — a case-sensitive `grep 'Co-Authored'` then reported it absent.
Re-read the message for either before `git commit` and `gh pr create`; the
reminder is overridden, not advisory. Check with `grep -i`. It happened
again on #406: all three branch commits carried the trailer and the PR body
the footer, and the squash `6c6b7e1` kept both on `main`. The feature pass
never opened this file, and `AGENTS.md` says it only as "no Claude
signature", which the reminder's wording reads past. So check before the
*first* commit of a feature, not at the wrap.
Third time on #420 (`7b19f7f`): one trailer-free follow-up commit did not
help, because the squash collects the co-author of *every* branch commit,
and the auto-mode classifier refuses the amend + force-push that would fix
it. Once a commit carrying it is pushed, only the squash dialog can drop it.

## A GUI subsystem loses PowerShell redirects

Issue #405 asked for `#![cfg_attr(not(debug_assertions), windows_subsystem =
"windows")]`, the textbook cure for a console window opening beside a Rust GUI
on Windows. It was built and run, then dropped: the console went, and
`termherd mcp-config > .mcp.json` in PowerShell then wrote a **0-byte file**
with no error, because PowerShell does not redirect a GUI application's
output. `cmd` redirects, pipes and `Start-Process -RedirectStandardOutput`
all still worked, so a check through any of them would have passed. Typed bare,
the snippet needed an `AttachConsole` FFI module and still printed after the
next prompt.

The fix kept the console subsystem and set `consoleAllocationPolicy` to
`detached` in the manifest (#508): Windows 11 24H2 and later allocate no
console when nothing hands one over, and a terminal still lends its own.
`tests/windows_manifest.rs` reads the policy back out of the built binary. The
general trap: a change to how a binary is linked is a change to every
subcommand's streams, and the one shell to test is the one users have.
