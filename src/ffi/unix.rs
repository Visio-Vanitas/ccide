//! POSIX (`macOS` + `Linux`) Terminal Raw Mode, `poll()`, `TIOCGWINSZ`, & `SIGUSR2` Bindings.

use std::ffi::{c_double, c_int, c_uint, c_void};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::Ordering;

use crate::ffi::{DIRTY_EVENT, RUNNING};

pub extern "C" fn handle_sig(_: c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

pub extern "C" fn handle_sigusr2(_: c_int) {
    DIRTY_EVENT.store(true, Ordering::SeqCst);
}

#[cfg(target_os = "macos")]
pub const SIGUSR2: c_int = 31;
#[cfg(not(target_os = "macos"))]
pub const SIGUSR2: c_int = 12;

#[cfg(target_os = "macos")]
pub type NfdsT = std::ffi::c_uint;
#[cfg(not(target_os = "macos"))]
pub type NfdsT = std::ffi::c_ulong;

#[cfg(target_os = "macos")]
pub const TIOCGWINSZ: std::ffi::c_ulong = 0x40087468;
#[cfg(not(target_os = "macos"))]
pub const TIOCGWINSZ: std::ffi::c_ulong = 0x5413;

#[repr(C)]
pub struct Winsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Termios {
    pub c_iflag: u64,
    pub c_oflag: u64,
    pub c_cflag: u64,
    pub c_lflag: u64,
    pub c_cc: [u8; 20],
    pub c_ispeed: u64,
    pub c_ospeed: u64,
}

#[cfg(not(target_os = "macos"))]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Termios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_line: u8,
    pub c_cc: [u8; 32],
    pub c_ispeed: u32,
    pub c_ospeed: u32,
}

impl Termios {
    pub fn zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }

    pub fn make_raw(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.c_iflag &= !(0x0000_0200 | 0x0000_0100);
            self.c_lflag &= !(0x0000_0080 | 0x0000_0100 | 0x0000_0008 | 0x0000_0400);
            self.c_cc[16] = 1;
            self.c_cc[17] = 0;
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.c_iflag &= !(0x0000_0400 | 0x0000_0100);
            self.c_lflag &= !(0x0000_0001 | 0x0000_0002 | 0x0000_0008 | 0x0000_8000);
            self.c_cc[6] = 1;
            self.c_cc[5] = 0;
        }
    }
}

#[repr(C)]
pub struct PollFd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

#[cfg_attr(target_os = "macos", link(name = "System"))]
extern "C" {
    pub fn ioctl(fd: c_int, request: std::ffi::c_ulong, ...) -> c_int;
    pub fn tcgetattr(fd: c_int, termios_p: *mut Termios) -> c_int;
    pub fn tcsetattr(fd: c_int, optional_actions: c_int, termios_p: *const Termios) -> c_int;
    pub fn poll(fds: *mut PollFd, nfds: NfdsT, timeout: c_int) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    pub fn signal(sig: c_int, handler: extern "C" fn(c_int)) -> usize;
    pub fn kill(pid: c_int, sig: c_int) -> c_int;
    pub fn getppid() -> c_int;
    pub fn geteuid() -> c_uint;
    pub fn getloadavg(loadavg: *mut c_double, nelem: c_int) -> c_int;
    #[cfg(not(target_os = "macos"))]
    pub fn sysconf(name: c_int) -> std::ffi::c_long;
}

pub fn current_euid() -> u32 {
    unsafe { geteuid() }
}

pub fn install_signal_handlers(enable_wakeup_sig: bool) {
    unsafe {
        signal(2, handle_sig);
        signal(15, handle_sig);
        if enable_wakeup_sig {
            signal(SIGUSR2, handle_sigusr2);
        }
    }
}

pub struct ConsoleRawSession {
    stdin_fd: c_int,
    orig: Termios,
    active: bool,
}

impl ConsoleRawSession {
    pub fn enter() -> Self {
        let fd = std::io::stdin().as_raw_fd();
        let mut orig = Termios::zeroed();
        let active = unsafe { tcgetattr(fd, &mut orig) == 0 };
        if active {
            let mut raw = orig;
            raw.make_raw();
            unsafe {
                let _ = tcsetattr(fd, 0, &raw);
            }
        }
        Self {
            stdin_fd: fd,
            orig,
            active,
        }
    }

    pub fn poll_stdin(&self, buf: &mut [u8], timeout_ms: i32) -> Option<usize> {
        let mut pfd = PollFd {
            fd: self.stdin_fd,
            events: 1,
            revents: 0,
        };
        if unsafe { poll(&mut pfd, 1 as NfdsT, timeout_ms as c_int) } <= 0 {
            return None;
        }
        if (pfd.revents & (0x0008 | 0x0010 | 0x0020)) != 0 {
            RUNNING.store(false, Ordering::SeqCst);
            return None;
        }
        if (pfd.revents & 1) == 0 {
            return None;
        }
        let n = unsafe { read(self.stdin_fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if n <= 0 {
            if n == 0 {
                RUNNING.store(false, Ordering::SeqCst);
            }
            return None;
        }
        let mut total = n as usize;
        while total < buf.len() && ends_with_incomplete_esc(&buf[..total]) {
            pfd.revents = 0;
            if unsafe { poll(&mut pfd, 1 as NfdsT, 2) } <= 0 || (pfd.revents & 1) == 0 {
                break;
            }
            let extra = unsafe {
                read(
                    self.stdin_fd,
                    buf[total..].as_mut_ptr() as *mut c_void,
                    buf.len() - total,
                )
            };
            if extra <= 0 {
                break;
            }
            total += extra as usize;
        }
        Some(total)
    }
}

fn ends_with_incomplete_esc(bytes: &[u8]) -> bool {
    let Some(esc_pos) = bytes.iter().rposition(|&b| b == 0x1b) else {
        return false;
    };
    let tail = &bytes[esc_pos..];
    tail.len() == 1 || (tail[1] == b'[' && !tail[2..].iter().any(|&b| (0x40..=0x7e).contains(&b)))
}

impl Drop for ConsoleRawSession {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                let _ = tcsetattr(self.stdin_fd, 0, &self.orig);
            }
        }
    }
}

pub fn query_terminal_winsize() -> Option<(usize, usize, f64, f64)> {
    unsafe {
        let mut ws = Winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if ioctl(1, TIOCGWINSZ, &mut ws as *mut Winsize) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            let (cols, rows) = (ws.ws_col as usize, ws.ws_row as usize);
            let cw = if ws.ws_xpixel > 0 {
                (ws.ws_xpixel as f64) / (cols as f64)
            } else {
                14.0
            };
            let ch = if ws.ws_ypixel > 0 {
                (ws.ws_ypixel as f64) / (rows as f64)
            } else {
                32.0
            };
            return Some((rows, cols, cw, ch));
        }
    }
    None
}

pub fn is_pid_alive(pid: u32) -> bool {
    pid > 0 && pid <= i32::MAX as u32 && unsafe { kill(pid as c_int, 0) == 0 }
}

pub fn current_ppid() -> u32 {
    let p = unsafe { getppid() };
    if p > 0 {
        p as u32
    } else {
        0
    }
}

#[cfg(not(target_os = "macos"))]
pub fn get_ppid_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.get(stat.rfind(')')? + 1..)?
        .split_whitespace()
        .nth(1)?
        .parse::<u32>()
        .ok()
}

#[cfg(not(target_os = "macos"))]
pub fn proc_start_ms(pid: u32) -> Option<u64> {
    static BTIME_S: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    if pid <= 1 {
        return None;
    }
    let btime = *BTIME_S.get_or_init(|| {
        std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())
            })
            .unwrap_or(0)
    });
    if btime == 0 {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks: u64 = stat
        .get(stat.rfind(')')? + 1..)?
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()?;
    let hz = unsafe { sysconf(2) };
    let clk = if hz > 0 { hz as u64 } else { 100 };
    Some(
        btime
            .saturating_mul(1000)
            .saturating_add(ticks.saturating_mul(1000) / clk),
    )
}

pub(crate) fn wake_pid(pid: u32) -> bool {
    pid > 1 && pid <= i32::MAX as u32 && unsafe { kill(pid as c_int, SIGUSR2) == 0 }
}
