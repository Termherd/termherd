//! The Windows executable asks for no console of its own.
//!
//! Launched from Explorer or the Start menu, a console app gets a console
//! window beside it. The manifest's `consoleAllocationPolicy` of `detached`
//! (Windows 11 24H2 and later) withholds it, while a terminal still lends the
//! process its console, so `termherd mcp-config > file` keeps working in every
//! shell. `build.rs` embeds the manifest; this reads it back out of the binary.
#![cfg(windows)]

#[test]
fn the_executable_asks_for_a_detached_console() {
    let binary = std::fs::read(env!("CARGO_BIN_EXE_termherd")).unwrap();
    let wanted = b"<consoleAllocationPolicy xmlns=\"http://schemas.microsoft.com/SMI/2024/WindowsSettings\">detached</consoleAllocationPolicy>";

    assert!(
        binary.windows(wanted.len()).any(|window| window == wanted),
        "termherd.exe carries no detached consoleAllocationPolicy in its manifest"
    );
}
