//! Session-Isolated Subagent Discovery, Per-Agent Model/Effort & `stoppedByUser` Tracking.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::json::{extract_json_bool, extract_json_str};
use crate::session::transcript::{update_jsonl_tracker, JsonlTracker};
use crate::session::{is_valid_model_name, map_alias_with_settings, short_model_label};

#[derive(Clone)]
pub struct SubagentItem {
    pub full_id: String,
    pub raw_type: String,
    pub short_type: String,
    pub desc: String,
    pub status: &'static str,
    pub model_raw: Option<String>,
    pub model_short: Option<String>,
    pub effort: Option<String>,
    pub tool: String,
    pub file: String,
    pub line: usize,
    pub span: usize,
    pub file_op_seq: usize,
    pub ctime: u64,
    pub footer_rank: usize,
}

pub fn find_session_dir(home: &str, session_id: &str) -> Option<PathBuf> {
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

    let settings_txt =
        fs::read_to_string(format!("{}/.claude/settings.json", home)).unwrap_or_default();
    let dismissed = load_dismissed_subagents(home, session_id);
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
        if extract_json_bool(&mtxt, "isObserver") == Some(true) {
            continue;
        }
        let fname = mpath.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let agent_id = fname
            .strip_prefix("agent-")
            .and_then(|s| s.strip_suffix(".meta.json"))
            .unwrap_or("")
            .to_string();
        if agent_id.is_empty() {
            continue;
        }

        let agent_cwd = extract_json_str(&mtxt, "worktreePath")
            .or_else(|| extract_json_str(&mtxt, "cwd"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| target_cwd.to_string());

        let jpath = mpath.with_file_name(format!("agent-{}.jsonl", agent_id));
        let tr = trackers.entry(agent_id.clone()).or_insert_with(|| {
            let sz = fs::metadata(&jpath).map(|m| m.len()).unwrap_or(0);
            JsonlTracker {
                jpath: None,
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
        update_jsonl_tracker(&jpath, &agent_cwd, home, tr);

        let j_mtime_s = fs::metadata(&jpath)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(mtime_ms / 1000);
        let age_s = now_s.saturating_sub(j_mtime_s);

        let stopped_by_user = extract_json_bool(&mtxt, "stoppedByUser") == Some(true);
        let is_done = extract_json_bool(&mtxt, "finished") == Some(true)
            || stopped_by_user
            || tr.done_from_tail
            || age_s > 25;
        if is_done && dismissed.contains(&agent_id) {
            continue;
        }
        let status = if stopped_by_user {
            "stopped"
        } else if is_done {
            "done"
        } else {
            "running"
        };

        let meta_model = extract_json_str(&mtxt, "model")
            .filter(|m| is_valid_model_name(m))
            .or_else(|| {
                extract_json_str(&mtxt, "pickedEffortModel").filter(|m| is_valid_model_name(m))
            })
            .or_else(|| {
                (!tr.last_model.is_empty() && is_valid_model_name(&tr.last_model))
                    .then(|| tr.last_model.clone())
            });
        let (model_raw, model_short) = if let Some(rm) = meta_model {
            let mapped = map_alias_with_settings(&rm, &settings_txt);
            let sh = short_model_label(&mapped);
            (Some(mapped), Some(sh))
        } else {
            (None, None)
        };
        let effort = extract_json_str(&mtxt, "effort").filter(|e| !e.is_empty());

        let raw_type =
            extract_json_str(&mtxt, "agentType").unwrap_or_else(|| "general-purpose".to_string());
        let label_src = extract_json_str(&mtxt, "name")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| raw_type.rsplit(':').next().unwrap_or(&raw_type).to_string());
        let short_type: String = label_src.chars().take(12).collect();
        let desc =
            extract_json_str(&mtxt, "description").unwrap_or_else(|| "Subagent task".to_string());

        items.push(SubagentItem {
            full_id: agent_id,
            raw_type,
            short_type,
            desc,
            status,
            model_raw,
            model_short,
            effort,
            tool: tr.last_tool.clone(),
            file: tr.last_file.clone(),
            line: tr.last_line,
            span: tr.last_span,
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
