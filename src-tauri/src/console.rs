//! `kurasi.exe` is a GUI-subsystem executable (no console window on double-click). The console
//! subcommands attach to the parent console so `kurasi.exe --version` and `scan` print as
//! usual when run from a terminal. When stdout is already a pipe or file nothing changes.

use windows::core::w;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
    STD_HANDLE, STD_OUTPUT_HANDLE,
};

fn handle_missing(which: STD_HANDLE) -> bool {
    unsafe {
        match GetStdHandle(which) {
            Ok(h) => h.0.is_null() || h == INVALID_HANDLE_VALUE,
            Err(_) => true,
        }
    }
}

pub fn attach() {
    let need_out = handle_missing(STD_OUTPUT_HANDLE);
    let need_err = handle_missing(STD_ERROR_HANDLE);
    if !need_out && !need_err {
        return;
    }
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        for (which, missing) in [(STD_OUTPUT_HANDLE, need_out), (STD_ERROR_HANDLE, need_err)] {
            if !missing || !handle_missing(which) {
                continue;
            }
            if let Ok(h) = CreateFileW(
                w!("CONOUT$"),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                Some(HANDLE::default()),
            ) {
                let _ = SetStdHandle(which, h);
            }
        }
    }
}
