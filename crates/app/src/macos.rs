//! macOS AppKit glue — the single audited `unsafe` module in the workspace.
//!
//! winit installs a default application menu whose **Quit** item invokes
//! AppKit's `terminate:` (⌘Q). `terminate:` ends the process *before* iced's
//! runtime can confirm the quit or shut down cleanly, so Cmd+Q hard-kills every
//! live Claude session with no warning. We repoint that one item's
//! action to `performClose:`, which routes through winit's `windowShouldClose:`
//! and reaches the shell as a `CloseRequested` event — the very seam the
//! window-close button already uses (see `shell::Shell::request_quit`).
//!
//! This module carries *mechanism only*: the confirm/exit *policy* stays in the
//! safe, headless-tested shell. It is the lone exception to the workspace-wide
//! `unsafe_code = "deny"` — every `unsafe` below is a plain ObjC message send to
//! AppKit menu objects on the main thread, the standard `objc2` idiom.
#![allow(unsafe_code)]

use objc2::sel;
use objc2_app_kit::NSApplication;
use objc2_foundation::MainThreadMarker;

/// Repoint the app-menu **Quit** item from `terminate:` to `performClose:` so
/// quitting flows through the iced runtime instead of AppKit terminating the
/// process out from under it. Fire-once at startup, on the main thread.
///
/// Best-effort: a missing menu or item only means Cmd+Q keeps its old AppKit
/// behaviour, so we log and return rather than ever blocking launch.
pub fn route_quit_through_close(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);

    // SAFETY: ordinary AppKit reads/writes on the main thread (guaranteed by
    // `mtm`). Every call returns an owned `Retained`/`Option`/`Copy` value; no
    // raw pointers escape and no aliasing or lifetime contract is owed beyond
    // what `objc2`'s own types already enforce.
    unsafe {
        let Some(menubar) = app.mainMenu() else {
            tracing::warn!("no main menu; Cmd+Q stays on AppKit terminate:");
            return;
        };
        let terminate = sel!(terminate:);
        // The Quit item lives in a submenu of the menu bar (the application
        // menu). Scan every submenu and match on the action, not a title or a
        // fixed index, so a winit menu-layout change can't silently miss it.
        for top in menubar.itemArray().iter() {
            let Some(submenu) = top.submenu() else {
                continue;
            };
            for item in submenu.itemArray().iter() {
                if item.action() == Some(terminate) {
                    item.setAction(Some(sel!(performClose:)));
                    // nil target → AppKit sends `performClose:` down the
                    // responder chain to the key window (winit's window), which
                    // answers with `windowShouldClose:` → `CloseRequested`.
                    item.setTarget(None);
                    tracing::info!("repointed Quit menu item to performClose:");
                    return;
                }
            }
        }
        tracing::warn!("Quit menu item not found; Cmd+Q stays on terminate:");
    }
}
