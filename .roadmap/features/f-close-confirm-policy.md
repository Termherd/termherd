+++
id = "F-close-confirm-policy"
type = "feature"
area = ["workspace"]
status = "done"
target = ["Must"]
+++

Configurable confirmation on tab close and app quit, keyed on activity.

Configurable close confirmation for tab close and app quit (`close.tab` /
`close.app` in `settings.json`, each `alwaysConfirm` / `confirmWhenActive` /
`noConfirmation`). One pure decision seam (`ConfirmClose::confirms(active)`)
backs both paths; `confirmWhenActive` reuses #140's `has_running_process`
predicate — a tab keys off `App::tab_has_running_process`, a quit off the
app-wide `any_running_process` (both over `LiveSession::has_running_process`),
so an idle shell closes/quits silently while a working shell or live Claude
confirms. Both default to `confirmWhenActive`, preserving #140's shipped tab
behaviour and extending the same predicate to quit — which is #80 (an all-idle
app now quits without a prompt). Built on #79/#140 (closed); the config surface
is the new part. Known gap: the predicate reads a plain shell running a
non-Claude foreground program (vim, a long `make`) as idle, so it can be
closed/quit silently — better foreground-process detection is tracked in #143
