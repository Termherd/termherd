//! Single-instance guard — one termherd per login session, enforced at
//! startup. Split out of `main` so the per-OS lock-naming rules and their
//! regression tests live together.

use single_instance::SingleInstance;
use tracing::warn;

/// Base name of the single-instance lock. Its *form* is adjusted per OS by
/// [`lock_name`], because `single-instance` gives this string a different
/// meaning on each platform.
const INSTANCE_LOCK: &str = "dev.termherd.lock";

/// The lock identifier, scoped to the temp dir so a build with its own
/// `TMPDIR` (`TMP` on Windows) runs beside the installed app.
///
/// `single-instance` reads the string differently per OS:
/// - **macOS**: a file path to `flock`; must be absolute, since a `.app`
///   starts in read-only `/`.
/// - **Windows**: a mutex name; a `\` disables the lock silently.
/// - **Linux**: an abstract socket name, at most 107 bytes.
///
/// Hence a hash of the dir off macOS, never the path.
fn lock_name() -> std::borrow::Cow<'static, str> {
    #[cfg(target_os = "macos")]
    {
        std::borrow::Cow::Owned(
            std::env::temp_dir()
                .join(INSTANCE_LOCK)
                .to_string_lossy()
                .into_owned(),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::borrow::Cow::Owned(scoped_name(&std::env::temp_dir()))
    }
}

#[cfg(not(target_os = "macos"))]
fn scoped_name(temp_dir: &std::path::Path) -> String {
    format!(
        "{INSTANCE_LOCK}-{:016x}",
        fnv1a(temp_dir.to_string_lossy().as_bytes())
    )
}

/// 64-bit FNV-1a: unlike `DefaultHasher`, stable across Rust releases, so
/// every build derives the same lock name.
#[cfg(not(target_os = "macos"))]
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Acquire the single-instance guard, returning it to hold for the process
/// lifetime — or `None` when the lock subsystem is unavailable.
///
/// Exits the process only when another instance already holds the lock. A
/// failure to *create* the lock must not stop the app from launching: that was
/// the "double-click does nothing, no error" bug on the `.app` bundle.
pub fn acquire_single_instance() -> Option<SingleInstance> {
    let name = lock_name();
    match SingleInstance::new(&name) {
        Ok(instance) => {
            if !instance.is_single() {
                warn!("another termherd instance is already running; exiting");
                // Non-zero so a launcher can tell this from a clean shutdown.
                std::process::exit(2);
            }
            Some(instance)
        }
        Err(e) => {
            warn!(
                error = %e, lock = %name,
                "single-instance lock unavailable; launching without it"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn lock_name_is_absolute_on_macos() {
        assert!(
            std::path::Path::new(super::lock_name().as_ref()).is_absolute(),
            "macOS single-instance lock path must be CWD-independent"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn lock_name_has_no_backslash_on_windows() {
        assert!(
            !super::lock_name().contains('\\'),
            "Windows single-instance mutex name must not contain backslashes"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_lock_name_follows_the_temp_dir() {
        let default = super::scoped_name(std::path::Path::new("/tmp"));
        let throwaway = super::scoped_name(std::path::Path::new("/tmp/tmp.aZ3"));
        assert_ne!(
            default, throwaway,
            "a throwaway temp dir must move the lock"
        );
        assert_eq!(
            default,
            super::scoped_name(std::path::Path::new("/tmp")),
            "one temp dir must always give one lock name"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_lock_name_is_short_and_has_no_backslash_whatever_the_dir() {
        let long = format!(r"C:\Users\someone\AppData\Local\Temp\{}", "x".repeat(300));
        let name = super::scoped_name(std::path::Path::new(&long));
        assert!(!name.contains('\\'), "{name}");
        assert!(
            name.len() < 107,
            "abstract socket names stop at 107 bytes: {name}"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn fnv1a_matches_the_reference_vectors() {
        // Published FNV-1a 64 vectors.
        assert_eq!(super::fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(super::fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
