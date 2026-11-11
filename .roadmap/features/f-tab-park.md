+++
id = "F-tab-park"
type = "feature"
area = ["workspace", "keymap"]
status = "done"
target = ["Could"]
+++

Park a tab: a compact chip at the strip's end, out of the tab cycle.

Presentational only: the session keeps running and its status still flows
(#353). Active tabs come first and parked tabs after, an order every
`Workspace` mutator keeps. Selecting a parked tab, dragging it out, or its
session asking for attention unparks it; dragging a tab into the parked
zone parks it. A natural entry for [F-tab-context-menu](#f-tab-context-menu).
Torture report: `.personal/feature-torture/reports/F-tab-park.md`.

Shipped (#353): `toggle-park-tab` with no default chord, a *Park tab* entry in
the tab menu, and `parked` on the snapshot. Attention unparks without taking
the focus; a drag takes the zone it lands in.

Deferred to a later tidy-first (#454), and since done, in three slices: shell
tab state stored by session rather than as an index re-anchored after each
message (#472); `core` reporting a change of the shown tab as
`Effect::TabShown` (#456); and one drop-zone rule, `Workspace::lands_parked`,
which `move_tab` applies and the strip's caret reads (#572). A drop from
another window (#153) has no `from` in the target strip, so it will need the
rule keyed on the carried tab's zone rather than its index.
