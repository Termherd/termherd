//! `termherd-pty` — the PTY host adapter (M2).
//!
//! Implements [`termherd_core::ports::PtyHost`]. Each session is owned by its
//! own OS thread that holds the PTY reader and an `alacritty_terminal` grid;
//! the rest of the system talks to it only through this manager's control
//! methods (`write`/`resize`/`kill`) and receives output/exit through a sink
//! given at construction. There is no shared `&mut Session` — the structural
//! fix for the `realSessionId` race (Q6, `docs/PRD.md` §4).
//!
//! Output is a [`Screen`] snapshot of the visible grid: per-cell RGB (xterm
//! 256 palette), the cursor, and a scrolled flag (FR4). Selection is the one
//! FR4 item still pending.
//!
//! The concerns live in submodules under the crate root:
//! - `input` — the terminal input byte protocol (keys, wheel, paste); a pure,
//!   GUI-free leaf.
//! - `grid` — the [`Screen`]/[`Palette`] rendered-cell types and the
//!   snapshot/colour/selection code over an `alacritty_terminal` grid.
//! - `events` — the out-of-band [`PtyEvent`]s and the [`EventSink`].
//! - `launch` — spawn/launch policy: command line, environment, and every
//!   private per-session file — the mcp config, the settings overlay, the
//!   shell-integration startup files — including their private-mode creation
//!   and their removal at teardown.
//! - `prompt` — the OSC 133 shell-integration marks a shell reports its
//!   prompt and command boundaries with; a pure leaf.
//! - `workdir` — the OSC 7 announcement a shell makes of the directory it is
//!   in, so a `cd` is not invisible; a pure leaf beside `prompt`.
//! - `integration` — the per-shell recipe (env, args, rc-file contents) that
//!   makes a shell emit those marks; pure data, performed by `launch`.
//! - `status` — where a session's activity comes from: the fold over Claude's
//!   OSC signals and the shell's prompt marks, plus the foreground-process
//!   fallback for a shell with no integration.
//! - `kill` — the OS-cfg kill reconciliation, quarantined in one file.
//! - `session` — the per-session actor: reader / waiter / terminal threads.
//! - `manager` — [`PtyManager`], the [`PtyHost`](termherd_core::ports::PtyHost)
//!   implementation that owns every session.
//!
//! Dependency direction: `manager → session → {grid, status, events, kill}`,
//! with `input`, `prompt`, `workdir` and `integration` free leaves and
//! `status` reading `prompt`.

mod events;
mod grid;
mod input;
mod integration;
mod kill;
mod launch;
mod manager;
mod prompt;
mod session;
mod status;
mod workdir;

pub use events::{EventSink, PtyEvent};
pub use grid::{Palette, Screen, ScreenCell};
pub use input::{KeyMods, TermKey, key_bytes, paste_bytes, wheel_bytes};
pub use manager::{PtyManager, Shell};
