//! Session Directory Scan (Mtime-Cached) & Live Yazi Tab Discovery (`src/session/scan.rs`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::ffi::{current_ppid, is_descendant_of, is_pid_alive};
use crate::json::{extract_json_bool, extract_json_str, extract_json_u64};
use crate::platform::{clean_canonicalize, ipc_dir, paths_equal_ci};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessEntry {
    pub pid: u32,
    pub session_id: String,
    pub cwd: PathBuf,
    pub kind: String,
    pub name: String,
    pub job_id: String,
    pub parked_job_id: String,
    pub is_busy: bool,
    pub updated_at: u64,
    pub started_at: u64,
}

struct CachedFile {
    path: PathBuf,
    mtime: SystemTime,
    entry: Option<SessEntry>,
}

struct SessionScanCache {
    sess_dir: String,
    dir_mtime: SystemTime,
    files: Vec<CachedFile>,
}

static SESSION_CACHE: Mutex<Option<SessionScanCache>> = Mutex::new(None);

fn parse_session_file(path: &Path) -> Option<SessEntry> {
    let txt = fs::read_to_string(path).ok()?;
    if extract_json_bool(&txt, "spare") == Some(true) {
        return None;
    }
    let pid = extract_json_u64(&txt, "pid").unwrap_or(0) as u32;
    let sid = extract_json_str(&txt, "sessionId")?;
    let cwd_raw = extract_json_str(&txt, "cwd").unwrap_or_default();
    let cwd = clean_canonicalize(&cwd_raw).unwrap_or_else(|_| PathBuf::from(&cwd_raw));
    let kind = extract_json_str(&txt, "kind").unwrap_or_default();
    let name = extract_json_str(&txt, "name").unwrap_or_default();
    let job_id = extract_json_str(&txt, "jobId").unwrap_or_default();
    let parked_job_id = extract_json_str(&txt, "parkedJobId").unwrap_or_default();
    let status = extract_json_str(&txt, "status").unwrap_or_default();
    let started_at = extract_json_u64(&txt, "startedAt").unwrap_or(0);
    let updated_at = extract_json_u64(&txt, "updatedAt").unwrap_or(started_at);
    Some(SessEntry {
        pid,
        session_id: sid,
        cwd,
        kind,
        name,
        job_id,
        parked_job_id,
        is_busy: status == "busy",
        updated_at,
        started_at,
    })
}

/// Scans `~/.claude/sessions/*.json`, caching directory entries by `dir_mtime` (and per-file
/// `mtime`) so 160ms sidebar ticks avoid redundant directory reads and JSON parsing.
pub fn scan_alive_sessions(home: &str) -> Vec<SessEntry> {
    let sess_dir = format!("{}/.claude/sessions", home);
    let Ok(dir_meta) = fs::metadata(&sess_dir) else {
        return Vec::new();
    };
    let Ok(dir_mtime) = dir_meta.modified() else {
        return Vec::new();
    };

    let mut guard = SESSION_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache_hit = guard
        .as_ref()
        .map(|c| c.sess_dir == sess_dir && c.dir_mtime == dir_mtime)
        .unwrap_or(false);

    if cache_hit {
        if let Some(cache) = guard.as_mut() {
            for cf in &mut cache.files {
                match fs::metadata(&cf.path).and_then(|m| m.modified()) {
                    Ok(mt) if mt == cf.mtime => {}
                    Ok(mt) => {
                        cf.mtime = mt;
                        cf.entry = parse_session_file(&cf.path);
                    }
                    Err(_) => {
                        cf.entry = None;
                    }
                }
            }
        }
    } else {
        let mut files = Vec::new();
        if let Ok(rd) = fs::read_dir(&sess_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let mt = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                let parsed = parse_session_file(&p);
                files.push(CachedFile {
                    path: p,
                    mtime: mt,
                    entry: parsed,
                });
            }
        }
        *guard = Some(SessionScanCache {
            sess_dir,
            dir_mtime,
            files,
        });
    }

    let mut alive_entries: Vec<SessEntry> = guard
        .as_ref()
        .map(|c| {
            c.files
                .iter()
                .filter_map(|cf| cf.entry.as_ref())
                .filter(|e| is_pid_alive(e.pid))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    drop(guard);

    alive_entries.sort_by_key(|b| std::cmp::Reverse((b.is_busy, b.updated_at, b.started_at)));
    alive_entries
}

pub fn resolve_yazi_id_from_ppid(cache_dir_path: &str) -> String {
    let ppid = current_ppid();
    if ppid <= 1 {
        return String::new();
    }
    let mut best: Option<(SystemTime, String)> = None;
    if let Ok(rd) = fs::read_dir(cache_dir_path) {
        for entry in rd.flatten() {
            let p = entry.path();
            let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if let Some(yid) = fname
                .strip_prefix("tab_shell_")
                .and_then(|s| s.strip_suffix(".txt"))
            {
                if let Ok(stxt) = fs::read_to_string(&p) {
                    if let Ok(spid) = stxt.trim().parse::<u32>() {
                        if is_descendant_of(ppid, spid) {
                            let mtime = entry
                                .metadata()
                                .and_then(|m| m.modified())
                                .unwrap_or(SystemTime::UNIX_EPOCH);
                            if best
                                .as_ref()
                                .map(|(b_mt, _)| mtime >= *b_mt)
                                .unwrap_or(true)
                            {
                                best = Some((mtime, yid.to_string()));
                            }
                        }
                    }
                }
            }
        }
    }
    best.map(|(_, yid)| yid).unwrap_or_default()
}

pub fn resolve_live_yazi_tab(
    cache_dir_path: &str,
    session_id: &str,
    cwd_path: &str,
) -> (String, String) {
    let ppid = current_ppid();
    let canon_cwd = if !cwd_path.is_empty() {
        clean_canonicalize(cwd_path).ok()
    } else {
        None
    };
    let bound_yid = if !session_id.is_empty() {
        fs::read_to_string(format!("{}/session_{}.yazi", cache_dir_path, session_id))
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        String::new()
    };

    struct LiveTab {
        mtime: SystemTime,
        yazi_id: String,
        cwd: String,
        shell_pid: u32,
    }

    let mut live_tabs = Vec::new();
    if let Ok(rd) = fs::read_dir(ipc_dir()) {
        for entry in rd.flatten() {
            let p = entry.path();
            let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !fname.starts_with("sidebar_tab_") || !fname.ends_with(".json") {
                continue;
            }
            let Ok(txt) = fs::read_to_string(&p) else {
                continue;
            };
            let sb_pid = extract_json_u64(&txt, "pid").unwrap_or(0) as u32;
            if !is_pid_alive(sb_pid) {
                continue;
            }
            let yid = extract_json_str(&txt, "yazi_id").unwrap_or_default();
            if yid.is_empty() {
                continue;
            }
            let cwd = extract_json_str(&txt, "cwd").unwrap_or_default();
            let shell_pid = extract_json_u64(&txt, "shell_pid").unwrap_or(0) as u32;
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            live_tabs.push(LiveTab {
                mtime,
                yazi_id: yid,
                cwd,
                shell_pid,
            });
        }
    }
    live_tabs.sort_by_key(|b| std::cmp::Reverse(b.mtime));

    if ppid > 1 {
        for lt in &live_tabs {
            if lt.shell_pid > 0 && is_descendant_of(ppid, lt.shell_pid) {
                return (lt.yazi_id.clone(), lt.cwd.clone());
            }
        }
    }
    if !bound_yid.is_empty() {
        if let Some(lt) = live_tabs.iter().find(|t| t.yazi_id == bound_yid) {
            return (lt.yazi_id.clone(), lt.cwd.clone());
        }
    }
    if let Some(ref cc) = canon_cwd {
        for lt in &live_tabs {
            if !lt.cwd.is_empty() {
                if let Ok(lt_canon) = clean_canonicalize(&lt.cwd) {
                    if paths_equal_ci(&lt_canon, cc) {
                        return (lt.yazi_id.clone(), lt.cwd.clone());
                    }
                }
            }
        }
    }

    let ppid_yid = resolve_yazi_id_from_ppid(cache_dir_path);
    if !ppid_yid.is_empty() {
        return (ppid_yid, cwd_path.to_string());
    }
    let env_yid = std::env::var("CC_YAZI_ID").unwrap_or_default();
    if !env_yid.is_empty() {
        return (env_yid, cwd_path.to_string());
    }
    (bound_yid, cwd_path.to_string())
}
