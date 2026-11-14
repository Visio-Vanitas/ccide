//! Strict Process-Isolated Session & Tab Resolution (`src/session/mod.rs`).

pub mod focus;
pub mod model;
pub mod subagents;
pub mod transcript;

use std::fs;
use std::path::PathBuf;

use crate::ffi::{current_ppid, get_ppid_of, is_descendant_of, is_pid_alive};
use crate::json::{extract_json_bool, extract_json_str, extract_json_u64};
use crate::platform::{
    cache_dir, clean_canonicalize, find_executable, ipc_dir, ipc_path, normalize_slashes,
};

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionBinding {
    pub session_id: String,
    pub ui_pid: u32,
    pub data_pid: u32,
}

/// Strips terminal job suffixes (` (claude)`, ` (node)`) and leading spinner/status glyphs
/// (`✳ `, `◐ `, `● `) from a Center pane title while preserving ASCII punctuation (`[v0.9]`).
pub fn clean_claude_pane_title(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if s.ends_with(')') {
        if let Some(idx) = s.rfind(" (") {
            let job = s[idx + 2..s.len() - 1].trim().to_ascii_lowercase();
            if job.starts_with("claude") || job.starts_with("node") {
                s = s[..idx].trim_end();
            }
        }
    }
    let cleaned = s
        .trim_start_matches(|c: char| c.is_whitespace() || (!c.is_ascii() && !c.is_alphanumeric()))
        .trim();
    if cleaned.is_empty()
        || cleaned.eq_ignore_ascii_case("claude")
        || cleaned.eq_ignore_ascii_case("claude code")
    {
        return None;
    }
    Some(cleaned.to_string())
}

fn query_tmux_pane_title_bounded(tmux_bin: &std::path::Path, center_pane: &str) -> Option<String> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child = Command::new(tmux_bin)
        .args([
            "-L",
            "ccide",
            "display-message",
            "-p",
            "-t",
            center_pane,
            "#{pane_title}",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_millis(150);
    loop {
        if let Ok(Some(st)) = child.try_wait() {
            if !st.success() {
                return None;
            }
            let mut out = String::new();
            use std::io::Read;
            if let Some(mut so) = child.stdout.take() {
                let _ = so.read_to_string(&mut out);
            }
            return clean_claude_pane_title(&out);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read_tab_pane_title(home: &str, tab_yazi_id: &str, query_tmux: bool) -> Option<String> {
    if tab_yazi_id.is_empty() {
        return None;
    }
    let spec =
        fs::read_to_string(ipc_path(&format!("panes_{tab_yazi_id}.txt"))).unwrap_or_default();
    if query_tmux {
        if let Some(rest) = spec.trim().strip_prefix("tmux:") {
            let center_pane = rest.split(':').next().unwrap_or("");
            if !center_pane.is_empty() {
                if let Some(tmux) = find_executable("tmux", home) {
                    return query_tmux_pane_title_bounded(&tmux, center_pane);
                }
            }
        }
    }
    if !spec.trim().starts_with("tmux:") {
        if let Ok(raw) = fs::read_to_string(ipc_path(&format!("tab_title_{tab_yazi_id}.txt"))) {
            if let Some(t) = clean_claude_pane_title(&raw) {
                return Some(t);
            }
        }
    }
    None
}

fn is_claimed_by_live_sibling(cdir: &std::path::Path, session_id: &str, my_yid: &str) -> bool {
    if session_id.is_empty() {
        return false;
    }
    let Ok(raw) = fs::read_to_string(cdir.join(format!("session_{session_id}.yazi"))) else {
        return false;
    };
    let other_yid = raw.trim();
    if other_yid.is_empty() || other_yid == my_yid {
        return false;
    }
    let Ok(reg) = fs::read_to_string(ipc_path(&format!("sidebar_tab_{other_yid}.json"))) else {
        return false;
    };
    let pid = extract_json_u64(&reg, "pid").unwrap_or(0) as u32;
    is_pid_alive(pid)
}

/// Strictly resolves `SessionBinding { session_id, ui_pid, data_pid }` for this tab's Claude
/// process, preserving the interactive viewer `ui_pid` (`e.pid`) for `SIGUSR2` view-switching
/// while tracking the attached `bg` session's `session_id` and `data_pid` under FleetView.
pub fn resolve_isolated_session_binding(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
) -> SessionBinding {
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
        name: String,
        job_id: String,
        parked_job_id: String,
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
                let name = extract_json_str(&txt, "name").unwrap_or_default();
                let job_id = extract_json_str(&txt, "jobId").unwrap_or_default();
                let parked_job_id = extract_json_str(&txt, "parkedJobId").unwrap_or_default();
                let status = extract_json_str(&txt, "status").unwrap_or_default();
                let started_at = extract_json_u64(&txt, "startedAt").unwrap_or(0);
                let updated_at = extract_json_u64(&txt, "updatedAt").unwrap_or(started_at);
                alive_entries.push(SessEntry {
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
                });
            }
        }
    }

    alive_entries.sort_by_key(|b| std::cmp::Reverse((b.is_busy, b.updated_at, b.started_at)));

    let matches_title = |cand: &SessEntry, title: &str| -> bool {
        cand.name == title
            || (!cand.job_id.is_empty() && cand.job_id == title)
            || (title.len() >= 8 && cand.session_id.starts_with(title))
    };

    if shell_pid > 0 {
        if let Some((_dist, e)) = alive_entries
            .iter()
            .filter_map(|e| ancestor_distance(e.pid, shell_pid).map(|d| (d, e)))
            .min_by_key(|(d, e)| {
                (
                    *d,
                    !e.parked_job_id.is_empty(),
                    std::cmp::Reverse((e.is_busy, e.updated_at, e.started_at)),
                )
            })
        {
            let mut chosen = e;
            let is_viewer = !e.parked_job_id.is_empty();
            if let Some(title) = read_tab_pane_title(home, tab_yazi_id, is_viewer) {
                if !(e.parked_job_id.is_empty() && matches_title(e, &title)) {
                    if let Some(bg_same_cwd) = alive_entries
                        .iter()
                        .find(|c| {
                            c.kind == "bg"
                                && c.cwd == e.cwd
                                && matches_title(c, &title)
                                && !is_claimed_by_live_sibling(&cdir, &c.session_id, tab_yazi_id)
                        })
                        .or_else(|| {
                            alive_entries.iter().find(|c| {
                                c.kind == "bg" && c.cwd == e.cwd && matches_title(c, &title)
                            })
                        })
                    {
                        chosen = bg_same_cwd;
                    } else if is_viewer {
                        if let Some(bg_any) = alive_entries
                            .iter()
                            .find(|c| {
                                c.kind == "bg"
                                    && matches_title(c, &title)
                                    && !is_claimed_by_live_sibling(
                                        &cdir,
                                        &c.session_id,
                                        tab_yazi_id,
                                    )
                            })
                            .or_else(|| {
                                alive_entries
                                    .iter()
                                    .find(|c| c.kind == "bg" && matches_title(c, &title))
                            })
                        {
                            chosen = bg_any;
                        }
                    }
                }
            }
            if chosen.pid == e.pid && is_viewer {
                if let Some(bg_fb) = alive_entries.iter().find(|c| {
                    c.kind == "bg"
                        && c.cwd == e.cwd
                        && !is_claimed_by_live_sibling(&cdir, &c.session_id, tab_yazi_id)
                }) {
                    chosen = bg_fb;
                }
            }
            if !tab_yazi_id.is_empty() {
                let _ = fs::write(
                    cdir.join(format!("session_{}.yazi", chosen.session_id)),
                    tab_yazi_id,
                );
            }
            return SessionBinding {
                session_id: chosen.session_id.clone(),
                ui_pid: e.pid,
                data_pid: chosen.pid,
            };
        }

        if is_pid_alive(shell_pid) {
            if let Some(title) = read_tab_pane_title(home, tab_yazi_id, false) {
                if let Some(bg) = alive_entries.iter().find(|c| {
                    c.kind == "bg"
                        && c.cwd == canon_target
                        && matches_title(c, &title)
                        && !is_claimed_by_live_sibling(&cdir, &c.session_id, tab_yazi_id)
                }) {
                    return SessionBinding {
                        session_id: bg.session_id.clone(),
                        ui_pid: bg.pid,
                        data_pid: bg.pid,
                    };
                }
            }
            return SessionBinding::default();
        }
    }

    if !tab_yazi_id.is_empty() {
        for e in &alive_entries {
            if e.parked_job_id.is_empty() {
                if let Ok(bound_yid) =
                    fs::read_to_string(cdir.join(format!("session_{}.yazi", e.session_id)))
                {
                    if bound_yid.trim() == tab_yazi_id {
                        return SessionBinding {
                            session_id: e.session_id.clone(),
                            ui_pid: e.pid,
                            data_pid: e.pid,
                        };
                    }
                }
            }
        }
    }

    for e in &alive_entries {
        if e.cwd == canon_target
            && e.parked_job_id.is_empty()
            && !is_claimed_by_live_sibling(&cdir, &e.session_id, tab_yazi_id)
        {
            if !tab_yazi_id.is_empty() {
                let _ = fs::write(
                    cdir.join(format!("session_{}.yazi", e.session_id)),
                    tab_yazi_id,
                );
            }
            return SessionBinding {
                session_id: e.session_id.clone(),
                ui_pid: e.pid,
                data_pid: e.pid,
            };
        }
    }

    SessionBinding::default()
}

pub fn resolve_isolated_session_id(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
) -> (String, u32) {
    let b = resolve_isolated_session_binding(home, tab_yazi_id, target_cwd, shell_pid_arg);
    (b.session_id, b.ui_pid)
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

#[cfg(test)]
mod tests {
    use super::clean_claude_pane_title;

    #[test]
    fn cleans_iterm_and_spinner_pane_titles_accurately() {
        assert_eq!(
            clean_claude_pane_title("palette-syntax-ghostty-support (claude)"),
            Some("palette-syntax-ghostty-support".to_string())
        );
        assert_eq!(
            clean_claude_pane_title("✳ high-concurrency-optimization (claude)"),
            Some("high-concurrency-optimization".to_string())
        );
        assert_eq!(
            clean_claude_pane_title("✳ agent connection test (claude)"),
            Some("agent connection test".to_string())
        );
        assert_eq!(
            clean_claude_pane_title("◐ 并发优化测试 (claude.exe)"),
            Some("并发优化测试".to_string())
        );
        assert_eq!(
            clean_claude_pane_title("✳ [v0.10] .github fix (claude)"),
            Some("[v0.10] .github fix".to_string())
        );
        assert_eq!(clean_claude_pane_title("Claude Code (claude)"), None);
        assert_eq!(clean_claude_pane_title("claude"), None);
    }
}
