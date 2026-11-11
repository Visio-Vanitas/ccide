//! Incremental Offset-Cached `.jsonl` Transcript Parser for Main & Subagent Sessions.
//!
//! Only seeks and reads newly appended bytes when a `.jsonl` transcript grows,
//! avoiding full-file re-reads on every TUI frame.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::json::extract_json_str;
use crate::tui::width::floor_char_boundary;

pub struct JsonlTracker {
    pub jpath: Option<PathBuf>,
    pub offset: u64,
    pub last_tool: String,
    pub last_file: String,
    pub last_model: String,
    pub seq: usize,
    pub done_from_tail: bool,
}

#[derive(Clone)]
pub struct SubagentItem {
    pub full_id: String,
    pub raw_type: String,
    pub short_type: String,
    pub desc: String,
    pub status: &'static str,
    pub tool: String,
    pub file: String,
    pub file_op_seq: usize,
    pub ctime: u64,
    pub footer_rank: usize,
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
    {
        return None;
    }
    // Check git worktree fallback (`/tmp/<worktree>/<rel_path>` -> `<target_cwd>/<rel_path>`)
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
    if Path::new(&abs).exists() {
        return Some(abs);
    }
    None
}

fn parse_jsonl_line_for_tools(
    line: &str,
    target_cwd: &str,
    claude_projects_prefix: &str,
    last_tool: &mut String,
    last_file: &mut String,
    seq: &mut usize,
) {
    if !line.contains("\"tool_use\"") || !line.contains("\"assistant\"") {
        return;
    }
    let mut search_pos = 0;
    while let Some(rel) = line[search_pos..].find("\"type\":\"tool_use\"") {
        let start = search_pos + rel;
        search_pos = start + 17;
        let slice_end = floor_char_boundary(line, (start + 900).min(line.len()));
        let chunk = &line[start..slice_end];
        if let Some(tname) = extract_json_str(chunk, "name") {
            if last_file.is_empty() {
                *last_tool = tname.clone();
            }
            if matches!(
                tname.as_str(),
                "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "Grep" | "Glob"
            ) {
                let cand = extract_json_str(chunk, "file_path")
                    .or_else(|| extract_json_str(chunk, "notebook_path"))
                    .or_else(|| extract_json_str(chunk, "path"))
                    .unwrap_or_default();
                if let Some(valid) =
                    resolve_valid_project_file(&cand, target_cwd, claude_projects_prefix)
                {
                    *last_file = valid;
                    *last_tool = tname;
                    *seq += 1;
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
            // Track `/model` command output or assistant message model in main transcript
            if let Some(pos) = line.find("Set model to ") {
                let rest = &line[pos + 13..];
                let cleaned = rest.trim_start_matches("\\u0060").trim_start_matches('`');
                let m: String = cleaned
                    .chars()
                    .take_while(|&c| c != '`' && c != '\\' && c != ' ' && c != '<' && c != '"')
                    .collect();
                if !m.is_empty() {
                    tracker.last_model = m;
                }
            } else if line.contains("\"role\":\"assistant\"")
                || line.contains("\"type\":\"message\"")
            {
                if let Some(m) = extract_json_str(line, "model") {
                    if !m.is_empty() {
                        tracker.last_model = m;
                    }
                }
            }

            parse_jsonl_line_for_tools(
                line,
                target_cwd,
                claude_projects_prefix,
                &mut tracker.last_tool,
                &mut tracker.last_file,
                &mut tracker.seq,
            );
            if line.contains("\"stop_reason\":\"end_turn\"") {
                tracker.done_from_tail = true;
            }
        }
        tracker.offset = len;
    }
}

fn find_session_dir(home: &str, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() {
        return None;
    }
    let projects_dir = format!("{}/.claude/projects", home);
    let rd = fs::read_dir(projects_dir).ok()?;
    for entry in rd.flatten() {
        let cand = entry.path().join(session_id);
        if cand.is_dir() {
            return Some(cand);
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
) -> (String, String, String) {
    let mut transcript_model = String::new();
    let mut transcript_tool = String::new();
    let mut transcript_file = String::new();

    if !session_id.is_empty() {
        let projects_dir = format!("{}/.claude/projects", home);
        let claude_projects_prefix = format!("{}/", projects_dir);
        let tr = main_tracker
            .entry(session_id.to_string())
            .or_insert_with(|| {
                let mut found_path = None;
                if let Ok(rd) = fs::read_dir(&projects_dir) {
                    for entry in rd.flatten() {
                        let cand = entry.path().join(format!("{}.jsonl", session_id));
                        if cand.is_file() {
                            found_path = Some(cand);
                            break;
                        }
                    }
                }
                let sz = found_path
                    .as_ref()
                    .and_then(|p| fs::metadata(p).ok())
                    .map(|m| m.len())
                    .unwrap_or(0);
                JsonlTracker {
                    jpath: found_path,
                    offset: sz.saturating_sub(262_144),
                    last_tool: String::new(),
                    last_file: String::new(),
                    last_model: String::new(),
                    seq: 0,
                    done_from_tail: false,
                }
            });
        if let Some(ref jp) = tr.jpath.clone() {
            update_jsonl_tracker(jp, target_cwd, &claude_projects_prefix, tr);
            transcript_tool = tr.last_tool.clone();
            transcript_file = tr.last_file.clone();
            transcript_model = tr.last_model.clone();
        }
    }

    if !tab_yazi_id.is_empty() {
        let cpath = format!("{}/.cache/cc-ide/main_last_file_{}.json", home, tab_yazi_id);
        if let Ok(txt) = fs::read_to_string(&cpath) {
            let fp = extract_json_str(&txt, "file").unwrap_or_default();
            let tl = extract_json_str(&txt, "tool").unwrap_or_else(|| "active".to_string());
            if !fp.is_empty() && Path::new(&fp).exists() {
                return (tl, fp, transcript_model);
            }
        }
    }

    (transcript_tool, transcript_file, transcript_model)
}

pub fn load_dismissed_subagents(home: &str, session_id: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    if session_id.is_empty() {
        return set;
    }
    let p = format!("{}/.cache/cc-ide/dismissed_{}.txt", home, session_id);
    if let Ok(txt) = fs::read_to_string(p) {
        for line in txt.lines() {
            let id = line.trim();
            if !id.is_empty() {
                set.insert(id.to_string());
            }
        }
    }
    set
}

pub fn append_dismissed_subagents(home: &str, session_id: &str, ids: &[String]) {
    if session_id.is_empty() || ids.is_empty() {
        return;
    }
    let dir = format!("{}/.cache/cc-ide", home);
    let _ = fs::create_dir_all(&dir);
    let existing = load_dismissed_subagents(home, session_id);
    let p = format!("{}/dismissed_{}.txt", dir, session_id);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
        for id in ids {
            if !id.is_empty() && !existing.contains(id) {
                let _ = writeln!(f, "{}", id);
            }
        }
    }
}

pub fn get_session_subagents(
    home: &str,
    session_id: &str,
    target_cwd: &str,
    trackers: &mut HashMap<String, JsonlTracker>,
) -> Vec<SubagentItem> {
    let Some(sess_dir) = find_session_dir(home, session_id) else {
        return vec![];
    };
    let sub_dir = sess_dir.join("subagents");
    let Ok(rd) = fs::read_dir(&sub_dir) else {
        return vec![];
    };

    let dismissed = load_dismissed_subagents(home, session_id);
    let claude_projects_prefix = format!("{}/.claude/projects/", home);
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut metas = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if fname.starts_with("agent-") && fname.ends_with(".meta.json") {
            let mtime = fs::metadata(&p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            metas.push((mtime, p));
        }
    }
    metas.sort_by_key(|b| std::cmp::Reverse(b.0));
    metas.truncate(64);

    let mut items = Vec::with_capacity(metas.len());
    for (mtime_ms, mpath) in metas {
        let Ok(mtxt) = fs::read_to_string(&mpath) else {
            continue;
        };
        let fname = mpath.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let agent_id = fname
            .strip_prefix("agent-")
            .and_then(|s| s.strip_suffix(".meta.json"))
            .unwrap_or("")
            .to_string();
        if agent_id.is_empty() {
            continue;
        }
        let jpath = mpath.with_file_name(format!("agent-{}.jsonl", agent_id));
        let tr = trackers.entry(agent_id.clone()).or_insert_with(|| {
            let sz = fs::metadata(&jpath).map(|m| m.len()).unwrap_or(0);
            JsonlTracker {
                jpath: None,
                offset: sz.saturating_sub(262_144),
                last_tool: String::new(),
                last_file: String::new(),
                last_model: String::new(),
                seq: 0,
                done_from_tail: false,
            }
        });
        update_jsonl_tracker(&jpath, target_cwd, &claude_projects_prefix, tr);

        let j_mtime_s = fs::metadata(&jpath)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(mtime_ms / 1000);
        let age_s = now_s.saturating_sub(j_mtime_s);

        let is_done = mtxt.contains("\"finished\"") || tr.done_from_tail || age_s > 25;
        if is_done && dismissed.contains(&agent_id) {
            continue;
        }
        let status = if is_done { "done" } else { "running" };
        let raw_type =
            extract_json_str(&mtxt, "agentType").unwrap_or_else(|| "general-purpose".to_string());
        let short_raw = raw_type.rsplit(':').next().unwrap_or(&raw_type);
        let short_type: String = short_raw.chars().take(12).collect();
        let desc =
            extract_json_str(&mtxt, "description").unwrap_or_else(|| "Subagent task".to_string());

        items.push(SubagentItem {
            full_id: agent_id,
            raw_type,
            short_type,
            desc,
            status,
            tool: tr.last_tool.clone(),
            file: tr.last_file.clone(),
            file_op_seq: tr.seq,
            ctime: mtime_ms,
            footer_rank: 1,
        });
    }

    let mut chron: Vec<(u64, String)> =
        items.iter().map(|x| (x.ctime, x.full_id.clone())).collect();
    chron.sort_by_key(|a| a.0);
    for it in &mut items {
        if let Some(pos) = chron.iter().position(|x| x.1 == it.full_id) {
            it.footer_rank = pos + 1;
        }
    }
    items
}
