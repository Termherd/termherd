+++
id = "F-app-shell"
type = "feature"
area = ["workspace"]
status = "done"
target = ["Must"]
+++

The window itself: lifecycle, saved bounds, and the native menu gap.

Window, lifecycle, bounds (menu: deferred to M3 with the keymap — no native
menu API in iced; menu items mirror keymap actions). The deferral left one
visible gap on macOS: winit builds only the *application* menu, so there is no
Window menu and **⌘M does not minimize** (#114) — a system gesture users expect
of any window, not a keymap preference
