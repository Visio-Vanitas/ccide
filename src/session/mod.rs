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
use crate::tui::{BLUE, CORAL, CYAN, LAVENDER, MINT};

#[derive(Clone)]
pub struct ConfigModels {
    pub main_family: &'static str,
    pub main_accent: &'static str,
    pub main_model_short: String,
    pub sub_family: &'static str,
    pub sub_accent: &'static str,
    pub sub_model_short: String,
}

pub fn classify_model_family(model_str: &str) -> (&'static str, &'static str) {
    let low = model_str.to_ascii_lowercase();
    if low.contains("gemini") || low.contains("argon") {
        ("gemini", BLUE)
    } else if low.contains("deepseek") {
        ("deepseek", CYAN)
    } else if low.contains("gpt")
        || low.contains("o1")
        || low.contains("o3")
        || low.contains("o4")
        || low.contains("openai")
    {
        ("openai", MINT)
    } else if low.contains("grok") {
        ("grok", LAVENDER)
    } else {
        ("claude", CORAL)
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

    let (main_family, main_accent) = classify_model_family(&resolved_main);
    let (sub_family, sub_accent) = classify_model_family(&resolved_sub);

    ConfigModels {
        main_family,
        main_accent,
        main_model_short: short_model_label(&resolved_main),
        sub_family,
        sub_accent,
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
                if extract_json_str(&txt, "kind").as_deref() == Some("bg")
                    || txt.contains("\"spare\":true")
                {
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
                let started_at = extract_json_u64(&txt, "startedAt").unwrap_or(0);
                alive_entries.push(SessEntry {
                    pid,
                    session_id: sid,
                    cwd,
                    started_at,
                });
            }
        }
    }

    alive_entries.sort_by_key(|b| std::cmp::Reverse(b.started_at));

    // Priority 1: Process tree ancestry (`claude_pid` is a descendant of this tab's `shell_pid`)
    if shell_pid > 0 {
        for e in &alive_entries {
            if is_descendant_of(e.pid, shell_pid) {
                if !tab_yazi_id.is_empty() {
                    let _ = fs::write(
                        format!("{}/session_{}.yazi", cache_dir, e.session_id),
                        tab_yazi_id,
                    );
                }
                return (e.session_id.clone(), e.pid);
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

/// Discovers `yazi_id` from the caller's parent process (`getppid()`) by matching against `tab_shell_<yazi_id>.txt`.
pub fn resolve_yazi_id_from_ppid(cache_dir: &str) -> String {
    let ppid = unsafe { getppid() as u32 };
    if ppid <= 1 {
        return String::new();
    }
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
                            return yid.to_string();
                        }
                    }
                }
            }
        }
    }
    String::new()
}
