//! The kill path — the crate's kill-side OS-cfg logic quarantined in one file.
//! `PtyManager::kill` issues the terminate, then hands the raw outcome here
//! where the Unix/Windows split lives, so the manager itself stays cfg-free.

use termherd_core::ports::PtyError;

/// Reconcile the OS kill outcome with the intent of `PtyHost::kill` — the
/// process should end — and return the caller's `Result`.
///
/// On Unix this is [`reconcile_kill`]. On Windows the terminate is best-effort:
/// portable-pty's `WinChildKiller::kill` inverts its result — it returns
/// `Err(last_os_error())` when `TerminateProcess` *succeeds* (a non-zero return
/// is success on Win32) — so its `Result` is unusable; the terminate is still
/// issued, so the call is treated as success.
#[cfg(not(windows))]
pub(crate) fn finish_kill(result: std::io::Result<()>) -> Result<(), PtyError> {
    reconcile_kill(result)
}

#[cfg(windows)]
pub(crate) fn finish_kill(result: std::io::Result<()>) -> Result<(), PtyError> {
    let _ = result;
    Ok(())
}

/// Reconcile a Unix `kill(2)` outcome with the intent of `PtyHost::kill`: the
/// process should end. If the child already exited on its own — the common case
/// when a shell's `exit` races a tab close — the OS reports `ESRCH` ("no such
/// process"); the goal is already met, so that is success, not a failure worth
/// logging. Any other error is a genuine fault.
#[cfg(not(windows))]
fn reconcile_kill(result: std::io::Result<()>) -> Result<(), PtyError> {
    // ESRCH is 3 on Linux, macOS and the BSDs.
    const ESRCH: i32 = 3;
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(ESRCH) => Ok(()),
        Err(e) => Err(PtyError::Io(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    #[test]
    fn reconcile_kill_treats_already_exited_as_success() {
        use super::reconcile_kill;
        use std::io::Error;
        use termherd_core::ports::PtyError;

        // A child that died on its own (ESRCH) is the goal, not a failure.
        assert!(reconcile_kill(Ok(())).is_ok());
        assert!(reconcile_kill(Err(Error::from_raw_os_error(3))).is_ok());

        // Any other OS error is a real fault and must propagate.
        let permission = reconcile_kill(Err(Error::from_raw_os_error(13)));
        assert!(matches!(permission, Err(PtyError::Io(_))));
    }
}
