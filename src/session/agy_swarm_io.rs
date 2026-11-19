//! Antigravity Swarm Board Tail, Brain Roots & Conversation ID Resolution.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::json::{
    extract_json_array_slice, extract_json_object, extract_json_str, extract_json_u64,
    extract_string_list_in_array, iter_object_members,
};
use crate::platform::{cache_dir, clean_canonicalize, paths_equal_ci, user_home_dir};
use crate::session::agent::{agy_home, is_safe_id};
use crate::session::agy::agy_cid_for_cwd;

type MembersCache = HashMap<PathBuf, (u64, SystemTime, String)>;
thread_local! {
    static MEMBERS_CACHE: RefCell<MembersCache> = RefCell::new(HashMap::new());
}

pub fn agy_sys_gen(transcript: &Path) -> Option<PathBuf> {
    let p = transcript.parent()?.parent()?;
    (p.file_name() == Some(OsStr::new(".system_generated"))).then(|| p.to_path_buf())
}

pub fn agy_brain_root(transcript: &Path) -> Option<PathBuf> {
    agy_sys_gen(transcript)?
        .parent()?
        .parent()
        .map(Path::to_path_buf)
}

pub(crate) fn read_members_cached(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok().filter(|m| m.len() <= 262_144)?;
    let key = (meta.len(), meta.modified().unwrap_or(UNIX_EPOCH));
    if let Some(hit) = MEMBERS_CACHE.with(|c| {
        c.borrow()
            .get(path)
            .filter(|(l, m, _)| (*l, *m) == key)
            .map(|(_, _, t)| t.clone())
    }) {
        return Some(hit);
    }
    let txt = fs::read_to_string(path).ok()?;
    MEMBERS_CACHE.with(|c| {
        let mut m = c.borrow_mut();
        if m.len() >= 32 {
            m.clear();
        }
        m.insert(path.to_path_buf(), (key.0, key.1, txt.clone()));
    });
    Some(txt)
}

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

pub(crate) fn decode_file_uri(uri: &str) -> Option<String> {
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
    let win = s.starts_with('/')
        && s.as_bytes().get(1).is_some_and(|c| c.is_ascii_alphabetic())
        && s.as_bytes().get(2) == Some(&b':');
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
            for sub in fs::read_dir(conv.path().join(".system_generated/subagents"))
                .into_iter()
                .flatten()
                .flatten()
            {
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

pub fn agy_is_member_of(coord_transcript: &Path, candidate_cid: &str) -> bool {
    if !is_safe_id(candidate_cid) {
        return false;
    }
    let Some(sg) = agy_sys_gen(coord_transcript) else {
        return false;
    };
    if let Some(txt) = read_members_cached(&sg.join("swarm/members.json")) {
        if let Some(obj) = extract_json_object(&txt, "members") {
            for (_, mj) in iter_object_members(obj) {
                if extract_json_str(mj, "conversation_id").as_deref() == Some(candidate_cid) {
                    return true;
                }
            }
        }
    }
    let sub = sg.join("subagents").join(format!("{candidate_cid}.json"));
    fs::metadata(&sub).ok().is_some_and(|m| m.len() <= 65_536)
        && fs::read_to_string(&sub).ok().is_some_and(|st| {
            extract_json_str(&st, "conversationId").as_deref() == Some(candidate_cid)
        })
}

pub fn agy_parent_coordinator(cid: &str, transcript_path: &Path) -> Option<(String, PathBuf)> {
    if !is_safe_id(cid)
        || agy_sys_gen(transcript_path).is_some_and(|sg| sg.join("swarm/members.json").is_file())
    {
        return None;
    }
    let mut roots = Vec::new();
    if let Some(br) = agy_brain_root(transcript_path) {
        roots.push(br);
    }
    for r in agy_brain_roots() {
        if !roots.contains(&r) {
            roots.push(r);
        }
    }
    let mut scanned = 0usize;
    for root in roots {
        for conv in fs::read_dir(&root).into_iter().flatten().flatten() {
            scanned += 1;
            if scanned > 128 {
                return None;
            }
            let cand_cid = conv.file_name().to_string_lossy().into_owned();
            if cand_cid == cid || !is_safe_id(&cand_cid) {
                continue;
            }
            let cand_tr = conv.path().join(".system_generated/logs/transcript.jsonl");
            if cand_tr.is_file() && agy_is_member_of(&cand_tr, cid) {
                return Some((cand_cid, cand_tr));
            }
        }
    }
    None
}

pub fn agy_resolve_cid_detail(home: &str, yid: &str, cwd: &str) -> Option<(String, bool)> {
    let valid = |s: &str| is_safe_id(s) && agy_transcript_for_cid(s).is_some();
    let from_tab = is_safe_id(yid)
        .then(|| fs::read_to_string(cache_dir(home).join(format!("tab_conv_{yid}.txt"))).ok())
        .flatten();
    let from_env = std::env::var("CCIDE_AGY_CONVERSATION").ok();
    if let Some(cid) = [from_tab, from_env]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .find(|c| valid(c))
    {
        return Some((cid, true));
    }
    if let Some(cid) = agy_cid_for_cwd(cwd).filter(|s| valid(s)) {
        return Some((cid, false));
    }
    let cc = clean_canonicalize(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
    scan_subagents_cid(&cc).map(|cid| (cid, false))
}

#[allow(dead_code)]
pub fn agy_resolve_cid(home: &str, yid: &str, cwd: &str) -> Option<String> {
    agy_resolve_cid_detail(home, yid, cwd).map(|(cid, _)| cid)
}
