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

Deferred to a later tidy-first (#454): shell tab state stored by session rather
than as an index re-anchored after each message, and one drop-zone rule shared
by the strip view and `Workspace::move_tab`. The third slice is done (#446):
`core` reports a change of the shown tab as `Effect::TabShown`.
