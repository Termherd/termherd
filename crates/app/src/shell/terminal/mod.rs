//! The embedded terminal. Three concerns, one per file: [`canvas`] renders the
//! grid and wires pointer events (the `canvas::Program`); [`selection`] holds
//! the pure pointer/selection geometry; and this module owns the shared cell
//! metric ([`cell_size`]) plus the OS handoffs ([`open_url`], [`notify`]) the
//! shell performs for the link-open and notification effects. The byte protocol
//! and the grid model live in `termherd_pty`.

mod canvas;
mod selection;

pub(super) use canvas::TerminalView;

/// Terminal cell metrics for the monospace grid, as ratios of the font size
/// so a zoomed font scales the grid proportionally. At the default
/// 14 px font they give the historical 8.4 × 18.0 cell. Used both to draw
/// and (in the parent) to translate the pane's pixel size into a PTY cell
/// geometry (FR4).
const CELL_W_RATIO: f32 = 8.4 / 14.0;
const CELL_H_RATIO: f32 = 18.0 / 14.0;

/// The cell box (width, height) for a terminal font size.
pub(super) fn cell_size(font_size: f32) -> (f32, f32) {
    (font_size * CELL_W_RATIO, font_size * CELL_H_RATIO)
}

/// Hand a detected link to the OS default handler. Fire-and-forget: the
/// child opener is spawned, not waited on. `url` has already been validated by
/// `core` (a recognised scheme, trimmed), and is always passed as a single
/// argument — never through a shell — so it can't be reinterpreted.
pub(super) fn open_url(url: &str) -> Result<(), termherd_core::ports::PtyError> {
    use std::process::Command;
    let spawn = |mut cmd: Command| {
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| termherd_core::ports::PtyError::Io(e.to_string()))
    };
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        spawn(cmd)
    }
    #[cfg(target_os = "windows")]
    {
        // `start` treats the first quoted argument as the window title, so the
        // empty "" keeps the URL from being swallowed as one.
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", url]);
        spawn(cmd)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        spawn(cmd)
    }
}

/// macOS bundle identifier (matches `Cargo.toml`'s packager `identifier`).
/// Used to attribute desktop notifications to TermHerd; see [`notify`].
#[cfg(target_os = "macos")]
const MACOS_BUNDLE_ID: &str = "dev.termherd";

/// Post a desktop notification to the OS notification centre. Like
/// `open_url`, this is an OS handoff, not a PTY call, and fire-and-forget: the
/// send runs on a detached thread and the result is logged there, never fatal —
/// a notification backend that's unavailable must not take a session down.
/// `title`/`body` come pre-derived from `core` (which session, what message).
///
/// **Why a thread, not a direct call:** on macOS the backend (`NSUserNotification`
/// via `mac-notification-sys`) drives an `NSRunLoop` to await delivery *when
/// invoked on the main thread*. iced calls `perform` from inside winit's event
/// handler, so pumping the run loop there re-enters it and aborts the process.
/// Off the main thread the backend takes a Condvar wait instead, so this is
/// both crash-safe and non-blocking for the UI.
pub(super) fn notify(title: &str, body: &str) -> Result<(), termherd_core::ports::PtyError> {
    // Attribute notifications to our bundle once, before the first send, so the
    // macOS backend doesn't AppleScript-probe for a placeholder app and pop a
    // "Where is …?" chooser. No-op (and harmless) when run unbundled.
    #[cfg(target_os = "macos")]
    {
        use std::sync::Once;
        static SET_APP: Once = Once::new();
        SET_APP.call_once(|| {
            let _ = notify_rust::set_application(MACOS_BUNDLE_ID);
        });
    }

    let (title, body) = (title.to_owned(), body.to_owned());
    std::thread::Builder::new()
        .name("os-notify".to_owned())
        .spawn(move || {
            if let Err(error) = notify_rust::Notification::new()
                .summary(&title)
                .body(&body)
                .show()
            {
                tracing::warn!(%error, "desktop notification failed");
            }
        })
        .map(|_| ())
        .map_err(|e| termherd_core::ports::PtyError::Io(e.to_string()))
}
