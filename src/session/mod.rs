//! Strict Process-Isolated Session & Tab Resolution (`src/session/mod.rs`).

pub mod focus;
pub mod model;
pub mod subagents;
pub mod transcript;

use std::fs;
use std::path::PathBuf;

use crate::ffi::{current_ppid, get_ppid_of, is_descendant_of, is_pid_alive};
use crate::json::{extract_json_bool, extract_json_str, extract_json_u64};
use crate::platform::{cache_dir, clean_canonicalize, ipc_dir, normalize_slashes};

pub use focus::{emit_yazi_blur, emit_yazi_reveal_at, extract_tool_line_target};
pub use model::{
    classify_model_family, compact_model_for_width, is_valid_model_name, load_config_models,
    map_alias_with_settings, short_model_label, ConfigModels,
};

fn ancestor_distance(mut pid: u32, target_ancestor: u32) -> Option<usize> {
    if target_ancestor == 0 || pid == 0 {
        return None;
    }
    for hops in 0..=32 {
        if pid == target_ancestor {
            return Some(hops);
        }
        #[cfg(unix)]
        if pid <= 1 {
            return None;
        }
        #[cfg(windows)]
        if pid == 4 {
            return None;
        }
        if hops == 32 {
            return None;
        }
        match get_ppid_of(pid) {
            Some(p) if p != pid && p > 0 => pid = p,
            _ => return None,
        }
    }
    None
}

/// Strictly resolves the `(sessionId, claude_pid)` belonging ONLY to this tab's Claude process.
pub fn resolve_isolated_session_id(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
) -> (String, u32) {
    let cdir = cache_dir(home);
    let shell_pid = if shell_pid_arg > 0 {
        shell_pid_arg
    } else if !tab_yazi_id.is_empty() {
        fs::read_to_string(cdir.join(format!("tab_shell_{}.txt", tab_yazi_id)))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0)
    } else {
        0
    };

    let canon_target = clean_canonicalize(target_cwd).unwrap_or_else(|_| PathBuf::from(target_cwd));
    let sess_dir = format!("{}/.claude/sessions", home);

    struct SessEntry {
        pid: u32,
        session_id: String,
        cwd: PathBuf,
        kind: String,
        is_busy: bool,
        updated_at: u64,
        started_at: u64,
    }

    let mut alive_entries = Vec::new();
    if let Ok(rd) = fs::read_dir(&sess_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(txt) = fs::read_to_string(&p) {
                if extract_json_bool(&txt, "spare") == Some(true) {
                    continue;
                }
                let pid = extract_json_u64(&txt, "pid").unwrap_or(0) as u32;
                if !is_pid_alive(pid) {
                    continue;
                }
                let Some(sid) = extract_json_str(&txt, "sessionId") else {
                    continue;
                };
                let cwd_raw = extract_json_str(&txt, "cwd").unwrap_or_default();
                let cwd = clean_canonicalize(&cwd_raw).unwrap_or_else(|_| PathBuf::from(&cwd_raw));
                let kind = extract_json_str(&txt, "kind").unwrap_or_default();
                let status = extract_json_str(&txt, "status").unwrap_or_default();
                let started_at = extract_json_u64(&txt, "startedAt").unwrap_or(0);
                let updated_at = extract_json_u64(&txt, "updatedAt").unwrap_or(started_at);
                alive_entries.push(SessEntry {
                    pid,
                    session_id: sid,
                    cwd,
                    kind,
                    is_busy: status == "busy",
                    updated_at,
                    started_at,
                });
            }
        }
    }

    alive_entries.sort_by_key(|b| std::cmp::Reverse((b.is_busy, b.updated_at, b.started_at)));

    if shell_pid > 0 {
        if let Some((_dist, e)) = alive_entries
            .iter()
            .filter_map(|e| ancestor_distance(e.pid, shell_pid).map(|d| (d, e)))
            .min_by_key(|(d, e)| {
                (
                    *d,
                    std::cmp::Reverse((e.is_busy, e.updated_at, e.started_at)),
                )
            })
        {
            let mut chosen = e;
            if !e.is_busy {
                if let Some(bg_active) = alive_entries.iter().find(|cand| {
                    cand.kind == "bg"
                        && cand.cwd == e.cwd
                        && (cand.is_busy || cand.updated_at > e.updated_at)
                        && (shell_pid == 0 || is_descendant_of(cand.pid, shell_pid))
                }) {
                    chosen = bg_active;
                }
            }
            if !tab_yazi_id.is_empty() {
                let _ = fs::write(
                    cdir.join(format!("session_{}.yazi", chosen.session_id)),
                    tab_yazi_id,
                );
            }
            return (chosen.session_id.clone(), chosen.pid);
        }
    }

    if !tab_yazi_id.is_empty() {
        for e in &alive_entries {
            if let Ok(bound_yid) =
                fs::read_to_string(cdir.join(format!("session_{}.yazi", e.session_id)))
            {
                if bound_yid.trim() == tab_yazi_id {
                    return (e.session_id.clone(), e.pid);
                }
            }
        }
    }

    for e in &alive_entries {
        if e.cwd == canon_target {
            if !tab_yazi_id.is_empty() {
                let _ = fs::write(
                    cdir.join(format!("session_{}.yazi", e.session_id)),
                    tab_yazi_id,
                );
            }
            return (e.session_id.clone(), e.pid);
        }
    }

    (String::new(), 0)
}

/// Normalizes `file_path` so if its canonical realpath lies under `canonicalize(target_cwd)`,
/// it uses `target_cwd`'s spelling (ensuring Yazi's `reveal` matches the directory spelling).
pub fn normalize_path_under_cwd(file_path: &str, target_cwd: &str) -> String {
    if file_path.is_empty() || target_cwd.is_empty() {
        return file_path.to_string();
    }
    if let (Ok(canon_file), Ok(canon_cwd)) = (
        clean_canonicalize(file_path),
        clean_canonicalize(target_cwd),
    ) {
        if let Ok(rel) = canon_file.strip_prefix(&canon_cwd) {
            let sep = if cfg!(windows) && target_cwd.contains('\\') {
                '\\'
            } else {
                '/'
            };
            let base = target_cwd.trim_end_matches(['/', '\\']);
            let rel_norm = normalize_slashes(&rel.to_string_lossy());
            if rel_norm.is_empty() {
                return base.to_string();
            }
            let rel_native = if sep == '\\' {
                rel_norm.replace('/', "\\")
            } else {
                rel_norm
            };
            return format!("{}{}{}", base, sep, rel_native);
        }
    }
    file_path.to_string()
}

pub fn resolve_yazi_id_from_ppid(cache_dir_path: &str) -> String {
    let ppid = current_ppid();
    if ppid <= 1 {
        return String::new();
    }
    let mut best: Option<(std::time::SystemTime, String)> = None;
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
                                .unwrap_or(std::time::UNIX_EPOCH);
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
        mtime: std::time::SystemTime,
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
                .unwrap_or(std::time::UNIX_EPOCH);
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
                    if &lt_canon == cc {
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
