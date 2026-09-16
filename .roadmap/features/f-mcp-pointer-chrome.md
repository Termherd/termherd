+++
id = "F-mcp-pointer-chrome"
type = "feature"
area = ["mcp", "workspace"]
status = "todo"
target = ["Could"]
+++

The pointer rung, chrome half: click and drag termherd's own interface.

The pointer rung, chrome half: click and drag termherd's **own** interface —
sidebar rows, the tab strip, split gutters — the way `press_keys` presses its
own keyboard. Filed as #301, `needs-design`.

A set of affordances exists only under the mouse, so no agent can reach or
regression-test any of them: drag a split gutter (#55), drag a tab to another
window (#153) or out to detach it (#154), auto-scroll a drag-selection at the
edge (#157), alt+drag for column selection (#159), Cmd/Ctrl-click a hyperlink
(#84, shipped — the gesture exists and an agent still cannot make it). The
argument that produced [F-mcp-keys](#f-mcp-keys) — a surface an
agent cannot reach is a surface an agent cannot regression-test — applies
unchanged to the pointer, with a larger uncovered surface behind it.

Pixel addressed, because the chrome is not a grid: `mouse_at_app(kind, x, y,
…)` in the same frame `screenshot` returns, so the loop is *see the pixels,
click the pixels* with no third coordinate system to reconcile. The answer
names what the click reached and the resulting focus, and an open overlay eats
a click as it eats a chord.

Open design questions, which is why it stays design-first: whether a drag is
one call carrying a path or a press / move / release sequence — the drag-heavy
tickets above should settle it — and how it reconciles with the keyboard rung's
invariant that neither tool may reach a state the keyboard cannot, since a
gutter drag is exactly such a state.

Landing it falsifies several claims written while no MCP caller had a pointer:
the "which is every MCP caller" premise on `escape` in `shell/routing.rs` and
its sweep in `shell.rs`, "Mouse-only, which is no one, over MCP" in
[F-mcp-keys](#f-mcp-keys), and "Two tools reach TermHerd's own interface" in
the manual's keyboard page.

Sibling of [F-mcp-pointer-terminal](#f-mcp-pointer-terminal), which is the half
that blocks #155.
