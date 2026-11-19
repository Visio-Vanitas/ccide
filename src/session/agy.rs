//! Antigravity CLI (`agy`) Discovery & Transcript Ops (`src/session/agy.rs`).

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::json::{
    extract_json_array_slice, extract_json_object, extract_json_str, extract_json_u64,
    iter_array_objects, iter_object_members,
};
use crate::platform::{clean_canonicalize, normalize_slashes, paths_equal_ci};
use crate::session::agent::{agy_home, is_safe_id};
#[allow(unused_imports)]
pub use crate::session::agy_swarm::agy_transcript_for_cid;
use crate::session::is_valid_model_name;
use crate::session::model::short_model_label;
use crate::session::subagents::SubagentItem;
use crate::session::transcript::JsonlTracker;
use crate::session::transcript_main::MainSessionOps;

fn parse_last_conversations(src: &str) -> Vec<(String, String)> {
    let Some(start) = src.find('{') else {
        return vec![];
    };
    iter_object_members(&src[start..])
        .into_iter()
        .filter_map(|(key, val)| {
            let cid = if val.starts_with('"') {
                extract_json_str(&format!("{{\"v\":{val}}}"), "v")
            } else if val.starts_with('{') {
                extract_json_str(val, "conversationId")
            } else {
                None
            }?;
            (!key.is_empty() && !cid.is_empty()).then_some((key, cid))
        })
        .collect()
}

fn norm_cmp_path(p: &Path) -> String {
    let s = normalize_slashes(&p.to_string_lossy())
        .trim_end_matches('/')
        .to_string();
    if cfg!(windows) {
        s.to_ascii_lowercase()
    } else {
        s
    }
}

pub fn agy_cid_for_cwd(cwd: &str) -> Option<String> {
    let raw = fs::read_to_string(agy_home().join("cache/last_conversations.json")).ok()?;
    let canon_cwd = clean_canonicalize(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
    let cwd_cmp = norm_cmp_path(&canon_cwd);
    let mut best_anc: Option<(usize, String)> = None;
    for (k, cid) in parse_last_conversations(&raw)
        .into_iter()
        .filter(|(_, c)| is_safe_id(c))
    {
        let ck = clean_canonicalize(&k).unwrap_or_else(|_| PathBuf::from(&k));
        if paths_equal_ci(&ck, &canon_cwd) {
            return Some(cid);
        }
        let kc = norm_cmp_path(&ck);
        let is_anc = !kc.is_empty() && (kc == "/" || cwd_cmp.starts_with(&format!("{kc}/")));
        if is_anc && best_anc.as_ref().map_or(true, |(l, _)| kc.len() > *l) {
            best_anc = Some((kc.len(), cid));
        }
    }
    best_anc.map(|(_, cid)| cid)
}

pub fn agy_transcript_path(cid: &str) -> PathBuf {
    if !is_safe_id(cid) {
        return PathBuf::new();
    }
    agy_home()
        .join("brain")
        .join(cid)
        .join(".system_generated/logs/transcript.jsonl")
}

pub fn agy_settings_model() -> Option<String> {
    let txt = fs::read_to_string(agy_home().join("settings.json")).ok()?;
    extract_json_str(&txt, "model").filter(|m| is_valid_model_name(m))
}

fn map_agy_tool_call(tc: &str) -> Option<(&'static str, String, usize, usize)> {
    let name = extract_json_str(tc, "name")?;
    let args = extract_json_object(tc, "arguments").unwrap_or(tc);
    let file = ["Path", "AbsolutePath", "TargetFile"]
        .into_iter()
        .find_map(|k| extract_json_str(args, k).filter(|p| !p.is_empty()))?;
    let start = extract_json_u64(args, "StartLine").unwrap_or(0) as usize;
    let end = extract_json_u64(args, "EndLine").unwrap_or(0) as usize;
    let span = if start > 0 && end >= start {
        (end - start + 1).min(120)
    } else {
        1
    };
    let (tool, ln, sp) = match name.as_str() {
        "read_file" | "view_file" => ("Read", start, if start > 0 { span.min(6) } else { 0 }),
        "edit_file" | "replace_file_content" | "multi_replace_file_content" => {
            ("Edit", start, if start > 0 { span } else { 0 })
        }
        "write_file" | "write_to_file" => ("Write", 1, 1),
        _ => return None,
    };
    Some((tool, file, ln, sp))
}

pub fn agy_main_file_ops(path: &Path, tr: &mut JsonlTracker) -> Option<MainSessionOps> {
    let meta = fs::metadata(path).ok()?;
    let (len, file_mt) = (meta.len(), meta.modified().unwrap_or(UNIX_EPOCH));
    if len < tr.offset {
        (tr.offset, tr.seq) = (0, 0);
    }
    if len > tr.offset {
        let (mut raw, mut f) = (Vec::new(), File::open(path).ok()?);
        let _ = f.seek(SeekFrom::Start(tr.offset));
        let ok = f.read_to_end(&mut raw).is_ok();
        if let Some(nl) = ok.then(|| raw.iter().rposition(|&b| b == b'\n')).flatten() {
            for line in String::from_utf8_lossy(&raw[..=nl]).lines() {
                if !line.contains("\"MODEL\"") && !line.contains("\"model\"") {
                    continue;
                }
                if let Some(m) = extract_json_str(line, "model").filter(|s| is_valid_model_name(s))
                {
                    (tr.last_model, tr.last_model_mtime) = (m, file_mt);
                }
                if line.contains("\"MODEL\"") && line.contains("\"tool_calls\"") {
                    for tc in extract_json_array_slice(line, "tool_calls")
                        .map(iter_array_objects)
                        .unwrap_or_default()
                    {
                        if let Some((tl, fp, ln, sp)) = map_agy_tool_call(tc) {
                            (tr.last_tool, tr.last_file, tr.last_line, tr.last_span) =
                                (tl.to_string(), fp, ln, sp);
                            tr.seq += 1;
                        }
                    }
                }
            }
            tr.offset += (nl + 1) as u64;
        }
    }
    Some(MainSessionOps {
        tool: tr.last_tool.clone(),
        file: tr.last_file.clone(),
        line: tr.last_line,
        span: tr.last_span,
        model: tr.last_model.clone(),
        model_mtime: tr.last_model_mtime,
        seq: tr.seq,
    })
}

fn parse_agy_sub_call(tc: &str, idx: usize, done: bool, ctime: u64) -> Option<SubagentItem> {
    let name = extract_json_str(tc, "name").unwrap_or_default();
    let args = extract_json_object(tc, "arguments").unwrap_or(tc);
    let (raw_type, desc, model_raw) = match name.as_str() {
        "invoke_subagent" => {
            let rt = extract_json_str(args, "TypeName").unwrap_or_else(|| "subagent".into());
            let role = extract_json_str(args, "Role").unwrap_or_else(|| rt.clone());
            (
                rt,
                role,
                extract_json_str(args, "Model").filter(|s| is_valid_model_name(s)),
            )
        }
        "swarm" if extract_json_str(args, "action").as_deref() == Some("add") => {
            let nm = extract_json_str(args, "name").unwrap_or_else(|| "swarm".into());
            (
                nm.clone(),
                nm,
                extract_json_str(args, "model").filter(|s| is_valid_model_name(s)),
            )
        }
        _ => return None,
    };
    Some(SubagentItem {
        full_id: extract_json_str(tc, "id").unwrap_or_else(|| format!("agy-{idx}")),
        short_type: raw_type.chars().take(12).collect(),
        raw_type,
        desc,
        status: if done { "done" } else { "running" },
        model_short: model_raw.as_deref().map(short_model_label),
        model_raw,
        ctime,
        footer_rank: idx,
        ..SubagentItem::default()
    })
}

pub fn agy_subagents(path: &Path, tr: &mut JsonlTracker) -> Vec<SubagentItem> {
    let Ok(mut f) = File::open(path) else {
        return vec![];
    };
    let Ok(meta) = f.metadata() else {
        return vec![];
    };
    tr.offset = meta.len();
    let _ = f.seek(SeekFrom::Start(meta.len().saturating_sub(262_144)));
    let mut raw = Vec::new();
    if f.read_to_end(&mut raw).is_err() {
        return vec![];
    }
    let ctime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64);
    let mut items = Vec::new();
    for line in String::from_utf8_lossy(&raw).lines() {
        if !line.contains("\"invoke_subagent\"") && !line.contains("\"swarm\"") {
            continue;
        }
        let done = extract_json_str(line, "status").as_deref() == Some("DONE");
        for tc in extract_json_array_slice(line, "tool_calls")
            .map(iter_array_objects)
            .unwrap_or_default()
        {
            if let Some(it) = parse_agy_sub_call(tc, items.len() + 1, done, ctime) {
                items.push(it);
            }
        }
    }
    items
}

#[cfg(test)]
#[path = "agy_tests.rs"]
mod tests;
