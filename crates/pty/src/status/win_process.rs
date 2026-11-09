//! The Windows process listing and start time a ConPTY pane's job is read
//! from — the workspace's second audited `unsafe` module, after
//! `crates/app/src/macos.rs`.
//!
//! ConPTY has no foreground process group, so [`super::job_in_front`]
//! finds the job by walking down the process tree from the pane's shell. That
//! walk needs a listing of every process with its parent (Toolhelp32), and
//! proving a Claude session file needs its writer's creation time as Claude
//! Code stamps it (`procStart`): the `FILETIME` from `GetProcessTimes`, written
//! as one decimal integer. Neither has a safe binding in `std`, so this module
//! is mechanism only: the choice of descendant stays in the pure, everywhere-
//! tested `process_tree`.
//!
//! Declared under `cfg(windows)` by `status`, the crate's sanctioned home for
//! that fork (`scripts/check-os-cfg-containment.sh`).
#![allow(unsafe_code)]

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::Win32::Foundation::{FILETIME, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::process_tree::Process;

/// Every process running now, with its parent and image name. `None` when
/// Windows refuses the snapshot.
pub(crate) fn processes() -> Option<Vec<Process>> {
    // SAFETY: `CreateToolhelp32Snapshot` takes no pointers; it returns a new
    // handle or `INVALID_HANDLE_VALUE`, checked before the handle is owned.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE || raw.is_null() {
        return None;
    }
    // SAFETY: `raw` is a valid handle this call just opened and nothing else
    // holds, so `OwnedHandle` may close it once, on drop.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).ok()?,
        ..PROCESSENTRY32W::default()
    };
    let mut listing = Vec::new();
    // SAFETY: `snapshot` is a live Toolhelp32 handle and `entry` a writable
    // `PROCESSENTRY32W` whose `dwSize` states its size, as both calls require.
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    while more {
        listing.push(Process {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            name: image_name(&entry.szExeFile),
        });
        // SAFETY: as for `Process32FirstW` above.
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    }
    Some(listing)
}

/// When process `pid` was created, as Claude Code writes it into `procStart`
/// on Windows: the `FILETIME` (100 ns ticks since 1601, UTC) as a decimal
/// integer. `None` when the process is gone or may not be queried.
pub(crate) fn creation_stamp(pid: u32) -> Option<String> {
    // SAFETY: `OpenProcess` takes no pointers; it returns a new handle or null,
    // checked before the handle is owned.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: `raw` is a valid handle this call just opened and nothing else
    // holds, so `OwnedHandle` may close it once, on drop.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let [mut created, mut exited, mut kernel, mut user] = [FILETIME::default(); 4];
    // SAFETY: `process` is live and opened with the access `GetProcessTimes`
    // needs; the four out-pointers name distinct, writable `FILETIME`s.
    let read = unsafe {
        GetProcessTimes(
            process.as_raw_handle(),
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    };
    (read != 0).then(|| filetime_stamp(created))
}

/// A `FILETIME` as the one integer its two halves make.
fn filetime_stamp(time: FILETIME) -> String {
    (u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime)).to_string()
}

/// An image name from Toolhelp32's NUL-terminated UTF-16 buffer.
fn image_name(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_filetime_is_stamped_as_claude_writes_it() {
        // Read from a live `~/.claude/sessions/<pid>.json` on Windows.
        let stamp: u64 = 134_386_429_107_394_753;
        let time = FILETIME {
            dwLowDateTime: (stamp & 0xffff_ffff) as u32,
            dwHighDateTime: (stamp >> 32) as u32,
        };
        assert_eq!(filetime_stamp(time), "134386429107394753");
    }

    #[test]
    fn an_image_name_ends_at_its_nul() {
        let mut buffer = [0_u16; 8];
        for (slot, unit) in buffer.iter_mut().zip("pwsh.exe".encode_utf16()) {
            *slot = unit;
        }
        assert_eq!(image_name(&buffer), "pwsh.exe");
        assert_eq!(image_name(&buffer[..4]), "pwsh");
    }

    #[test]
    fn this_test_is_listed_under_its_own_parent() {
        let me = std::process::id();
        let listing = processes().expect("a process snapshot");
        let entry = listing
            .iter()
            .find(|process| process.pid == me)
            .expect("this process in the listing");
        assert!(
            Path::new(&entry.name).extension().is_some(),
            "an image name, got {:?}",
            entry.name
        );
        assert!(listing.iter().any(|process| process.pid == entry.parent));
    }

    #[test]
    fn a_live_process_has_a_creation_stamp_and_a_missing_one_has_none() {
        let stamp = creation_stamp(std::process::id()).expect("this test's own process");
        // After 2024-01-01 (133_485_408_000_000_000 ticks), as a plain integer.
        assert!(
            stamp
                .parse::<u64>()
                .is_ok_and(|ticks| ticks > 133_485_408_000_000_000)
        );
        assert_eq!(creation_stamp(u32::MAX), None);
    }
}
