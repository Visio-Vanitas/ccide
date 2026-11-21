//! Antigravity Native Swarm State & Member Lifecycle (`src/session/agy_swarm.rs`).

#[path = "agy_swarm_io.rs"]
mod io;

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::json::{extract_json_bool, extract_json_object, extract_json_str, iter_object_members};
use crate::session::agent::is_safe_id;
use crate::session::agy::{agy_main_file_ops, agy_subagents};
use crate::session::is_valid_model_name;
use crate::session::model::short_model_label;
use crate::session::subagent_state::load_dismissed_subagents;
use crate::session::subagents::{is_subagent_alive, SubagentItem};
use crate::session::transcript::JsonlTracker;

#[allow(unused_imports)]
pub use io::{
    agy_brain_root, agy_brain_roots, agy_is_member_of, agy_parent_coordinator, agy_resolve_cid,
    agy_resolve_cid_detail, agy_sys_gen, agy_transcript_for_cid, read_board_tail,
};
use io::{read_members_cached, sanitize_line};

#[derive(Clone, Default)]
pub struct AgySwarmSnapshot {
    pub active: bool,
    pub title: String,
    pub board_head: u64,
    pub coordinator_model: Option<String>,
    pub members: Vec<SubagentItem>,
    pub worker_transcripts: Vec<(String, PathBuf, String)>,
    pub board_latest: Vec<(String, u64, String)>,
}

pub fn swarm_member_status(member_json: &str) -> &'static str {
    let killed = extract_json_bool(member_json, "killed") == Some(true);
    let phase = extract_json_str(member_json, "phase").unwrap_or_default();
    let desired = extract_json_str(member_json, "desired_state").unwrap_or_default();
    if killed || phase == "KILLED" || desired == "KILLED" {
        return "killed";
    }
    if phase != "RUNNING" {
        return "stopped";
    }
    match extract_json_str(member_json, "wake_mode").as_deref() {
        Some("working") => "running",
        _ => "waiting",
    }
}

fn pick_model(json: &str) -> Option<String> {
    ["applied_model", "desired_model", "model"]
        .into_iter()
        .find_map(|k| extract_json_str(json, k).filter(|s| is_valid_model_name(s)))
}

fn build_item(
    cid: String,
    desc: String,
    raw_type: String,
    status: &'static str,
    model_raw: Option<String>,
    br: &Path,
    trs: &mut HashMap<String, JsonlTracker>,
) -> (SubagentItem, PathBuf) {
    let wt = br
        .join(&cid)
        .join(".system_generated/logs/transcript.jsonl");
    let desc = sanitize_line(&desc, 64, false);
    let raw_type = sanitize_line(&raw_type, 64, false);
    let mut it = SubagentItem {
        full_id: cid.clone(),
        short_type: raw_type.chars().take(12).collect(),
        raw_type,
        desc,
        status,
        model_short: model_raw.as_deref().map(short_model_label),
        model_raw,
        ..SubagentItem::default()
    };
    if let Ok(meta) = fs::metadata(&wt) {
        it.ctime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        let tr = trs
            .entry(cid)
            .or_insert_with(|| JsonlTracker::from_tail(Some(wt.clone())));
        if let Some(ops) = agy_main_file_ops(&wt, tr) {
            (it.tool, it.file, it.line, it.span, it.file_op_seq) =
                (ops.tool, ops.file, ops.line, ops.span, ops.seq);
        }
    }
    (it, wt)
}

fn append_plain_subagents(
    sub_dir: &Path,
    br: &Path,
    seen: &mut HashSet<String>,
    items: &mut Vec<SubagentItem>,
    workers: &mut Vec<(String, PathBuf, String)>,
    trs: &mut HashMap<String, JsonlTracker>,
) {
    for entry in fs::read_dir(sub_dir)
        .into_iter()
        .flatten()
        .flatten()
        .take(64)
    {
        let p = entry.path();
        let ok = p.extension() == Some(OsStr::new("json"))
            && fs::metadata(&p).ok().is_some_and(|m| m.len() <= 65_536);
        let Some(txt) = ok.then(|| fs::read_to_string(&p).ok()).flatten() else {
            continue;
        };
        let Some(cid) = extract_json_str(&txt, "conversationId").filter(|s| is_safe_id(s)) else {
            continue;
        };
        if !seen.insert(cid.clone()) {
            continue;
        }
        let state = extract_json_str(&txt, "state").or_else(|| extract_json_str(&txt, "status"));
        let st = match state.as_deref() {
            Some("SUBAGENT_STATE_ALIVE") => "running",
            Some("SUBAGENT_STATE_KILLED") => "killed",
            _ => "stopped",
        };
        let d = extract_json_object(&txt, "subagentDescriptor").unwrap_or(&txt);
        let g = |k| {
            extract_json_str(d, k)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "subagent".into())
        };
        let (it, wt) = build_item(cid.clone(), g("role"), g("typeName"), st, None, br, trs);
        workers.push((cid, wt, String::new()));
        items.push(it);
    }
}

pub fn load_agy_swarm(
    home: &str,
    main_sid: &str,
    transcript: &Path,
    sub_trackers: &mut HashMap<String, JsonlTracker>,
    fallback_tr: &mut JsonlTracker,
) -> AgySwarmSnapshot {
    let mut snap = AgySwarmSnapshot::default();
    let (Some(sg), Some(br)) = (agy_sys_gen(transcript), agy_brain_root(transcript)) else {
        snap.members = agy_subagents(transcript, fallback_tr);
        return snap;
    };
    let (mut seen, mut items) = (HashSet::new(), Vec::new());
    let has_members = if let Some(txt) = read_members_cached(&sg.join("swarm/members.json")) {
        let t = extract_json_str(&txt, "title").unwrap_or_default();
        snap.title = sanitize_line(&t, 60, false);
        for (name, mj) in extract_json_object(&txt, "members")
            .map(iter_object_members)
            .unwrap_or_default()
        {
            let role = extract_json_str(mj, "role").unwrap_or_default();
            if name == "coordinator" || role == "coordinator" {
                if let Some(m) = pick_model(mj) {
                    snap.coordinator_model = Some(m);
                }
                continue;
            }
            let Some(cid) = extract_json_str(mj, "conversation_id").filter(|s| is_safe_id(s))
            else {
                continue;
            };
            if !seen.insert(cid.clone()) {
                continue;
            }
            let model = pick_model(mj);
            let rt = if role.is_empty() {
                "worker".into()
            } else {
                role
            };
            let st = swarm_member_status(mj);
            let (it, wt) = build_item(cid.clone(), name, rt, st, model.clone(), &br, sub_trackers);
            snap.worker_transcripts
                .push((cid, wt, model.unwrap_or_default()));
            items.push(it);
        }
        true
    } else {
        false
    };
    snap.active = has_members && !items.is_empty();
    if snap.active {
        let bpath = sg.join("swarm/board.jsonl");
        (snap.board_head, snap.board_latest) = read_board_tail(&bpath, 65_536);
    }
    let sub_dir = sg.join("subagents");
    let has_sub = sub_dir.is_dir();
    if has_sub {
        append_plain_subagents(
            &sub_dir,
            &br,
            &mut seen,
            &mut items,
            &mut snap.worker_transcripts,
            sub_trackers,
        );
    }
    sub_trackers.retain(|k, _| seen.contains(k));
    if !has_members && !has_sub {
        items = agy_subagents(transcript, fallback_tr);
    }
    let dismissed = load_dismissed_subagents(home, main_sid);
    items.retain(|it| is_subagent_alive(it.status) || !dismissed.contains(&it.full_id));
    let rank = |s: &str| match s {
        "running" => 0,
        "waiting" => 1,
        "stopped" => 2,
        _ => 3,
    };
    items.sort_by(|a, b| {
        rank(a.status)
            .cmp(&rank(b.status))
            .then_with(|| a.desc.cmp(&b.desc))
            .then_with(|| a.full_id.cmp(&b.full_id))
    });
    for (idx, it) in items.iter_mut().enumerate() {
        it.footer_rank = idx + 1;
    }
    snap.members = items;
    snap
}

#[cfg(test)]
#[path = "agy_swarm_tests.rs"]
mod tests;
