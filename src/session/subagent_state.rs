//! Subagent Session Directory, Dismissal & Parent Task-State Helpers (`src/session/subagent_state.rs`).

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

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

pub(crate) fn load_id_set(path: &str) -> HashSet<String> {
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

pub(crate) fn load_parent_task_states(parent_jsonl: &Path) -> HashMap<String, &'static str> {
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
