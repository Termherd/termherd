//! OS handoffs the effect executor performs: opening a detected link in the
//! default handler ([`open_url`]) and posting a desktop notification
//! ([`notify`]). Both are fire-and-forget, never fatal. This module plus
//! `crate::macos` are the only homes for `cfg(target_os)` in the app crate — OS
//! divergence is quarantined here rather than scattered through the shell.

use termherd_core::ports::PtyError;

/// macOS bundle identifier (matches `Cargo.toml`'s packager `identifier`).
/// Used to attribute desktop notifications to TermHerd; see [`notify`].
#[cfg(target_os = "macos")]
const MACOS_BUNDLE_ID: &str = "dev.termherd";

/// Hand a resolved file to the OS default handler — the same three openers a
/// URL takes, since each accepts a path as readily as a URL. The `:line` the
/// terminal printed cannot survive this handoff: no OS opener takes a
/// position. Carrying it this far is what lets the configurable editor command
/// honour it later without re-deriving anything.
pub(super) fn open_path(path: &std::path::Path) -> Result<(), PtyError> {
    open_url(&path.to_string_lossy())
}

/// Hand a detected link to the OS default handler. Fire-and-forget: the
/// child opener is spawned, not waited on. The target is always passed as a
/// single argument to a real executable — never as text a shell re-parses.
///
/// That last part is the whole reason Windows does **not** go through
/// `cmd /C start`. `cmd` re-parses its command line and treats `&`, `|`, `^`
/// and `%VAR%` as syntax, while Rust quotes an argument only when it contains
/// whitespace. A file legitimately named `Q&A.md` would split in half, and a
/// filename an attacker chose — a path in this app now comes from whatever the
/// terminal printed — would run its second half as a command. `explorer.exe` is
/// an ordinary executable, so the argument reaches it intact: the string is
/// never rendered into `cmd`'s grammar in the first place.
pub(super) fn open_url(url: &str) -> Result<(), PtyError> {
    use std::process::Command;
    let spawn = |mut cmd: Command| {
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| PtyError::Io(e.to_string()))
    };
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        spawn(cmd)
    }
    #[cfg(target_os = "windows")]
    {
        // `explorer` hands both a path and a URL to the registered handler,
        // exactly as `start` did, without a shell in between. Its exit status
        // is famously unreliable, which costs nothing here: the spawn is
        // fire-and-forget and never waited on.
        let mut cmd = Command::new("explorer");
        cmd.arg(url);
        spawn(cmd)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        spawn(cmd)
    }
}

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
pub(super) fn notify(title: &str, body: &str) -> Result<(), PtyError> {
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
        .map_err(|e| PtyError::Io(e.to_string()))
}
