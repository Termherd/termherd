//! Build script: embed the Windows application icon into `termherd.exe`, so the
//! executable and the shortcuts the installer creates show the app icon in
//! Explorer, the Start menu and the taskbar. A no-op on every non-Windows host.
//!
//! The `#[cfg(windows)]` here is the *host* platform (build scripts are
//! compiled for the host), matching the host-evaluated
//! `[target.'cfg(windows)'.build-dependencies]` in `Cargo.toml`: the
//! `winresource` reference only compiles where the dependency is present. CI
//! builds the Windows artifact on a Windows runner, so the icon lands there.

fn main() {
    #[cfg(windows)]
    embed_windows_icon();
}

#[cfg(windows)]
fn embed_windows_icon() {
    println!("cargo:rerun-if-changed=icons/icon.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("icons/icon.ico");
    if let Err(error) = res.compile() {
        // The icon is a nicety, not a correctness requirement: a missing RC
        // toolchain must warn, never fail the build.
        println!("cargo:warning=could not embed Windows icon: {error}");
    }
}
