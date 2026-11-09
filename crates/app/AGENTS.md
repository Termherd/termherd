# crates/app — rules for the GUI shell

Read with the root [`AGENTS.md`](../../AGENTS.md); these rules apply only to
code under `crates/app/`. The incidents behind them are in
[`docs/agents/lessons.md`](../../docs/agents/lessons.md) and
[`docs/agents/mcp.md`](../../docs/agents/mcp.md).

## Async and threads

- **No `tokio::task::spawn_blocking` in a future handed to `Task::perform`.**
  `iced` runs without its `tokio` feature, so the future polls on the
  `futures` pool: the call panics, the effect never answers, and the pool
  loses a worker for good. Do blocking work directly in the `async` block, or
  on a `std::thread` + channel when it must outlive the task. `tokio::` is
  fine in code the runtime spawned (`shell::bridge`, `app::mcp`) and in a
  `#[tokio::test]`.
  [Why](../../docs/agents/lessons.md#no-spawn_blocking-inside-an-iced-task)

## Shell state

- **A tab index the shell keeps is a copy of a position `core` may move.**
  Shell state that outlives a message names a tab by a session it hosts
  (`Shell::anchor_of`) and resolves the index with `Workspace::tab_of` when it
  reads it, as `closing`, `tab_drag`, `split_drag` and `tab_rename` do. A new
  anchor joins the `follow` in `after_pane_vanished`.
  [Why](../../docs/agents/lessons.md#a-tab-index-is-a-copy-core-may-move)

## The keyboard ladder

- Every overlay rung can be left with `escape`; the sweep over
  `KeyboardOwner::ALL` fails a rung added without an exit. The one hand-off it
  names is `DocDiscard`, whose `escape` gives the keyboard back to the editor.
- **Arm a destructive prompt on a deliberate gesture, never on an async
  answer**: a prompt raised by a reply can be answered by the next Enter meant
  for something else.
- **A modal scrim blocks the mouse, not the keyboard** (`opaque` captures
  pointer events only). A widget under a prompt takes input only while it owns
  the keyboard — `keyboard_owner()` is the one predicate the view and the
  handler both read.
- A handler that can refuse returns `Option` and says so at its own refusal,
  so `run_action`'s `inert` reasons need no list re-derived elsewhere.

## The pointer

- `PointerEvent::route` in `core::app::pointer` is the one predicate deciding
  whether a gesture is forwarded to the child, kept as a selection, or
  dropped; the terminal thread, the MCP shell and the canvas all read it.
  Under any mouse reporting it never answers `Select`.
- **A gesture's owner is decided at its press and kept to its release**
  (`canvas::Held`). A gesture the child holds follows the pointer off the
  grid, clamped to the border cell, and motion is reported once per cell.

## The MCP bridge

- Tools live in `src/mcp/handler.rs`, the transport in `shell::bridge`, the
  shell's answers in `shell::serve` — the one place an external caller meets
  `core::App`. `core` has no MCP awareness: every mutation goes through an
  existing `Event`.
- A new tool, argument or outcome updates the pages the root `AGENTS.md`
  lists under *Keeping the book current*, and the server instructions in
  `get_info` when it changes how the tools combine.

## Windows

- **`termherd.exe` stays on the console subsystem.** `windows_subsystem =
  "windows"` hides the console but makes `termherd mcp-config > file` write
  an empty file in PowerShell; the manifest's `detached` console policy
  (`build.rs`) hides it instead.
  [Why](../../docs/agents/lessons.md#a-gui-subsystem-loses-powershell-redirects)

## Paths

- **A canonical path that leaves the process** (to git, a shell, a file
  another tool reads) goes through `shell::links::resolved`, which drops
  Windows's verbatim `\\?\` prefix; a bare `canonicalize` there breaks on
  Windows only, after merge.
  [Why](../../docs/agents/lessons.md#code-that-runs-differently-per-host)

## macOS FFI

`src/macos.rs` is the workspace's one `unsafe` module outside `crates/pty`
(whose `status::win_process` reads the Win32 process tree): the Cmd+Q quit
path, the Edit menu that gives Ctrl+Cmd+Space its *Emoji & Symbols* item, the
Window menu that gives Cmd+M its *Minimize*, the `insertText:` relay that
turns Character Viewer text winit drops into an input-method commit, and the
`GetURL` Apple Event relay that drops a `termherd://` link into the inbox.
