//! macOS AppKit glue — the single audited `unsafe` module in the workspace.
//!
//! Two repairs to what winit 0.30 does on macOS, both *mechanism only*: the
//! policy each one feeds stays in the safe, headless-tested shell.
//!
//! **Cmd+Q.** winit installs a default application menu whose **Quit** item
//! invokes AppKit's `terminate:` (⌘Q). `terminate:` ends the process *before*
//! iced's runtime can confirm the quit or shut down cleanly, so Cmd+Q
//! hard-kills every live Claude session with no warning. We repoint that one
//! item's action to `performClose:`, which routes through winit's
//! `windowShouldClose:` and reaches the shell as a `CloseRequested` event — the
//! very seam the window-close button already uses (see
//! `shell::Shell::request_quit`).
//!
//! **Text inserted outside a keystroke.** winit's `insertText:replacementRange:`
//! only reports text while a composition (marked text) is in progress, and
//! leaves everything else to the `keyDown:` assumed to have caused it. The
//! Character Viewer (Ctrl+Cmd+Space) inserts with no composition and no key
//! press, so its emoji were dropped before reaching iced: a focused
//! `text_input` never saw them. [`route_stray_text_through_ime`] replays such
//! an insertion as a one-shot composition through winit's own
//! `NSTextInputClient` methods, so it surfaces as the ordinary `Ime::Commit`
//! every input widget already handles. Upstream winit (master, checked
//! 2026-10) still gates the commit on a prior composition, so a dependency
//! bump does not retire this yet.
//!
//! It is the lone exception to the workspace-wide `unsafe_code = "deny"` —
//! every `unsafe` below is an ObjC message send or runtime call on the main
//! thread, the standard `objc2` idiom.
#![allow(unsafe_code)]

use std::ffi::c_char;

use objc2::ffi;
use objc2::runtime::{AnyClass, AnyObject, Imp, Method, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::{NSApplication, NSEventType};
use objc2_foundation::{MainThreadMarker, NSRange, NSString, NSUInteger};

/// Repoint the app-menu **Quit** item from `terminate:` to `performClose:` so
/// quitting flows through the iced runtime instead of AppKit terminating the
/// process out from under it. Fire-once at startup, on the main thread.
///
/// Best-effort: a missing menu or item only means Cmd+Q keeps its old AppKit
/// behaviour, so we log and return rather than ever blocking launch.
pub fn route_quit_through_close(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);

    let terminate = sel!(terminate:);
    let perform_close = sel!(performClose:);

    // SAFETY: ordinary AppKit reads/writes on the main thread (guaranteed by
    // `mtm`). Every call returns an owned `Retained`/`Option`/`Copy` value; no
    // raw pointers escape and no aliasing or lifetime contract is owed beyond
    // what `objc2`'s own types already enforce.
    unsafe {
        let Some(menubar) = app.mainMenu() else {
            tracing::warn!("no main menu; Cmd+Q stays on AppKit terminate:");
            return;
        };
        // The Quit item lives in a submenu of the menu bar (the application
        // menu). Scan every submenu and match on the action, not a title or a
        // fixed index, so a winit menu-layout change can't silently miss it.
        for top in menubar.itemArray().iter() {
            let Some(submenu) = top.submenu() else {
                continue;
            };
            for item in submenu.itemArray().iter() {
                match item.action() {
                    Some(action) if action == terminate => {
                        item.setAction(Some(perform_close));
                        // Target the window explicitly, not nil. A nil target
                        // routes `performClose:` down the responder chain from
                        // the *key* window — but with the sole window minimized
                        // there is no key or main window, so NSMenu
                        // auto-enabling (`autoenablesItems`, on by default) would
                        // disable Quit and Cmd+Q would just beep. Pinning the
                        // window keeps Quit enabled and dispatching in every
                        // window state; `performClose:` still reaches winit's
                        // `windowShouldClose:` → `CloseRequested`.
                        match app
                            .keyWindow()
                            .or_else(|| app.mainWindow())
                            .or_else(|| app.windows().firstObject())
                        {
                            Some(window) => item.setTarget(Some(&window)),
                            None => {
                                item.setTarget(None);
                                tracing::warn!(
                                    "no app window to target; Quit uses the responder chain"
                                );
                            }
                        }
                        tracing::info!("repointed Quit menu item to performClose:");
                        return;
                    }
                    // A previous `Opened` already repointed it. Return quietly —
                    // emitting the "not found" warning below would be a false
                    // alarm implying Cmd+Q is unprotected when it is fine.
                    Some(action) if action == perform_close => return,
                    _ => {}
                }
            }
        }
        tracing::warn!("Quit menu item not found; Cmd+Q stays on terminate:");
    }
}

/// The class winit declares for the window's content view — the
/// `NSTextInputClient` the system's input methods talk to.
const WINIT_VIEW: &str = "WinitView";

/// `NSNotFound`, the "no replacement range" marker of `NSTextInputClient`.
const NOT_FOUND: NSUInteger = isize::MAX.unsigned_abs();

/// The signature of `insertText:replacementRange:`, encoded `v@:@{_NSRange=QQ}`.
type InsertText = unsafe extern "C" fn(&AnyObject, Sel, &AnyObject, NSRange);

/// Whether an `insertText:` winit is about to drop should be replayed as a
/// composition. winit reports it on its own while marked text is pending, and
/// during a `keyDown:` the key event that follows carries the same text, so
/// replaying either would type it twice.
fn replays_as_composition(has_marked_text: bool, during_key_down: bool) -> bool {
    !has_marked_text && !during_key_down
}

/// Wrap winit's `insertText:replacementRange:` so text inserted outside a
/// keystroke — the Character Viewer, and anything else that inserts without
/// composing — reaches iced as an `Ime::Commit` instead of being dropped.
/// Fire-once at startup, on the main thread, once the window (and with it the
/// `WinitView` class) exists; a repeat call is a no-op.
///
/// Best-effort like the Quit reroute: if winit's view class or method is not
/// where 0.30 puts it, log and leave input exactly as winit delivers it.
pub fn route_stray_text_through_ime(_mtm: MainThreadMarker) {
    let Some(view) = AnyClass::get(WINIT_VIEW) else {
        tracing::warn!("no {WINIT_VIEW} class; Character Viewer text stays dropped");
        return;
    };
    let relay = sel!(termherdWinitInsertText:replacementRange:);
    if view.instance_method(relay).is_some() {
        return;
    }
    let Some(method) = view.instance_method(sel!(insertText:replacementRange:)) else {
        tracing::warn!("{WINIT_VIEW} has no insertText:; Character Viewer text stays dropped");
        return;
    };
    let hook: InsertText = insert_text;
    // SAFETY: `Imp` is the type-erased `unsafe extern "C" fn()` the runtime
    // stores every method as; it is only ever called back through the type
    // encoding recorded beside it, which is the original method's own and the
    // one `hook` is written against. `Method` and `AnyClass` are `repr(C)`
    // wrappers of the runtime's own structs, obtained from live lookups.
    // Adding a method and then exchanging two implementations of one class is
    // the documented swizzle: the relay selector is new, so the add clobbers
    // nothing, and the early return above makes it happen once.
    unsafe {
        let imp = std::mem::transmute::<InsertText, Imp>(hook);
        let types: *const c_char =
            ffi::method_getTypeEncoding(std::ptr::from_ref::<Method>(method).cast());
        let class: *mut ffi::objc_class = std::ptr::from_ref::<AnyClass>(view).cast_mut().cast();
        if ffi::class_addMethod(class, relay.as_ptr(), Some(imp), types) == ffi::NO {
            tracing::warn!(
                "could not add the insertText: relay; Character Viewer text stays dropped"
            );
            return;
        }
        let Some(added) = view.instance_method(relay) else {
            return;
        };
        method.exchange_implementation(added);
    }
    tracing::info!("routed out-of-keystroke insertText: through the input method");
}

/// What runs as `WinitView`'s `insertText:replacementRange:` once
/// [`route_stray_text_through_ime`] has swapped it in; winit's own method now
/// answers to the relay selector.
///
/// For an insertion winit would drop, open a composition holding exactly the
/// inserted text, let winit's `insertText:` commit it — winit commits only over
/// marked text — then close the composition, so the next key press starts from
/// the ground state rather than from a stale marked range.
unsafe extern "C" fn insert_text(this: &AnyObject, _cmd: Sel, string: &AnyObject, range: NSRange) {
    let no_range = NSRange::new(NOT_FOUND, 0);
    // SAFETY: `this` is a live `WinitView` (the runtime dispatched to it), and
    // `string` is the `NSString` / `NSAttributedString` the protocol
    // guarantees; both answer `length`. Every selector sent to `this` is one
    // `WinitView` implements for `NSTextInputClient`, with the argument types
    // its own declaration encodes. AppKit calls a view's text-input methods on
    // the main thread only.
    unsafe {
        let has_marked_text: bool = msg_send![this, hasMarkedText];
        let replay = replays_as_composition(has_marked_text, during_key_down());
        if replay {
            let length: NSUInteger = msg_send![string, length];
            let selected = NSRange::new(length, 0);
            let _: () = msg_send![this, setMarkedText: string, selectedRange: selected, replacementRange: no_range];
        }
        let _: () = msg_send![this, termherdWinitInsertText: string, replacementRange: range];
        if replay {
            let empty = NSString::new();
            let _: () = msg_send![this, setMarkedText: &*empty, selectedRange: NSRange::new(0, 0), replacementRange: no_range];
        }
    }
}

/// Whether AppKit is dispatching a key press right now — the case where
/// winit's `keyDown:` reports the inserted text itself. A Character Viewer
/// insertion comes from another process with no key event of ours behind it.
fn during_key_down() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    NSApplication::sharedApplication(mtm)
        .currentEvent()
        // SAFETY: reading an `NSEvent`'s type has no precondition.
        .is_some_and(|event| unsafe { event.r#type() } == NSEventType::KeyDown)
}

#[cfg(test)]
mod tests {
    use super::replays_as_composition;

    #[test]
    fn text_inserted_with_no_composition_and_no_key_press_is_replayed() {
        assert!(replays_as_composition(false, false));
    }

    #[test]
    fn a_pending_composition_is_left_to_winit() {
        assert!(!replays_as_composition(true, false));
        assert!(!replays_as_composition(true, true));
    }

    #[test]
    fn text_a_key_press_inserted_is_left_to_the_key_event() {
        assert!(!replays_as_composition(false, true));
    }
}
