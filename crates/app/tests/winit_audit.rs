//! `src/macos.rs` relays Character Viewer text through winit's own
//! `NSTextInputClient` methods, which leans on internals no API promises: the
//! private `WinitView` class name, and how its `setMarkedText:` and
//! `insertText:` move its IME state. That was audited against one winit
//! release. A bump must be re-audited — the relay could start doubling text, or
//! stop firing — so a lockfile that resolves any other winit fails here, on
//! every OS, rather than on a Mac weeks later.

/// The winit release whose `platform_impl/macos/view.rs` the relay was read
/// against.
const AUDITED: &str = "0.30.13";

#[test]
fn the_lockfile_resolves_the_winit_the_macos_text_relay_was_audited_against() {
    let lock = include_str!("../../../Cargo.lock");
    let versions: Vec<&str> = lock
        .split("[[package]]")
        .filter(|package| package.contains("\nname = \"winit\"\n"))
        .filter_map(|package| {
            package
                .lines()
                .find_map(|line| line.strip_prefix("version = \""))
                .and_then(|rest| rest.strip_suffix('"'))
        })
        .collect();
    assert_eq!(
        versions,
        vec![AUDITED],
        "winit moved: re-read its macOS `insertText:`, `setMarkedText:` and \
         `keyDown:` against `route_stray_text_through_ime` in src/macos.rs, \
         then update AUDITED"
    );
}
