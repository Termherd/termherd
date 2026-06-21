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
    emit_build_date();
    #[cfg(windows)]
    embed_windows_icon();
}

/// Stamp the wall-clock build time (UTC) into `TERMHERD_BUILD_DATE` so `main`
/// can surface it on the startup log line via `env!`. Dependency-free: the
/// civil-date conversion is Howard Hinnant's `days_from_civil` inverse.
fn emit_build_date() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=TERMHERD_BUILD_DATE={}", format_utc(secs));
}

/// UNIX seconds -> `YYYY-MM-DD HH:MM:SS UTC`, no external crate.
fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, min, sec) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = yoe + era * 400 + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02} UTC")
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
