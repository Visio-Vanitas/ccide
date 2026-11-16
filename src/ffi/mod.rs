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

use crate::platform::{ipc_dir, ipc_path};

pub static RUNNING: AtomicBool = AtomicBool::new(true);
pub static DIRTY_EVENT: AtomicBool = AtomicBool::new(true);

#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(unix)]
pub use unix::*;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "macos")]
pub use macos::{get_ppid_of, proc_start_ms};

/// Generation-guarded process identity token (`<pid>:<start_ms>`) preventing PID-reuse mis-signaling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcToken {
    pub pid: u32,
    pub start_ms: u64,
}

impl ProcToken {
    pub fn parse(s: &str) -> Option<Self> {
        let t = s.trim();
        if t.is_empty() {
            return None;
        }
        let (pid_s, start_ms) = match t.split_once(':') {
            Some((p, ms)) => (p.trim(), ms.trim().parse::<u64>().ok()?),
            None => (t, 0),
        };
        let pid = pid_s.parse::<u32>().ok()?;
        if pid <= 1 || pid > i32::MAX as u32 {
            return None;
        }
        Some(Self { pid, start_ms })
    }

    pub fn encode(&self) -> String {
        if self.start_ms == 0 {
            format!("{}", self.pid)
        } else {
            format!("{}:{}", self.pid, self.start_ms)
        }
    }

    pub fn current(pid: u32) -> Option<Self> {
        if pid <= 1 || pid > i32::MAX as u32 {
            None
        } else {
            Some(Self {
                pid,
                start_ms: proc_start_ms(pid)?,
            })
        }
    }

    pub fn self_token() -> Self {
        let pid = std::process::id();
        Self {
            pid,
            start_ms: proc_start_ms(pid).unwrap_or(0),
        }
    }

    /// Returns `true` only when `start_ms != 0` and the live kernel process start time matches.
    pub fn is_alive_match(&self) -> bool {
        self.start_ms != 0 && Self::current(self.pid) == Some(*self)
    }
}

/// Wakes target process identified by `t`: on Unix sends `SIGUSR2` ONLY when `t.is_alive_match()`;
/// on Windows signals `Local\ccide_wake_<pid>` named event.
pub fn wake_token(t: &ProcToken) -> bool {
    #[cfg(unix)]
    {
        t.is_alive_match() && wake_pid(t.pid)
    }
    #[cfg(windows)]
    {
        t.pid > 1
            && t.pid <= i32::MAX as u32
            && (t.start_ms == 0 || t.is_alive_match())
            && wake_pid(t.pid)
    }
}

/// Wakes the sidebar process registered for `session_id` or `yazi_id` if its `ProcToken` matches.
pub fn notify_sidebar(session_id: &str, yazi_id: &str) {
    let mut woke_any = false;
    for (id, prefix) in [(session_id, "sb_"), (yazi_id, "sb_yazi_")] {
        if id.is_empty() {
            continue;
        }
        let p = ipc_path(&format!("{prefix}{id}.pid"));
        if let Ok(txt) = std::fs::read_to_string(&p) {
            if let Some(tok) = ProcToken::parse(&txt).filter(|t| t.is_alive_match()) {
                let _ = wake_token(&tok);
                woke_any = true;
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    if !woke_any {
        if let Ok(rd) = std::fs::read_dir(ipc_dir()) {
            for ent in rd.flatten() {
                let p = ent.path();
                let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if fname.starts_with("sb_yazi_") && fname.ends_with(".pid") {
                    if let Ok(txt) = std::fs::read_to_string(&p) {
                        if let Some(tok) = ProcToken::parse(&txt).filter(|t| t.is_alive_match()) {
                            let _ = wake_token(&tok);
                        } else {
                            let _ = std::fs::remove_file(&p);
                        }
                    }
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
    for _ in 0..=32 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn proc_token_parses_encodes_and_verifies_live_self_token() {
        let bare = ProcToken::parse(" 4242 ").expect("bare pid");
        assert_eq!(
            bare,
            ProcToken {
                pid: 4242,
                start_ms: 0
            }
        );
        assert_eq!(bare.encode(), "4242");
        assert!(!bare.is_alive_match());
        assert!(ProcToken::parse("1").is_none());
        assert!(ProcToken::parse("0:123").is_none());

        let full = ProcToken::parse("4242:1700000000123").expect("full token");
        assert_eq!(full.encode(), "4242:1700000000123");

        let me = ProcToken::self_token();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(me.start_ms > 0 && now_ms.abs_diff(me.start_ms) < 60_000);
        assert!(me.is_alive_match());
        assert!(!ProcToken {
            pid: me.pid,
            start_ms: me.start_ms + 5000
        }
        .is_alive_match());
    }
}
