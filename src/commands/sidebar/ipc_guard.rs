//! `ready_pid_<pid>` Patch Handshake, Legacy IPC Fallback & Signal Safety Guards.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::ffi::{current_euid, ProcToken};
use crate::platform::{ipc_dir, ipc_path, legacy_ipc_dir};
use crate::session::is_valid_model_name;

pub(crate) fn mtime_epoch_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Validates that `path` is a regular file (not a symlink), owned by `expected_uid` on Unix,
/// and modified no earlier than `start_ms - 1000` (proving the current process instance wrote it).
pub(crate) fn is_valid_proof_file(path: &Path, start_ms: u64, expected_uid: u32) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    let ft = meta.file_type();
    if !ft.is_file() || ft.is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != expected_uid {
            return false;
        }
    }
    #[cfg(not(unix))]
    let _ = expected_uid;
    mtime_epoch_ms(&meta).saturating_add(1000) >= start_ms
}

/// Verifies that `pid` belongs to a live, patched Claude Code instance that installed its
/// `SIGUSR2` handler (via `ready_pid_<pid>` or legacy `cur_pid_<pid>.txt`), returning the
/// directory where `req_pid_<pid>.json` should be written along with the verified `ProcToken`.
pub fn claude_signalable(pid: u32) -> Option<(PathBuf, ProcToken)> {
    if cfg!(windows) {
        return ProcToken::current(pid).map(|t| (ipc_dir(), t));
    }
    let tok = ProcToken::current(pid)?;
    if tok.start_ms == 0 {
        return None;
    }
    let euid = current_euid();
    let idir = ipc_dir();
    let mut cands = vec![
        idir.join(format!("ready_pid_{pid}")),
        idir.join(format!("cur_pid_{pid}.txt")),
    ];
    if let Some(ldir) = legacy_ipc_dir() {
        cands.push(ldir.join(format!("ready_pid_{pid}")));
        cands.push(ldir.join(format!("cur_pid_{pid}.txt")));
    }
    for cand in cands {
        if is_valid_proof_file(&cand, tok.start_ms, euid) {
            return Some((cand.parent()?.to_path_buf(), tok));
        }
    }
    None
}

/// Returns `true` if a view-switch request (`req_pid_<pid>.json` / `req_<sid>.json`) was
/// written within the last `1.5s` and has not yet been unlinked by Claude Code.
pub fn has_pending_switch_request(claude_pid: u32, session_id: &str) -> bool {
    let mut cands = Vec::with_capacity(2);
    if claude_pid > 1 {
        cands.push(ipc_path(&format!("req_pid_{claude_pid}.json")));
        if let Some(ldir) = legacy_ipc_dir() {
            cands.push(ldir.join(format!("req_pid_{claude_pid}.json")));
        }
    } else if !session_id.is_empty() {
        cands.push(ipc_path(&format!("req_{session_id}.json")));
    }
    cands.into_iter().any(|cand| {
        fs::metadata(&cand)
            .and_then(|m| m.modified())
            .ok()
            .map(|mt| {
                SystemTime::now().duration_since(mt).unwrap_or_default()
                    < Duration::from_millis(1500)
            })
            .unwrap_or(false)
    })
}

pub fn read_pushed_claude_state(
    claude_pid: u32,
    session_id: &str,
) -> Option<(String, Option<(SystemTime, String)>)> {
    let mut cands = Vec::with_capacity(3);
    if claude_pid > 1 {
        cands.push(ipc_path(&format!("cur_pid_{claude_pid}.txt")));
        if let Some(ldir) = legacy_ipc_dir() {
            cands.push(ldir.join(format!("cur_pid_{claude_pid}.txt")));
        }
    }
    if !session_id.is_empty() {
        cands.push(ipc_path(&format!("cur_{session_id}.txt")));
    }
    for path in cands {
        if let (Ok(meta), Ok(txt)) = (fs::metadata(&path), fs::read_to_string(&path)) {
            let mt = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let mut lines = txt.lines();
            let view = lines.next().unwrap_or("main").trim();
            let norm = if view.is_empty() { "main" } else { view }.to_string();
            let rm = lines.next().unwrap_or("").trim();
            let valid = is_valid_model_name(rm).then(|| (mt, rm.to_string()));
            return Some((norm, valid));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_proof_file_ownership_symlink_and_mtime() {
        let tmp = std::env::temp_dir().join(format!("ccide_proof_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let euid = current_euid();

        let missing = tmp.join("missing");
        assert!(!is_valid_proof_file(&missing, 1000, euid));

        let reg = tmp.join("ready_pid_123");
        fs::write(&reg, "1\n").unwrap();
        let now_ms = mtime_epoch_ms(&fs::symlink_metadata(&reg).unwrap());

        assert!(is_valid_proof_file(&reg, now_ms, euid));
        assert!(is_valid_proof_file(&reg, now_ms + 900, euid));
        assert!(!is_valid_proof_file(&reg, now_ms + 5000, euid));

        #[cfg(unix)]
        {
            assert!(!is_valid_proof_file(&reg, now_ms, euid.wrapping_add(1)));
            let link = tmp.join("ready_symlink");
            std::os::unix::fs::symlink(&reg, &link).unwrap();
            assert!(!is_valid_proof_file(&link, now_ms, euid));
        }

        let _ = fs::remove_dir_all(&tmp);
    }
}
