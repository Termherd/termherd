//! Single-instance guard — one termherd per login session, enforced at
//! startup. Split out of `main` so the per-OS lock-naming rules and their
//! regression tests live together.

use single_instance::SingleInstance;
use tracing::warn;

/// Base name of the single-instance lock. Its *form* is adjusted per OS by
/// [`lock_name`], because `single-instance` gives this string a different
/// meaning on each platform.
const INSTANCE_LOCK: &str = "dev.termherd.lock";

/// The single-instance lock identifier, in the form each OS requires. The
/// `single-instance` crate gives this one string three different meanings, and
/// a value valid on one platform is broken on another:
///
/// - **macOS** treats it as a *file path* and `flock`s it, so it must be an
///   absolute, writable path. A bare name is created relative to the CWD —
///   which is `/` (read-only) when launched from the `.app` bundle, so the app
///   would silently refuse to start (the "double-click does nothing" bug).
/// - **Windows** passes it to `CreateMutexW` as a *mutex name*, which must not
///   contain `\` (those are reserved namespace separators). A full path there
///   yields `ERROR_PATH_NOT_FOUND` (3) and the lock silently disables itself.
///   A bare name lands in the per-login-session namespace — the scope we want.
/// - **Linux** binds it as an *abstract socket name* — any opaque string works.
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
        std::borrow::Cow::Borrowed(INSTANCE_LOCK)
    }
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
        // A CWD-relative lock path is the Finder/.app launch bug: from `/`
        // (read-only) it can't be created and the app refuses to start.
        assert!(
            std::path::Path::new(super::lock_name().as_ref()).is_absolute(),
            "macOS single-instance lock path must be CWD-independent"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn lock_name_has_no_backslash_on_windows() {
        // A Windows mutex name treats `\` as a namespace separator: a path
        // yields ERROR_PATH_NOT_FOUND (3) and silently disables the lock.
        assert!(
            !super::lock_name().contains('\\'),
            "Windows single-instance mutex name must not contain backslashes"
        );
    }
}
