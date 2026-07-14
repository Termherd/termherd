//! The out-of-band events the PTY host emits back to the runtime, and the sink
//! that carries them. [`PtyEvent`] names [`crate::grid::Screen`] for its output
//! payload; nothing in the crate depends back on this module.

use std::sync::Arc;

use termherd_core::SessionStatus;
use termherd_core::workspace::SessionId;

use crate::grid::Screen;

/// What the adapter emits back to the runtime, out-of-band. The iced shell
/// maps these onto `core` events.
#[derive(Debug, Clone)]
pub enum PtyEvent {
    /// New terminal screen contents — the visible grid with per-cell colour
    /// and the cursor (FR4).
    Output { session: SessionId, screen: Screen },
    /// Activity reclassified from the OSC stream (FR8).
    Status {
        session: SessionId,
        status: SessionStatus,
    },
    /// The session's reported title changed; drives the tab label.
    Title { session: SessionId, title: String },
    /// An OSC 9 notification fired: Claude wants the user. Carries the
    /// raw payload text, forwarded to the OS notification centre on top of the
    /// in-app `Attention` status (which `Status` already conveys).
    Notification { session: SessionId, body: String },
    /// The session's PTY process exited. `clean` is true when the child
    /// reported successful completion (exit code 0, no signal); false also
    /// covers a status the reaper could not observe.
    Exited { session: SessionId, clean: bool },
    /// The text of the session's current selection, in response to a copy
    /// request — read from the live grid selection so it is exact even right
    /// after a fast drag. Absent (no event) when nothing is selected.
    SelectionCopied { session: SessionId, text: String },
}

/// A sink for [`PtyEvent`]s. Cheap to clone, callable from the reader threads.
pub type EventSink = Arc<dyn Fn(PtyEvent) + Send + Sync + 'static>;
