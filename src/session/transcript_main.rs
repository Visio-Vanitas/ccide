//! Main-Session Transcript File-Ops Lookup & `main_last_file` Cache Merge (`src/session/transcript_main.rs`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::json::{extract_json_str, extract_json_u64};
use crate::platform::{clean_canonicalize, normalize_slashes};
use crate::session::transcript::{update_jsonl_tracker, JsonlTracker};

pub struct MainSessionOps {
    pub tool: String,
    pub file: String,
    pub line: usize,
    pub span: usize,
    pub model: String,
    pub model_mtime: SystemTime,
    pub seq: usize,
}

fn find_session_jsonl(home: &str, session_id: &str) -> Option<PathBuf> {
    let pdir = format!("{}/.claude/projects", home);
    if let Ok(rd) = fs::read_dir(&pdir) {
        for entry in rd.flatten() {
            let cand = entry.path().join(format!("{}.jsonl", session_id));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

pub fn get_main_session_file_ops(
    home: &str,
    session_id: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    main_tracker: &mut HashMap<String, JsonlTracker>,
) -> MainSessionOps {
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
            .or_insert_with(|| JsonlTracker::from_tail(find_session_jsonl(home, session_id)));
        if tr.jpath.as_ref().map(|p| !p.is_file()).unwrap_or(true) {
            tr.jpath = find_session_jsonl(home, session_id);
            tr.offset = 0;
            tr.last_model.clear();
            tr.last_model_mtime = SystemTime::UNIX_EPOCH;
        }
        if let Some(ref jp) = tr.jpath.clone() {
            if let Ok(m) = fs::metadata(jp).and_then(|m| m.modified()) {
                jsonl_mtime = m;
            }
            update_jsonl_tracker(jp, target_cwd, home, tr);
            out.tool = tr.last_tool.clone();
            out.file = tr.last_file.clone();
            out.line = tr.last_line;
            out.span = tr.last_span;
            out.model = tr.last_model.clone();
            out.model_mtime = tr.last_model_mtime;
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
