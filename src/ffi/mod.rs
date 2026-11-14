//! Cross-Platform (`macOS`, `Linux`, `Windows MSVC`) Low-Level OS & Signal/Event FFI (`src/ffi/`).

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub(crate) mod win_sys;
#[cfg(windows)]
pub mod windows;

use std::sync::atomic::AtomicBool;

use crate::platform::ipc_path;

pub static RUNNING: AtomicBool = AtomicBool::new(true);
pub static DIRTY_EVENT: AtomicBool = AtomicBool::new(true);

#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(unix)]
pub use unix::*;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "macos")]
pub use macos::get_ppid_of;

/// Wakes the sidebar process registered for `session_id` or `yazi_id` in `< 0.05ms`
/// (`SIGUSR2` on macOS/Linux; `Local\ccide_wake_<pid>` Win32 Named Event on Windows).
pub fn notify_sidebar(session_id: &str, yazi_id: &str) {
    for (id, prefix) in [(session_id, "sb_"), (yazi_id, "sb_yazi_")] {
        if id.is_empty() {
            continue;
        }
        let p = ipc_path(&format!("{}{}.pid", prefix, id));
        if let Ok(txt) = std::fs::read_to_string(&p) {
            if let Ok(pid) = txt.trim().parse::<u32>() {
                if is_pid_alive(pid) {
                    wake_pid(pid);
                } else {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
}

/// Walks the kernel process tree upward from `pid` to verify whether `pid` is a descendant of `ancestor_pid`.
pub fn is_descendant_of(mut pid: u32, ancestor_pid: u32) -> bool {
    if ancestor_pid == 0 {
        return false;
    }
    for _ in 0..16 {
        if pid == ancestor_pid {
            return true;
        }
        #[cfg(unix)]
        if pid <= 1 {
            return false;
        }
        #[cfg(windows)]
        if pid == 0 || pid == 4 {
            return false;
        }
        match get_ppid_of(pid) {
            Some(p) if p != pid && p > 0 => pid = p,
            _ => return false,
        }
    }
    false
}
