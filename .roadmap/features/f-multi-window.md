+++
id = "F-multi-window"
type = "feature"
area = ["workspace"]
status = "todo"
target = ["Could"]
+++

More than one termherd window, and tabs that travel between them.

More than one termherd window, and tabs that travel between them. Filed as
three issues in dependency order: **#149** opens a second window
(`mod+shift+n`) — architectural, since the shell is built on
`iced::application` (single-window) and would convert to `iced::daemon`;
**#153** moves a tab to another window by drag-and-drop and **#154** detaches
one by dropping outside any window, both *blocked by* #149 and both reusing the
in-window `TabDrag` plumbing that already reorders tabs. The gate is #149's
conversion: `core::Workspace` is one tree today, so "which window owns this
tab" has no representation yet
