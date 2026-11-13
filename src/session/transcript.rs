//! Incremental Offset-Cached `.jsonl` Transcript Parser for Main & Subagent Sessions.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::json::{extract_json_str, extract_json_u64};
use crate::session::{extract_tool_line_target, is_valid_model_name};
use crate::tui::width::floor_char_boundary;

pub use crate::session::subagents::{
    append_dismissed_subagents, get_session_subagents, SubagentItem,
};

pub struct JsonlTracker {
    pub jpath: Option<PathBuf>,
    pub offset: u64,
    pub last_tool: String,
    pub last_file: String,
    pub last_line: usize,
    pub last_span: usize,
    pub last_model: String,
    pub seq: usize,
    pub done_from_tail: bool,
}

pub struct MainSessionOps {
    pub tool: String,
    pub file: String,
    pub line: usize,
    pub span: usize,
    pub model: String,
    pub model_mtime: SystemTime,
    pub seq: usize,
}

fn resolve_valid_project_file(
    cand: &str,
    target_cwd: &str,
    claude_projects_prefix: &str,
) -> Option<String> {
    if cand.is_empty() {
        return None;
    }
    let abs = if Path::new(cand).is_absolute() {
        cand.to_string()
    } else {
        format!("{}/{}", target_cwd.trim_end_matches('/'), cand)
    };
    if abs.starts_with(claude_projects_prefix)
        || abs.contains("/tmp/claude-")
        || abs.contains("/.cache/cc-ide/")
        || abs.contains("/.lib/")
    {
        return None;
    }
    if (abs.starts_with("/tmp/") || abs.starts_with("/private/tmp/"))
        && !target_cwd.starts_with("/tmp")
        && !target_cwd.starts_with("/private/tmp")
    {
        let stripped = abs
            .strip_prefix("/private/tmp/")
            .or_else(|| abs.strip_prefix("/tmp/"))
            .unwrap_or("");
        if let Some((_wt, rel)) = stripped.split_once('/') {
            let alt = format!("{}/{}", target_cwd.trim_end_matches('/'), rel);
            if Path::new(&alt).exists() {
                return Some(alt);
            }
        }
        return None;
    }
    Path::new(&abs).exists().then_some(abs)
}

fn parse_jsonl_line_for_tools(
    line: &str,
    target_cwd: &str,
    claude_projects_prefix: &str,
    tr: &mut JsonlTracker,
) {
    if !line.contains("\"tool_use\"") || !line.contains("\"assistant\"") {
        return;
    }
    let mut search_pos = 0;
    while let Some(rel) = line[search_pos..].find("\"type\":\"tool_use\"") {
        let start = search_pos + rel;
        search_pos = start + 17;
        let next_tool = line[search_pos..]
            .find("\"type\":\"tool_use\"")
            .map(|r| search_pos + r)
            .unwrap_or(line.len());
        let slice_end = floor_char_boundary(line, next_tool);
        let chunk = &line[start..slice_end];
        if let Some(tname) = extract_json_str(chunk, "name") {
            if tr.last_file.is_empty() {
                tr.last_tool = tname.clone();
            }
            if matches!(
                tname.as_str(),
                "Read"
                    | "Edit"
                    | "Update"
                    | "FileEdit"
                    | "StrReplace"
                    | "Write"
                    | "MultiEdit"
                    | "NotebookEdit"
                    | "LSP"
                    | "Grep"
                    | "Glob"
            ) {
                let cand = extract_json_str(chunk, "file_path")
                    .or_else(|| extract_json_str(chunk, "filePath"))
                    .or_else(|| extract_json_str(chunk, "notebook_path"))
                    .or_else(|| extract_json_str(chunk, "path"))
                    .unwrap_or_default();
                if let Some(valid) =
                    resolve_valid_project_file(&cand, target_cwd, claude_projects_prefix)
                {
                    if Path::new(&valid).is_file() {
                        let (ln, sp) = extract_tool_line_target(chunk, &tname, &valid);
                        tr.last_file = valid;
                        tr.last_tool = tname;
                        tr.last_line = ln;
                        tr.last_span = sp;
                        tr.seq += 1;
                    }
                }
            }
        }
    }
}

pub fn update_jsonl_tracker(
    jpath: &Path,
    target_cwd: &str,
    claude_projects_prefix: &str,
    tracker: &mut JsonlTracker,
) {
    let Ok(meta) = fs::metadata(jpath) else {
        return;
    };
    let len = meta.len();
    if len < tracker.offset {
        tracker.offset = 0;
    }
    if len == tracker.offset {
        return;
    }
    let Ok(mut f) = File::open(jpath) else {
        return;
    };
    let _ = f.seek(SeekFrom::Start(tracker.offset));
    let mut raw_bytes = Vec::new();
    if f.read_to_end(&mut raw_bytes).is_ok() {
        let buf = String::from_utf8_lossy(&raw_bytes);
        for line in buf.lines() {
            if let Some(pos) = line.find("Set model to ") {
                let rest = &line[pos + 13..];
                let cleaned = rest.trim_start_matches("\\u0060").trim_start_matches('`');
                let m: String = cleaned
                    .chars()
                    .take_while(|&c| c != '`' && c != '\\' && c != ' ' && c != '<' && c != '"')
                    .collect();
                if is_valid_model_name(&m) {
                    tracker.last_model = m;
                }
            } else if line.contains("\"role\":\"assistant\"")
                || line.contains("\"type\":\"message\"")
            {
                if let Some(m) = extract_json_str(line, "model") {
                    if is_valid_model_name(&m) {
                        tracker.last_model = m;
                    }
                }
            }

            parse_jsonl_line_for_tools(line, target_cwd, claude_projects_prefix, tracker);
            if line.contains("\"stop_reason\":\"end_turn\"") {
                tracker.done_from_tail = true;
            }
        }
        tracker.offset = len;
    }
}

pub fn get_main_session_file_ops(
    home: &str,
    session_id: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    main_tracker: &mut HashMap<String, JsonlTracker>,
) -> MainSessionOps {
    let claude_projects_prefix = format!("{}/.claude/projects/", home);
    let mut out = MainSessionOps {
        tool: String::new(),
        file: String::new(),
        line: 0,
        span: 0,
        model: String::new(),
        model_mtime: SystemTime::UNIX_EPOCH,
        seq: 0,
    };
    let mut jsonl_mtime = SystemTime::UNIX_EPOCH;

    if !session_id.is_empty() {
        let tr = main_tracker
            .entry(session_id.to_string())
            .or_insert_with(|| {
                let mut jp = None;
                let pdir = format!("{}/.claude/projects", home);
                if let Ok(rd) = fs::read_dir(&pdir) {
                    for entry in rd.flatten() {
                        let cand = entry.path().join(format!("{}.jsonl", session_id));
                        if cand.is_file() {
                            jp = Some(cand);
                            break;
                        }
                    }
                }
                let sz = jp
                    .as_ref()
                    .and_then(|p| fs::metadata(p).ok())
                    .map(|m| m.len())
                    .unwrap_or(0);
                JsonlTracker {
                    jpath: jp,
                    offset: sz.saturating_sub(262_144),
                    last_tool: String::new(),
                    last_file: String::new(),
                    last_line: 0,
                    last_span: 0,
                    last_model: String::new(),
                    seq: 0,
                    done_from_tail: false,
                }
            });
        if let Some(ref jp) = tr.jpath.clone() {
            if let Ok(m) = fs::metadata(jp).and_then(|m| m.modified()) {
                jsonl_mtime = m;
            }
            update_jsonl_tracker(jp, target_cwd, &claude_projects_prefix, tr);
            out.tool = tr.last_tool.clone();
            out.file = tr.last_file.clone();
            out.line = tr.last_line;
            out.span = tr.last_span;
            out.model = tr.last_model.clone();
            out.model_mtime = jsonl_mtime;
            out.seq = tr.seq;
        }
    }

    if !tab_yazi_id.is_empty() {
        let cpath = format!("{}/.cache/cc-ide/main_last_file_{}.json", home, tab_yazi_id);
        let cache_mtime = fs::metadata(&cpath)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if cache_mtime >= jsonl_mtime || out.file.is_empty() {
            if let Ok(txt) = fs::read_to_string(&cpath) {
                let fp = extract_json_str(&txt, "file").unwrap_or_default();
                let tl = extract_json_str(&txt, "tool").unwrap_or_else(|| "active".to_string());
                let ln = extract_json_u64(&txt, "line").unwrap_or(0) as usize;
                let sp = extract_json_u64(&txt, "span").unwrap_or(0) as usize;
                if !fp.is_empty() && Path::new(&fp).is_file() {
                    out.tool = tl;
                    out.file = fp;
                    out.line = ln;
                    out.span = sp;
                }
            }
        }
    }

    out
}
