//! OS handoffs the effect executor performs: opening a detected link in the
//! default handler ([`open_url`]) and posting a desktop notification
//! ([`notify`]). Both are fire-and-forget, never fatal. This module plus
//! `crate::macos` are the only homes for `cfg(target_os)` in the app crate — OS
//! divergence is quarantined here rather than scattered through the shell.

use termherd_core::ports::PtyError;

use super::super::notify_click::ClickSlot;

/// macOS bundle identifier (matches `Cargo.toml`'s packager `identifier`).
/// Used to attribute desktop notifications to TermHerd; see [`notify`].
#[cfg(target_os = "macos")]
const MACOS_BUNDLE_ID: &str = "dev.termherd";

/// Hand a resolved file to the OS default handler — the same three openers a
/// URL takes, since each accepts a path as readily as a URL. The `:line` the
/// terminal printed cannot survive this handoff: no OS opener takes a
/// position, which is the whole reason the configured command exists.
pub(super) fn open_path(path: &std::path::Path) -> Result<(), PtyError> {
    open_url(&path.to_string_lossy())
}

/// Run the user's configured editor command. `core` already substituted the
/// argv, so this spawns it as-is: a real executable with its arguments, never
/// a line a shell re-parses — the same rule as [`open_url`], and here it is
/// load-bearing twice over, since one of those arguments is a path the
/// terminal printed.
///
/// Fire-and-forget like every other handoff: the error this returns is the
/// spawn's alone (no such program, no permission), never the editor's exit.
///
/// **The three streams are closed, not inherited.** A GUI editor ignores them,
/// but a terminal one (`vim +{line} {path}`, which the settings template shows
/// as a *grammar*, not as a suggestion) would otherwise attach to the app's own
/// stdio: an invisible editor holding a window-less process's terminal, able to
/// block on a full pipe, killable only from outside.
///
/// **The child is not reaped.** Like its `open_url` / `notify` siblings the
/// handle is dropped, so on Unix each spawn leaves an entry until termherd
/// exits. It matters more here than there: `open` and `xdg-open` return in
/// milliseconds, where an editor this *launches* can outlive the click by
/// hours. Waiting would cost a parked thread per open — worse than the entry it
/// removes — so the entry stands, named rather than discovered later.
pub(super) fn spawn_editor(program: &str, args: &[String]) -> Result<(), PtyError> {
    use std::process::Stdio;
    std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| PtyError::Io(e.to_string()))
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

/// Label of the XDG `"default"` action. Most servers never draw it — it is the
/// key a click on the body reports — but a few render it as a button.
#[cfg(all(unix, not(target_os = "macos")))]
const OPEN_ACTION_LABEL: &str = "Show";

/// Post a desktop notification to the OS notification centre. Like
/// `open_url`, this is an OS handoff, not a PTY call, and fire-and-forget: the
/// send runs on a detached thread and the result is logged there, never fatal —
/// a notification backend that's unavailable must not take a session down.
/// `title`/`body` come pre-derived from `core` (which session, what message).
pub(super) fn notify(title: &str, body: &str) -> Result<(), PtyError> {
    spawn(title, body, |title, body| {
        notify_rust::Notification::new()
            .summary(&title)
            .body(&body)
            .show()
            .map(drop)
    })
}

/// [`notify`], then wait on the same thread for the user's answer and hand a
/// click to `slot`. A dismissal or an expiry clicks nothing; the slot drops
/// with the thread either way.
pub(super) fn notify_clickable(title: &str, body: &str, slot: ClickSlot) -> Result<(), PtyError> {
    spawn(title, body, move |title, body| {
        show_and_wait(&title, &body, slot)
    })
}

/// Replace the notification the OS knows as `id` — one whose waiter is still
/// parked, and carries this one's click too. Only XDG ever hands out such an
/// id (see [`ClickSlot::shown`]).
pub(super) fn notify_replacing(title: &str, body: &str, id: u32) -> Result<(), PtyError> {
    spawn(title, body, move |title, body| {
        clickable(&title, &body).id(id).show().map(drop)
    })
}

/// **Why a thread, not a direct call:** on macOS the backend (`NSUserNotification`
/// via `mac-notification-sys`) drives an `NSRunLoop` to await delivery *when
/// invoked on the main thread*. iced calls `perform` from inside winit's event
/// handler, so pumping the run loop there re-enters it and aborts the process.
/// Off the main thread the backend takes a Condvar wait instead, so this is
/// both crash-safe and non-blocking for the UI — and the same off-main wait is
/// what lets a clickable notification block until it is answered, with winit's
/// own run loop delivering the click.
fn spawn(
    title: &str,
    body: &str,
    job: impl FnOnce(String, String) -> notify_rust::error::Result<()> + Send + 'static,
) -> Result<(), PtyError> {
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
            if let Err(error) = job(title, body) {
                tracing::warn!(%error, "desktop notification failed");
            }
        })
        .map(|_| ())
        .map_err(|e| PtyError::Io(e.to_string()))
}

/// A notification whose body click is reported. Only XDG needs the action
/// declared for that; macOS and Windows report the click without it, and both
/// would draw a declared action as a visible button.
fn clickable(title: &str, body: &str) -> notify_rust::Notification {
    let mut notification = notify_rust::Notification::new();
    notification.summary(title).body(body);
    #[cfg(all(unix, not(target_os = "macos")))]
    notification.action("default", OPEN_ACTION_LABEL);
    notification
}

/// Whether the user's answer is a click on the notification itself — the one
/// answer that brings its session back. A button, a reply or a close is not.
fn is_click(response: &notify_rust::NotificationResponse) -> bool {
    response.is_default_action()
}

#[cfg(not(target_os = "macos"))]
fn show_and_wait(title: &str, body: &str, slot: ClickSlot) -> notify_rust::error::Result<()> {
    let handle = clickable(title, body).show()?;
    #[cfg(all(unix, not(target_os = "macos")))]
    slot.shown(handle.id());
    handle.wait_for_response(|response: &notify_rust::NotificationResponse| {
        if is_click(response) {
            slot.clicked();
        }
    })
}

/// macOS goes to `mac-notification-sys` directly: notify-rust 4.18's
/// `wait_for_response` sends with neither a button nor `wait_for_click`, which
/// the backend reads as nothing to wait for, so it answered "expired" at once
/// and a click never arrived.
#[cfg(target_os = "macos")]
fn show_and_wait(title: &str, body: &str, slot: ClickSlot) -> notify_rust::error::Result<()> {
    let mut notification = mac_notification_sys::Notification::new();
    notification.title(title).message(body).wait_for_click(true);
    if is_click(&from_mac(notification.send()?)) {
        slot.clicked();
    }
    Ok(())
}

/// The macOS backend's answer in notify-rust's terms, mapped as notify-rust
/// maps it itself, so [`is_click`] is the one decision on every OS.
#[cfg(target_os = "macos")]
fn from_mac(
    response: mac_notification_sys::NotificationResponse,
) -> notify_rust::NotificationResponse {
    use mac_notification_sys::NotificationResponse as Mac;
    use notify_rust::{CloseReason, NotificationResponse as Answer};
    match response {
        Mac::Click => Answer::Default,
        Mac::ActionButton(label) => Answer::Action(label),
        Mac::Reply(text) => Answer::Reply(text),
        Mac::CloseButton(_) => Answer::Closed(CloseReason::Dismissed),
        Mac::None => Answer::Closed(CloseReason::Expired),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify_rust::{CloseReason, NotificationResponse};

    #[test]
    fn only_a_click_on_the_notification_counts_as_a_click() {
        assert!(is_click(&NotificationResponse::Default));
        for other in [
            NotificationResponse::Action("show".to_owned()),
            NotificationResponse::Reply("ok".to_owned()),
            NotificationResponse::Closed(CloseReason::Dismissed),
            NotificationResponse::Closed(CloseReason::Expired),
        ] {
            assert!(!is_click(&other), "{other:?} is not a click");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_answer_reaches_the_shared_click_decision() {
        use mac_notification_sys::NotificationResponse as Mac;
        assert!(is_click(&from_mac(Mac::Click)));
        for other in [
            Mac::None,
            Mac::CloseButton("Close".to_owned()),
            Mac::ActionButton("Show".to_owned()),
            Mac::Reply("ok".to_owned()),
        ] {
            assert!(!is_click(&from_mac(other)));
        }
    }

    /// A program every host has, told to copy `from` to `to` — so the test can
    /// prove the *arguments* arrived, not merely that something started. The
    /// one place a per-OS branch is allowed, and the reason this test lives
    /// here rather than beside the pure argv building in `core`.
    fn copy_command(from: &str, to: &str) -> (&'static str, Vec<String>) {
        #[cfg(windows)]
        {
            (
                "cmd",
                vec![
                    "/C".to_owned(),
                    "copy".to_owned(),
                    from.to_owned(),
                    to.to_owned(),
                ],
            )
        }
        #[cfg(not(windows))]
        {
            ("/bin/cp", vec![from.to_owned(), to.to_owned()])
        }
    }

    #[test]
    fn the_argv_reaches_the_program_that_was_named() {
        // The failure test below cannot tell "spawned the wrong thing" from
        // "spawned the right thing": only an observable side effect can, and a
        // copy is the smallest one every host can perform.
        let dir = std::env::temp_dir().join(format!("termherd-spawn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let (from, to) = (dir.join("clicked.txt"), dir.join("opened.txt"));
        std::fs::write(&from, b"opened").expect("write source");

        let (program, args) = copy_command(&from.to_string_lossy(), &to.to_string_lossy());
        spawn_editor(program, &args).expect("the copy must start");

        // Fire-and-forget means the child is not waited on, so poll for its
        // effect rather than assert it has already landed.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !to.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::read(&to).ok().as_deref(),
            Some(b"opened".as_slice()),
            "the arguments reached the program, in order"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_editor_reports_the_failure_rather_than_swallowing_it() {
        // The caller turns this error into a desktop notification: a typo in
        // `settings.json` must reach the only person who can fix it, since a
        // click that opens nothing looks exactly like a click that missed.
        let outcome = spawn_editor("termherd-no-such-editor-b7c1", &["{path}".to_owned()]);
        assert!(matches!(outcome, Err(PtyError::Io(_))));
    }
}
