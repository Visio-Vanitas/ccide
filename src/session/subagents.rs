//! Session-Isolated Subagent Discovery, Per-Agent Model/Effort & `stoppedByUser` Tracking.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

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

fn load_id_set(path: &str) -> HashSet<String> {
    fs::read_to_string(path)
        .map(|t| {
            t.lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn load_dismissed_subagents(home: &str, session_id: &str) -> HashSet<String> {
    if session_id.is_empty() {
        return HashSet::new();
    }
    load_id_set(&format!(
        "{}/.cache/cc-ide/dismissed_{}.txt",
        home, session_id
    ))
}

fn load_parent_task_states(parent_jsonl: &Path) -> HashMap<String, &'static str> {
    let mut states = HashMap::new();
    let Ok(mut f) = File::open(parent_jsonl) else {
        return states;
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(524_288)));
    let mut raw = Vec::new();
    if f.read_to_end(&mut raw).is_err() {
        return states;
    }
    for line in String::from_utf8_lossy(&raw).lines() {
        let task_id = line
            .find("<task-id>")
            .and_then(|s| line[s + 9..].split("</task-id>").next())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if line.contains("was resumed by the user") {
            if let Some(id) = task_id {
                states.remove(id);
            }
            states.retain(|k: &String, _| !line.contains(k.as_str()));
            continue;
        }
        if let Some(id) = task_id {
            if line.contains("<status>killed</status>") {
                states.insert(id.to_string(), "stopped");
            } else if line.contains("<status>completed</status>")
                || line.contains("<status>failed</status>")
            {
                states.insert(id.to_string(), "done");
            }
        }
        if let Some(pos) = line.find("Successfully stopped task: ") {
            let id: String = line[pos + 27..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !id.is_empty() {
                states.insert(id, "stopped");
            }
        }
    }
    states
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
    let stopped_hook = load_id_set(&format!(
        "{}/.cache/cc-ide/stopped_{}.txt",
        home, session_id
    ));
    let parent_states = load_parent_task_states(&sess_dir.with_extension("jsonl"));

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
        let tracker_key = format!("{session_id}:{agent_id}");
        let tr = trackers.entry(tracker_key).or_insert_with(|| {
            let sz = fs::metadata(&jpath).map(|m| m.len()).unwrap_or(0);
            JsonlTracker {
                jpath: Some(jpath.clone()),
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

        let parent_st = parent_states.get(&agent_id).copied();
        let stopped_by_user =
            extract_json_bool(&mtxt, "stoppedByUser") == Some(true) || parent_st == Some("stopped");
        let is_done = extract_json_bool(&mtxt, "finished") == Some(true)
            || stopped_by_user
            || stopped_hook.contains(&agent_id)
            || parent_st == Some("done")
            || tr.done_from_tail;
        let status = if stopped_by_user {
            "stopped"
        } else if is_done {
            "done"
        } else {
            "running"
        };
        if status != "running" && dismissed.contains(&agent_id) {
            continue;
        }

        let raw_type =
            extract_json_str(&mtxt, "agentType").unwrap_or_else(|| "general-purpose".to_string());
        let shim_resolved = crate::orchestrator::model_from_shim_agent_type(home, &raw_type);
        let meta_model = extract_json_str(&mtxt, "model")
            .filter(|m| is_valid_model_name(m))
            .or_else(|| {
                extract_json_str(&mtxt, "pickedEffortModel").filter(|m| is_valid_model_name(m))
            })
            .or_else(|| shim_resolved.as_ref().map(|(_, m)| m.clone()))
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

        let label_src = extract_json_str(&mtxt, "name")
            .filter(|s| !s.is_empty())
            .or_else(|| shim_resolved.map(|(badge, _)| badge.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_stop_killed_and_resumed_transitions_status() {
        let base = std::env::temp_dir().join(format!("ccide_subagent_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let sid = "sess-123";
        let proj = base.join(".claude/projects/proj");
        let sub_dir = proj.join(sid).join("subagents");
        fs::create_dir_all(&sub_dir).unwrap();
        fs::write(
            sub_dir.join("agent-abc.meta.json"),
            r#"{"agentType":"Explore"}"#,
        )
        .unwrap();
        let parent = proj.join(format!("{}.jsonl", sid));
        fs::write(&parent, "<task-id>abc</task-id><status>killed</status>\n").unwrap();

        let home = base.to_string_lossy().into_owned();
        let mut trackers = HashMap::new();
        let items = get_session_subagents(&home, sid, "/tmp", &mut trackers);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, "stopped");

        let mut f = OpenOptions::new().append(true).open(&parent).unwrap();
        writeln!(f, "<task-id>abc</task-id> was resumed by the user").unwrap();
        let items2 = get_session_subagents(&home, sid, "/tmp", &mut trackers);
        assert_eq!(items2.len(), 1);
        assert_eq!(items2[0].status, "running");
        let _ = fs::remove_dir_all(&base);
    }
}
