//! Antigravity Swarm Board Tail, Brain Roots & Conversation ID Resolution.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::json::{
    extract_json_array_slice, extract_json_str, extract_json_u64, extract_string_list_in_array,
};
use crate::platform::{cache_dir, clean_canonicalize, paths_equal_ci, user_home_dir};
use crate::session::agent::{agy_home, is_safe_id};
use crate::session::agy::agy_cid_for_cwd;

pub fn agy_brain_roots() -> Vec<PathBuf> {
    let a = agy_home().join("brain");
    let b = PathBuf::from(user_home_dir()).join(".gemini/antigravity/brain");
    if b != a {
        vec![a, b]
    } else {
        vec![a]
    }
}

pub fn agy_transcript_for_cid(cid: &str) -> Option<PathBuf> {
    if !is_safe_id(cid) {
        return None;
    }
    agy_brain_roots()
        .into_iter()
        .map(|r| r.join(cid).join(".system_generated/logs/transcript.jsonl"))
        .find(|p| p.is_file())
}

pub(crate) fn sanitize_line(raw: &str, max: usize, ell: bool) -> String {
    let clean: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let joined = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut ch = joined.chars();
    let head: String = ch.by_ref().take(max).collect();
    if ell && ch.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

pub fn read_board_tail(board: &Path, max_bytes: u64) -> (u64, Vec<(String, u64, String)>) {
    let Ok(mut f) = File::open(board) else {
        return (0, vec![]);
    };
    let start = f
        .metadata()
        .map_or(0, |m| m.len())
        .saturating_sub(max_bytes);
    let mut raw = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_to_end(&mut raw).is_err() {
        return (0, vec![]);
    }
    let txt = String::from_utf8_lossy(&raw);
    let body = if start > 0 {
        txt.find('\n').map_or("", |n| &txt[n + 1..])
    } else {
        &txt
    };
    let (mut head, mut by_agent) = (0u64, HashMap::<String, (u64, String)>::new());
    for line in body.lines() {
        if let Some(seq) = extract_json_u64(line, "seq") {
            head = head.max(seq);
            if let Some(ag) = extract_json_str(line, "agent").filter(|a| !a.is_empty()) {
                let ag = sanitize_line(&ag, 64, false);
                let tx = extract_json_str(line, "text").unwrap_or_default();
                let sum = sanitize_line(&tx, 96, true);
                if by_agent.get(&ag).map_or(true, |(p, _)| seq >= *p) {
                    by_agent.insert(ag, (seq, sum));
                }
            }
        }
    }
    let mut out: Vec<_> = by_agent.into_iter().map(|(a, (s, t))| (a, s, t)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out.truncate(32);
    (head, out)
}

fn decode_file_uri(uri: &str) -> Option<String> {
    let raw = uri
        .strip_prefix("file://localhost")
        .or_else(|| uri.strip_prefix("file://"))?;
    let (b, mut out, mut i) = (raw.as_bytes(), Vec::with_capacity(raw.len()), 0);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    let s = String::from_utf8_lossy(&out).into_owned();
    let win = cfg!(windows) && s.starts_with('/') && s.as_bytes().get(2) == Some(&b':');
    Some(if win { s[1..].into() } else { s })
}

fn scan_subagents_cid(canon_cwd: &Path) -> Option<String> {
    let (mut best, mut scanned) = (None::<(SystemTime, String)>, 0usize);
    for root in agy_brain_roots() {
        for conv in fs::read_dir(&root).into_iter().flatten().flatten() {
            let cid = conv.file_name().to_string_lossy().into_owned();
            if !is_safe_id(&cid) || agy_transcript_for_cid(&cid).is_none() {
                continue;
            }
            let sd = conv.path().join(".system_generated/subagents");
            for sub in fs::read_dir(sd).into_iter().flatten().flatten() {
                let p = sub.path();
                if p.extension() != Some(OsStr::new("json")) {
                    continue;
                }
                scanned += 1;
                if scanned > 200 {
                    return best.map(|(_, c)| c);
                }
                let Ok(meta) = fs::metadata(&p) else {
                    continue;
                };
                if meta.len() > 65_536 {
                    continue;
                }
                let Ok(txt) = fs::read_to_string(&p) else {
                    continue;
                };
                let hits = extract_json_array_slice(&txt, "workspaceUris")
                    .map(extract_string_list_in_array)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|u| decode_file_uri(u))
                    .any(|s| {
                        let cu = clean_canonicalize(&s).unwrap_or_else(|_| PathBuf::from(&s));
                        paths_equal_ci(&cu, canon_cwd)
                    });
                let mt = meta.modified().unwrap_or(UNIX_EPOCH);
                if hits && best.as_ref().map_or(true, |(pm, _)| mt > *pm) {
                    best = Some((mt, cid.clone()));
                }
            }
        }
    }
    best.map(|(_, c)| c)
}

pub fn agy_resolve_cid(home: &str, yid: &str, cwd: &str) -> Option<String> {
    let valid = |s: &str| is_safe_id(s) && agy_transcript_for_cid(s).is_some();
    let from_tab = is_safe_id(yid)
        .then(|| fs::read_to_string(cache_dir(home).join(format!("tab_conv_{yid}.txt"))).ok())
        .flatten();
    let from_env = std::env::var("CCIDE_AGY_CONVERSATION").ok();
    [from_tab, from_env]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .find(|c| valid(c))
        .or_else(|| agy_cid_for_cwd(cwd).filter(|s| valid(s)))
        .or_else(|| {
            let cc = clean_canonicalize(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
            scan_subagents_cid(&cc)
        })
}
