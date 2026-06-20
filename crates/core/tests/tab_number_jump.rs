//! Integration: the number-row chord drives a tab jump end to end (issue #26).
//!
//! This stitches the two pure halves the GUI sits between — the [`Keymap`]
//! (chord → [`Action`]) and the [`App`] state machine ([`Event`] → active tab)
//! — without any GUI/PTY. It is the closest headless proxy for "press ⌘3, land
//! on the third tab".

use termherd_core::keymap::primary_mod;
use termherd_core::{Action, App, Effect, Event, KeyChord, Keymap, LaunchSpec};

/// Open a session as a new (active) tab.
fn launch(app: &mut App, title: &str) {
    let effects = app.apply(Event::LaunchSession(LaunchSpec {
        cwd: None,
        resume: None,
        title: title.into(),
    }));
    assert!(
        matches!(effects.as_slice(), [Effect::Spawn(_)]),
        "launching a session should request a spawn",
    );
}

/// Resolve a number-row chord, then apply whatever tab jump it names.
fn press_number(app: &mut App, keymap: &Keymap, digit: u8) -> Option<Action> {
    let action = keymap.lookup(&KeyChord::new(digit.to_string(), primary_mod()));
    if let Some(Action::ActivateTab(index)) = action {
        app.apply(Event::ActivateTab(index));
    }
    action
}

#[test]
fn primary_three_jumps_to_the_third_tab() {
    let keymap = Keymap::defaults();
    let mut app = App::new();
    for title in ["a", "b", "c", "d"] {
        launch(&mut app, title);
    }
    assert_eq!(app.workspace.active, 3, "the last opened tab starts active");

    let action = press_number(&mut app, &keymap, 3);

    assert_eq!(action, Some(Action::ActivateTab(2)));
    assert_eq!(
        app.workspace.active, 2,
        "⌘3 / Ctrl+3 lands on the third tab"
    );
}

#[test]
fn primary_one_through_nine_each_select_their_tab() {
    let keymap = Keymap::defaults();
    let mut app = App::new();
    for title in ["t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8", "t9"] {
        launch(&mut app, title);
    }

    for digit in 1u8..=9 {
        let action = press_number(&mut app, &keymap, digit);
        assert_eq!(action, Some(Action::ActivateTab(usize::from(digit) - 1)));
        assert_eq!(app.workspace.active, usize::from(digit) - 1);
    }
}

#[test]
fn primary_number_beyond_open_tabs_keeps_the_active_tab() {
    let keymap = Keymap::defaults();
    let mut app = App::new();
    for title in ["a", "b"] {
        launch(&mut app, title);
    }
    assert_eq!(app.workspace.active, 1);

    // The chord must still resolve to a jump; the App is what makes the
    // out-of-range index a no-op (⌘5 with two tabs open).
    let action = press_number(&mut app, &keymap, 5);

    assert_eq!(action, Some(Action::ActivateTab(4)));
    assert_eq!(app.workspace.active, 1, "no fifth tab to jump to");
}
