//! Native Win32 (`x86_64-pc-windows-msvc` / `aarch64-pc-windows-msvc`) Console, Process & Event FFI.

use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use crate::ffi::win_sys::*;
use crate::ffi::{DIRTY_EVENT, RUNNING};

unsafe extern "system" fn ctrl_handler(_ctrl_type: u32) -> i32 {
    RUNNING.store(false, Ordering::SeqCst);
    1
}

fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn current_euid() -> u32 {
    0
}

pub fn install_signal_handlers(_enable_wakeup_sig: bool) {
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), 1);
    }
}

pub struct ConsoleRawSession {
    h_in: Handle,
    h_out: Handle,
    wake_ev: Handle,
    orig_in_mode: u32,
    orig_out_mode: u32,
    has_in: bool,
    has_out: bool,
    pending: Mutex<Vec<u8>>,
}

impl ConsoleRawSession {
    pub fn enter() -> Self {
        unsafe {
            let (h_in, h_out) = (
                GetStdHandle(STD_INPUT_HANDLE),
                GetStdHandle(STD_OUTPUT_HANDLE),
            );
            let (mut orig_in_mode, mut orig_out_mode) = (0u32, 0u32);
            let has_in = !h_in.is_null()
                && h_in != INVALID_HANDLE_VALUE
                && GetConsoleMode(h_in, &mut orig_in_mode) != 0;
            let has_out = !h_out.is_null()
                && h_out != INVALID_HANDLE_VALUE
                && GetConsoleMode(h_out, &mut orig_out_mode) != 0;
            if has_out {
                let _ = SetConsoleMode(
                    h_out,
                    orig_out_mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                );
            }
            if has_in {
                let mut raw_in = (orig_in_mode
                    | ENABLE_VIRTUAL_TERMINAL_INPUT
                    | ENABLE_MOUSE_INPUT
                    | ENABLE_WINDOW_INPUT
                    | ENABLE_EXTENDED_FLAGS)
                    & !(ENABLE_LINE_INPUT
                        | ENABLE_ECHO_INPUT
                        | ENABLE_PROCESSED_INPUT
                        | ENABLE_QUICK_EDIT_MODE);
                if SetConsoleMode(h_in, raw_in) == 0 {
                    raw_in &= !ENABLE_VIRTUAL_TERMINAL_INPUT;
                    let _ = SetConsoleMode(h_in, raw_in);
                }
            }
            let ev_name = wide_null(&format!("Local\\ccide_wake_{}", std::process::id()));
            let wake_ev = CreateEventW(ptr::null_mut(), 0, 0, ev_name.as_ptr());
            Self {
                h_in,
                h_out,
                wake_ev,
                orig_in_mode,
                orig_out_mode,
                has_in,
                has_out,
                pending: Mutex::new(Vec::new()),
            }
        }
    }

    pub fn poll_stdin(&self, buf: &mut [u8], timeout_ms: i32) -> Option<usize> {
        if !buf.is_empty() {
            if let Ok(mut q) = self.pending.lock() {
                if !q.is_empty() {
                    let n = q.len().min(buf.len());
                    buf[..n].copy_from_slice(&q[..n]);
                    q.drain(..n);
                    return Some(n);
                }
            }
        }
        unsafe {
            let t_ms = if timeout_ms < 0 {
                u32::MAX
            } else {
                timeout_ms as u32
            };
            let mut handles = [ptr::null_mut(); 2];
            let mut cnt = 0usize;
            if self.has_in {
                handles[cnt] = self.h_in;
                cnt += 1;
            }
            if !self.wake_ev.is_null() {
                handles[cnt] = self.wake_ev;
                cnt += 1;
            }
            if cnt == 0 {
                std::thread::sleep(std::time::Duration::from_millis(t_ms as u64));
                return None;
            }
            let wait_rc = WaitForMultipleObjects(cnt as u32, handles.as_ptr(), 0, t_ms);
            if self.has_in && wait_rc == WAIT_OBJECT_0 {
                let mut recs: [InputRecord; 128] = std::mem::zeroed();
                let mut read_n = 0u32;
                if ReadConsoleInputW(self.h_in, recs.as_mut_ptr(), 128, &mut read_n) != 0
                    && read_n > 0
                {
                    let mut out = String::with_capacity(128);
                    for rec in &recs[..read_n as usize] {
                        rec.append_vt(&mut out, &DIRTY_EVENT);
                    }
                    let bytes = out.as_bytes();
                    if !bytes.is_empty() && !buf.is_empty() {
                        let copy_n = bytes.len().min(buf.len());
                        buf[..copy_n].copy_from_slice(&bytes[..copy_n]);
                        if bytes.len() > copy_n {
                            if let Ok(mut q) = self.pending.lock() {
                                q.extend_from_slice(&bytes[copy_n..]);
                            }
                        }
                        return Some(copy_n);
                    }
                }
            } else if wait_rc == WAIT_OBJECT_0 + (if self.has_in { 1 } else { 0 }) {
                DIRTY_EVENT.store(true, Ordering::SeqCst);
            }
            None
        }
    }
}

impl Drop for ConsoleRawSession {
    fn drop(&mut self) {
        unsafe {
            if self.has_in {
                let _ = SetConsoleMode(self.h_in, self.orig_in_mode);
            }
            if self.has_out {
                let _ = SetConsoleMode(self.h_out, self.orig_out_mode);
            }
            if !self.wake_ev.is_null() {
                let _ = CloseHandle(self.wake_ev);
            }
        }
    }
}

pub fn query_terminal_winsize() -> Option<(usize, usize, f64, f64)> {
    unsafe {
        let h_out = GetStdHandle(STD_OUTPUT_HANDLE);
        if h_out.is_null() || h_out == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut info: ConsoleScreenBufferInfo = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(h_out, &mut info) != 0 {
            let cols = (info.window.right as i32 - info.window.left as i32 + 1).max(1) as usize;
            let rows = (info.window.bottom as i32 - info.window.top as i32 + 1).max(1) as usize;
            return Some((rows, cols, 14.0, 32.0));
        }
        None
    }
}

pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE;
        let _ = CloseHandle(h);
        ok
    }
}

pub fn proc_start_ms(pid: u32) -> Option<u64> {
    if pid <= 1 {
        return None;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let z = FileTime {
            dw_low_date_time: 0,
            dw_high_date_time: 0,
        };
        let (mut ct, mut et, mut kt, mut ut) = (z, z, z, z);
        let ok = GetProcessTimes(h, &mut ct, &mut et, &mut kt, &mut ut) != 0;
        let _ = CloseHandle(h);
        if !ok {
            return None;
        }
        let raw = ((ct.dw_high_date_time as u64) << 32) | (ct.dw_low_date_time as u64);
        raw.checked_sub(116_444_736_000_000_000).map(|d| d / 10_000)
    }
}

pub fn get_ppid_of(pid: u32) -> Option<u32> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: ProcessEntry32W = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
        let mut found = None;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                if entry.th32_process_id == pid {
                    found = Some(entry.th32_parent_process_id);
                    break;
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        let ppid = found?;
        if let (Some(ps), Some(cs)) = (proc_start_ms(ppid), proc_start_ms(pid)) {
            if ps > cs {
                return None;
            }
        }
        Some(ppid)
    }
}

pub fn current_ppid() -> u32 {
    get_ppid_of(std::process::id()).unwrap_or(0)
}

pub(crate) fn wake_pid(pid: u32) -> bool {
    if pid <= 1 {
        return false;
    }
    unsafe {
        let ev_name = wide_null(&format!("Local\\ccide_wake_{pid}"));
        let h = OpenEventW(EVENT_MODIFY_STATE, 0, ev_name.as_ptr());
        if h.is_null() {
            return false;
        }
        let ok = SetEvent(h) != 0;
        let _ = CloseHandle(h);
        ok
    }
}
