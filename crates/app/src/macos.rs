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
//! The relay leans on winit internals audited against 0.30.13 only; see
//! `tests/winit_audit.rs`.
//!
//! It is the lone exception to the workspace-wide `unsafe_code = "deny"` —
//! every `unsafe` below is an ObjC message send or runtime call on the main
//! thread, the standard `objc2` idiom.
#![allow(unsafe_code)]

use std::ffi::{c_char, c_void};

use objc2::ffi;
use objc2::runtime::{AnyClass, AnyObject, Imp, Method, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSNotFound, NSRange, NSString, NSUInteger};

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

/// The associated-object key that marks a `WinitView` as inside its own
/// `keyDown:`. Only its address matters; the flag itself lives on the view.
static IN_KEY_DOWN: u8 = 0;

/// `keyDown:`, encoded `v@:@`.
type KeyDown = unsafe extern "C" fn(&AnyObject, Sel, Option<&AnyObject>);

/// `insertText:replacementRange:`, encoded `v@:@{_NSRange=QQ}`.
type InsertText = unsafe extern "C" fn(&AnyObject, Sel, Option<&AnyObject>, NSRange);

/// What an `insertText:` arrives with, as far as replaying it is concerned.
#[derive(Clone, Copy)]
struct Insertion {
    /// A composition is pending, and winit commits over it on its own.
    has_marked_text: bool,
    /// winit's `keyDown:` is running, and its key event carries the text.
    in_key_down: bool,
    /// The text replaces a range (the press-and-hold accent popup does), which
    /// winit's commit cannot express: a replay would append instead.
    replaces_range: bool,
}

/// Whether an `insertText:` winit is about to drop should be replayed as a
/// composition. Replaying any other kind would type the text twice, or turn
/// a replacement into an append.
fn replays_as_composition(insertion: Insertion) -> bool {
    !insertion.has_marked_text && !insertion.in_key_down && !insertion.replaces_range
}

/// Wrap winit's `insertText:replacementRange:` so text inserted outside a
/// keystroke — the Character Viewer, and anything else that inserts without
/// composing — reaches iced as an `Ime::Commit` instead of being dropped.
/// `keyDown:` is wrapped too, only to mark the view while it runs: text a key
/// press inserts reaches iced through the key event and must not be replayed.
/// Fire-once at startup, on the main thread, once the window (and with it the
/// `WinitView` class) exists; a repeat call is a no-op.
///
/// Best-effort like the Quit reroute: if winit's view class or methods are not
/// where 0.30 puts them, log and leave input exactly as winit delivers it.
pub fn route_stray_text_through_ime(_mtm: MainThreadMarker) {
    let Some(view) = AnyClass::get(WINIT_VIEW) else {
        tracing::warn!("no {WINIT_VIEW} class; Character Viewer text stays dropped");
        return;
    };
    let key_down: KeyDown = key_down;
    let insert_text: InsertText = insert_text;
    // SAFETY: each hook is installed over the method whose encoding its type
    // alias spells out, so the runtime calls it back with that signature.
    let installed = unsafe {
        swizzle(
            view,
            sel!(keyDown:),
            sel!(termherdWinitKeyDown:),
            std::mem::transmute::<KeyDown, Imp>(key_down),
        ) && swizzle(
            view,
            sel!(insertText:replacementRange:),
            sel!(termherdWinitInsertText:replacementRange:),
            std::mem::transmute::<InsertText, Imp>(insert_text),
        )
    };
    if installed {
        tracing::info!("routed out-of-keystroke insertText: through the input method");
    }
}

/// Install `imp` as `class`'s `original` method, keeping the implementation it
/// replaces callable as `relay`. True once installed, by this call or an
/// earlier one — a present relay means the swap already happened.
///
/// # Safety
///
/// `imp` must be an `extern "C"` function with exactly the signature that
/// `original`'s type encoding describes, and `class` a registered class.
unsafe fn swizzle(class: &AnyClass, original: Sel, relay: Sel, imp: Imp) -> bool {
    if class.instance_method(relay).is_some() {
        return true;
    }
    let Some(method) = class.instance_method(original) else {
        tracing::warn!(
            "{WINIT_VIEW} has no {}; Character Viewer text stays dropped",
            original.name()
        );
        return false;
    };
    // SAFETY: `Method` and `AnyClass` are `repr(C)` wrappers of the runtime's
    // own structs, from live lookups. The relay is added with the original's
    // own type encoding (the caller vouches `imp` matches it), and adding a
    // method then exchanging two implementations of one class is the
    // documented swizzle; the relay selector is new, so the add clobbers
    // nothing.
    unsafe {
        let types: *const c_char =
            ffi::method_getTypeEncoding(std::ptr::from_ref::<Method>(method).cast());
        let raw: *mut ffi::objc_class = std::ptr::from_ref::<AnyClass>(class).cast_mut().cast();
        if ffi::class_addMethod(raw, relay.as_ptr(), Some(imp), types) == ffi::NO {
            tracing::warn!(
                "could not add the {} relay; Character Viewer text stays dropped",
                original.name()
            );
            return false;
        }
        let Some(added) = class.instance_method(relay) else {
            return false;
        };
        method.exchange_implementation(added);
    }
    true
}

/// The view as the runtime's associated-object functions take it.
fn as_object(view: &AnyObject) -> *mut ffi::objc_object {
    std::ptr::from_ref(view).cast_mut().cast()
}

fn in_key_down_key() -> *const c_void {
    std::ptr::from_ref(&IN_KEY_DOWN).cast()
}

/// What runs as `WinitView`'s `keyDown:`: winit's own, with the view marked
/// for its duration. The mark is an assigned (unretained) pointer to the view
/// itself, restored afterwards so a nested `keyDown:` cannot clear an outer
/// one's.
unsafe extern "C" fn key_down(this: &AnyObject, _cmd: Sel, event: Option<&AnyObject>) {
    let object = as_object(this);
    // SAFETY: `this` is a live `WinitView` the runtime dispatched to; the
    // associated-object calls take any object and a stable key address, and
    // `OBJC_ASSOCIATION_ASSIGN` neither retains nor releases. The relay is
    // winit's `keyDown:`, sent with the event we received.
    unsafe {
        let outer = ffi::objc_getAssociatedObject(object, in_key_down_key());
        ffi::objc_setAssociatedObject(
            object,
            in_key_down_key(),
            object,
            ffi::OBJC_ASSOCIATION_ASSIGN,
        );
        let _: () = msg_send![this, termherdWinitKeyDown: event];
        ffi::objc_setAssociatedObject(
            object,
            in_key_down_key(),
            outer.cast_mut(),
            ffi::OBJC_ASSOCIATION_ASSIGN,
        );
    }
}

/// Whether `view` is inside its own `keyDown:` right now.
fn in_key_down(view: &AnyObject) -> bool {
    // SAFETY: reading an associated object has no precondition beyond a live
    // object, which `view` is.
    unsafe { !ffi::objc_getAssociatedObject(as_object(view), in_key_down_key()).is_null() }
}

/// What runs as `WinitView`'s `insertText:replacementRange:` once
/// [`route_stray_text_through_ime`] has swapped it in; winit's own method now
/// answers to the relay selector.
///
/// For an insertion winit would drop, open a composition holding exactly the
/// inserted text, let winit's `insertText:` commit it — winit commits only over
/// marked text — then close the composition, so the next key press starts from
/// the ground state rather than from a stale marked range.
unsafe extern "C" fn insert_text(
    this: &AnyObject,
    _cmd: Sel,
    string: Option<&AnyObject>,
    range: NSRange,
) {
    // winit's own method takes the text as non-null, so handing it a nil would
    // be undefined behaviour; there is nothing to insert anyway.
    let Some(string) = string else {
        return;
    };
    let not_found = NSNotFound.unsigned_abs();
    let no_range = NSRange::new(not_found, 0);
    // SAFETY: `this` is a live `WinitView` (the runtime dispatched to it), and
    // `string` is the `NSString` / `NSAttributedString` the protocol
    // guarantees; both answer `length`. Every selector sent to `this` is one
    // `WinitView` implements for `NSTextInputClient`, with the argument types
    // its own declaration encodes. AppKit calls a view's text-input methods on
    // the main thread only.
    unsafe {
        let has_marked_text: bool = msg_send![this, hasMarkedText];
        let replay = replays_as_composition(Insertion {
            has_marked_text,
            in_key_down: in_key_down(this),
            replaces_range: range.location != not_found,
        });
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

#[cfg(test)]
mod tests {
    use super::{Insertion, replays_as_composition};

    /// A Character Viewer pick: no composition, no key press, no range.
    const PICKED: Insertion = Insertion {
        has_marked_text: false,
        in_key_down: false,
        replaces_range: false,
    };

    #[test]
    fn text_inserted_with_no_composition_and_no_key_press_is_replayed() {
        assert!(replays_as_composition(PICKED));
    }

    #[test]
    fn a_pending_composition_is_left_to_winit() {
        let composing = Insertion {
            has_marked_text: true,
            ..PICKED
        };
        assert!(!replays_as_composition(composing));
    }

    #[test]
    fn text_a_key_press_inserted_is_left_to_the_key_event() {
        let typed = Insertion {
            in_key_down: true,
            ..PICKED
        };
        assert!(!replays_as_composition(typed));
    }

    #[test]
    fn a_replacement_is_left_to_winit_rather_than_appended() {
        let accent_popup = Insertion {
            replaces_range: true,
            ..PICKED
        };
        assert!(!replays_as_composition(accent_popup));
    }
}
