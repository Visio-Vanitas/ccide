//! Strict Process-Isolated Session Resolution & Config-Driven Model Loader.
//!
//! Invariants:
//! 1. **Never reads any global shared `live_latest.json`**.
//! 2. Resolves each tab's `sessionId` by matching alive PIDs in `~/.claude/sessions/*.json`
//!    against the tab's Center pane `shell_pid` via `proc_pidinfo(PROC_PIDTBSDINFO)` process
//!    ancestry, or explicit `session_<sessionId>.yazi == tab_yazi_id`, or `realpath(cwd)`.
//! 3. **Main Model**: Resolved per-session from `live_sess_<sessionId>.json` / `live_tab_<yaziId>.json`
//!    (exported by `statusline` for that exact session), falling back to `~/.claude/settings.json`.
//! 4. **Subagent Model**: Resolved strictly from `~/.claude/settings.json` (`CLAUDE_CODE_SUBAGENT_MODEL`),
//!    never from API/statusline payloads.

pub mod transcript;

use std::fs;
use std::path::PathBuf;

use crate::ffi::{getppid, is_descendant_of, is_pid_alive};
use crate::json::{extract_json_str, extract_json_u64};
use crate::tui::width::floor_char_boundary;

#[derive(Clone)]
pub struct ConfigModels {
    pub main_family: &'static str,
    pub main_model_short: String,
    pub sub_family: &'static str,
    pub sub_model_short: String,
}

pub fn classify_model_family(model_str: &str) -> &'static str {
    let low = model_str.to_ascii_lowercase();
    if low.contains("gemini") || low.contains("argon") {
        "gemini"
    } else if low.contains("deepseek") {
        "deepseek"
    } else if low.contains("gpt")
        || low.contains("o1")
        || low.contains("o3")
        || low.contains("o4")
        || low.contains("openai")
    {
        "openai"
    } else if low.contains("grok") {
        "grok"
    } else {
        "claude"
    }
}

pub fn short_model_label(raw: &str) -> String {
    let base = raw.split('[').next().unwrap_or(raw).trim();
    if base.len() <= 18 {
        base.to_string()
    } else {
        base[..floor_char_boundary(base, 18)].to_string()
    }
}

fn map_alias_with_settings(raw_model: &str, settings_content: &str) -> String {
    let m_low = raw_model.trim().to_ascii_lowercase();
    if m_low.contains("fable") {
        extract_json_str(settings_content, "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME")
            .or_else(|| extract_json_str(settings_content, "ANTHROPIC_DEFAULT_FABLE_MODEL"))
            .unwrap_or_else(|| raw_model.to_string())
    } else if m_low.starts_with("opus") || m_low == "claude-opus" {
        extract_json_str(settings_content, "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME")
            .or_else(|| extract_json_str(settings_content, "ANTHROPIC_DEFAULT_OPUS_MODEL"))
            .unwrap_or_else(|| raw_model.to_string())
    } else if m_low.starts_with("sonnet") || m_low == "claude-sonnet" {
        extract_json_str(settings_content, "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME")
            .or_else(|| extract_json_str(settings_content, "ANTHROPIC_DEFAULT_SONNET_MODEL"))
            .unwrap_or_else(|| raw_model.to_string())
    } else if m_low.starts_with("haiku") || m_low == "claude-haiku" {
        extract_json_str(settings_content, "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME")
            .or_else(|| extract_json_str(settings_content, "ANTHROPIC_DEFAULT_HAIKU_MODEL"))
            .unwrap_or_else(|| raw_model.to_string())
    } else {
        raw_model.to_string()
    }
}

/// Resolves:
/// - **Main Model** for this specific session (`live_sess_<session_id>.json`, `transcript_model`
///   from `<session_id>.jsonl`, or `~/.claude/settings.json` `"model"`).
/// - **Subagent Model** strictly from `~/.claude/settings.json` (`CLAUDE_CODE_SUBAGENT_MODEL`).
pub fn load_config_models(
    home: &str,
    session_id: &str,
    tab_yazi_id: &str,
    transcript_model: &str,
) -> ConfigModels {
    let settings_path = format!("{}/.claude/settings.json", home);
    let content = fs::read_to_string(&settings_path).unwrap_or_default();
    let cache_dir = format!("{}/.cache/cc-ide", home);

    // 1. Compare modification timestamps of `live_sess_<session_id>.json` vs `.jsonl` transcript
    //    so `/model` switches reflect within 150ms whether triggered via statusline or transcript stdout!
    let mut session_raw_model = None;
    let mut live_mtime = std::time::UNIX_EPOCH;
    if !session_id.is_empty() {
        let spath = format!("{}/live_sess_{}.json", cache_dir, session_id);
        if let Ok(meta) = fs::metadata(&spath) {
            if let Ok(mt) = meta.modified() {
                live_mtime = mt;
            }
        }
        if let Ok(stxt) = fs::read_to_string(&spath) {
            session_raw_model = extract_json_str(&stxt, "model").filter(|m| !m.is_empty());
        }
    }

    if !transcript_model.is_empty() {
        let mut jsonl_newer = session_raw_model.is_none();
        if !jsonl_newer && !session_id.is_empty() {
            let projects_dir = format!("{}/.claude/projects", home);
            if let Ok(rd) = fs::read_dir(&projects_dir) {
                for entry in rd.flatten() {
                    let jp = entry.path().join(format!("{}.jsonl", session_id));
                    if let Ok(meta) = fs::metadata(&jp) {
                        if let Ok(j_mt) = meta.modified() {
                            if j_mt > live_mtime {
                                jsonl_newer = true;
                            }
                            break;
                        }
                    }
                }
            }
        }
        if jsonl_newer {
            session_raw_model = Some(transcript_model.to_string());
        }
    }

    if session_raw_model.is_none() && !tab_yazi_id.is_empty() {
        let tpath = format!("{}/live_tab_{}.json", cache_dir, tab_yazi_id);
        if let Ok(ttxt) = fs::read_to_string(&tpath) {
            let file_sid = extract_json_str(&ttxt, "session_id").unwrap_or_default();
            if session_id.is_empty() || file_sid == session_id {
                session_raw_model = extract_json_str(&ttxt, "model").filter(|m| !m.is_empty());
            }
        }
    }

    let raw_main = session_raw_model
        .or_else(|| extract_json_str(&content, "model"))
        .unwrap_or_else(|| "fable".to_string());
    let resolved_main = map_alias_with_settings(&raw_main, &content);

    // 2. Subagent model strictly from `~/.claude/settings.json` `CLAUDE_CODE_SUBAGENT_MODEL`
    let raw_sub = extract_json_str(&content, "CLAUDE_CODE_SUBAGENT_MODEL")
        .or_else(|| extract_json_str(&content, "subagentModel"))
        .unwrap_or_else(|| resolved_main.clone());
    let resolved_sub = map_alias_with_settings(&raw_sub, &content);

    let main_family = classify_model_family(&resolved_main);
    let sub_family = classify_model_family(&resolved_sub);

    ConfigModels {
        main_family,
        main_model_short: short_model_label(&resolved_main),
        sub_family,
        sub_model_short: short_model_label(&resolved_sub),
    }
}

/// Strictly resolves the `(sessionId, claude_pid)` belonging ONLY to this tab's Claude process.
pub fn resolve_isolated_session_id(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
) -> (String, u32) {
    let cache_dir = format!("{}/.cache/cc-ide", home);
    let shell_pid = if shell_pid_arg > 0 {
        shell_pid_arg
    } else if !tab_yazi_id.is_empty() {
        fs::read_to_string(format!("{}/tab_shell_{}.txt", cache_dir, tab_yazi_id))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0)
    } else {
        0
    };

    let canon_target = fs::canonicalize(target_cwd).unwrap_or_else(|_| PathBuf::from(target_cwd));
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
                if txt.contains("\"spare\":true") {
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
                let cwd = fs::canonicalize(&cwd_raw).unwrap_or_else(|_| PathBuf::from(&cwd_raw));
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

    // Priority 1: Process tree ancestry (`claude_pid` is a descendant of this tab's `shell_pid`).
    // In Claude Code 2.1+, attaching a background PTY session (`bg-pty-host`) leaves the host CLI
    // process `idle` while the attached `"kind":"bg"` worker in the same `cwd` executes tools/tasks.
    if shell_pid > 0 {
        for e in &alive_entries {
            if is_descendant_of(e.pid, shell_pid) {
                let mut chosen = e;
                if !e.is_busy {
                    if let Some(bg_active) = alive_entries.iter().find(|cand| {
                        cand.kind == "bg"
                            && cand.cwd == e.cwd
                            && (cand.is_busy || cand.updated_at > e.updated_at)
                    }) {
                        chosen = bg_active;
                    }
                }
                if !tab_yazi_id.is_empty() {
                    let _ = fs::write(
                        format!("{}/session_{}.yazi", cache_dir, chosen.session_id),
                        tab_yazi_id,
                    );
                }
                return (chosen.session_id.clone(), chosen.pid);
            }
        }
    }

    // Priority 2: Explicit `session_<sessionId>.yazi == tab_yazi_id` binding
    if !tab_yazi_id.is_empty() {
        for e in &alive_entries {
            if let Ok(bound_yid) =
                fs::read_to_string(format!("{}/session_{}.yazi", cache_dir, e.session_id))
            {
                if bound_yid.trim() == tab_yazi_id {
                    return (e.session_id.clone(), e.pid);
                }
            }
        }
    }

    // Priority 3: Canonicalized `realpath(cwd)` match among alive sessions
    for e in &alive_entries {
        if e.cwd == canon_target {
            if !tab_yazi_id.is_empty() {
                let _ = fs::write(
                    format!("{}/session_{}.yazi", cache_dir, e.session_id),
                    tab_yazi_id,
                );
            }
            return (e.session_id.clone(), e.pid);
        }
    }

    (String::new(), 0)
}

/// Normalizes `file_path` so if its canonical realpath lies under `canonicalize(target_cwd)`,
/// it uses `target_cwd`'s spelling (e.g. `/Users/...` symlink instead of `/Volumes/External/Home/...`),
/// ensuring Yazi's `reveal` matches the directory spelling Yazi opened.
pub fn normalize_path_under_cwd(file_path: &str, target_cwd: &str) -> String {
    if file_path.is_empty() || target_cwd.is_empty() {
        return file_path.to_string();
    }
    if let (Ok(canon_file), Ok(canon_cwd)) =
        (fs::canonicalize(file_path), fs::canonicalize(target_cwd))
    {
        if let Ok(rel) = canon_file.strip_prefix(&canon_cwd) {
            let base = target_cwd.trim_end_matches('/');
            let rel_str = rel.to_string_lossy();
            if rel_str.is_empty() {
                return base.to_string();
            }
            return format!("{}/{}", base, rel_str);
        }
    }
    file_path.to_string()
}

/// Discovers `yazi_id` from the caller's parent process (`getppid()`) by matching against
/// `tab_shell_<yazi_id>.txt`, picking the **most recently modified** entry when a tab has been
/// rebalanced or reopened under the same shell PID.
pub fn resolve_yazi_id_from_ppid(cache_dir: &str) -> String {
    let ppid = unsafe { getppid() as u32 };
    if ppid <= 1 {
        return String::new();
    }
    let mut best: Option<(std::time::SystemTime, String)> = None;
    if let Ok(rd) = fs::read_dir(cache_dir) {
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

/// Resolves `(live_yazi_id, tab_cwd)` for `yazi-hook` by consulting live `sidebar_tab_*.json`
/// registrations first (verifying `is_pid_alive(sidebar_pid)` so stale `CC_YAZI_ID` env vars from
/// closed/rebalanced panes are never used, and matching `bg-pty-host` sessions via `realpath(cwd)`).
pub fn resolve_live_yazi_tab(
    cache_dir: &str,
    session_id: &str,
    cwd_path: &str,
) -> (String, String) {
    let ppid = unsafe { getppid() as u32 };
    let canon_cwd = if !cwd_path.is_empty() {
        fs::canonicalize(cwd_path).ok()
    } else {
        None
    };
    let bound_yid = if !session_id.is_empty() {
        fs::read_to_string(format!("{}/session_{}.yazi", cache_dir, session_id))
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
    if let Ok(rd) = fs::read_dir("/tmp/.cc-ide") {
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

    // 1. Direct process ancestry under a live tab's `shell_pid`
    if ppid > 1 {
        for lt in &live_tabs {
            if lt.shell_pid > 0 && is_descendant_of(ppid, lt.shell_pid) {
                return (lt.yazi_id.clone(), lt.cwd.clone());
            }
        }
    }
    // 2. Explicit `session_<sid>.yazi` pointing to an alive tab
    if !bound_yid.is_empty() {
        if let Some(lt) = live_tabs.iter().find(|t| t.yazi_id == bound_yid) {
            return (lt.yazi_id.clone(), lt.cwd.clone());
        }
    }
    // 3. Canonicalized `realpath(cwd)` match against an alive tab (handles `bg-pty-host` attached sessions)
    if let Some(ref cc) = canon_cwd {
        for lt in &live_tabs {
            if !lt.cwd.is_empty() {
                if let Ok(lt_canon) = fs::canonicalize(&lt.cwd) {
                    if &lt_canon == cc {
                        return (lt.yazi_id.clone(), lt.cwd.clone());
                    }
                }
            }
        }
    }

    // 4. Fallback when no `sidebar_tab_*.json` is running (e.g. `--no-sidebar` or cold startup)
    let ppid_yid = resolve_yazi_id_from_ppid(cache_dir);
    if !ppid_yid.is_empty() {
        return (ppid_yid, cwd_path.to_string());
    }
    let env_yid = std::env::var("CC_YAZI_ID").unwrap_or_default();
    if !env_yid.is_empty() {
        return (env_yid, cwd_path.to_string());
    }
    (bound_yid, cwd_path.to_string())
}
