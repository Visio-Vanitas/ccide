//! OpenAI Codex CLI (`codex`) Main & Subagent Rollout Ops (`src/session/codex.rs`).

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::json::extract_json_str;
use crate::session::agent::{codex_home, is_safe_id};
use crate::session::is_valid_model_name;
use crate::session::model::short_model_label;
use crate::session::subagents::SubagentItem;
use crate::session::transcript::JsonlTracker;
use crate::session::transcript_main::MainSessionOps;

#[allow(unused_imports)]
pub use crate::session::codex_discover::{
    codex_config, find_rollout_by_id, find_rollout_for_cwd, CodexConfig,
};
#[allow(unused_imports)]
pub(crate) use crate::session::codex_discover::{
    read_first_line, recent_day_dirs, rollout_session_id,
};

pub(crate) fn extract_apply_patch_target(patch: &str) -> Option<(String, usize)> {
    let (mut last_file, mut first_line) = (None, 0usize);
    for ln in patch.lines() {
        let t = ln.trim();
        let p = t
            .strip_prefix("*** Update File: ")
            .or_else(|| t.strip_prefix("*** Add File: "));
        if let Some(path) = p.map(str::trim).filter(|s| !s.is_empty()) {
            last_file = Some(path.to_string());
            first_line = 0;
        } else if first_line == 0 && t.starts_with("@@") {
            if let Some(plus) = t.split_whitespace().find(|w| w.starts_with('+')) {
                first_line = plus
                    .trim_start_matches('+')
                    .split(',')
                    .next()?
                    .parse()
                    .unwrap_or(0);
            }
        }
    }
    last_file.map(|f| (f, first_line))
}

pub fn codex_main_file_ops(path: &Path, tr: &mut JsonlTracker) -> Option<MainSessionOps> {
    let meta = fs::metadata(path).ok()?;
    let (len, file_mt) = (meta.len(), meta.modified().unwrap_or(UNIX_EPOCH));
    if len < tr.offset {
        tr.offset = 0;
        tr.seq = 0;
    }
    if len > tr.offset {
        let mut f = File::open(path).ok()?;
        let _ = f.seek(SeekFrom::Start(tr.offset));
        let mut raw = Vec::new();
        if f.read_to_end(&mut raw).is_ok() {
            if let Some(nl) = raw.iter().rposition(|&b| b == b'\n') {
                for line in String::from_utf8_lossy(&raw[..=nl]).lines() {
                    if line.contains("\"turn_context\"") {
                        if let Some(m) =
                            extract_json_str(line, "model").filter(|s| is_valid_model_name(s))
                        {
                            tr.last_model = m;
                            tr.last_model_mtime = file_mt;
                        }
                    } else if line.contains("\"response_item\"") && line.contains("\"apply_patch\"")
                    {
                        let patch = extract_json_str(line, "input").or_else(|| {
                            extract_json_str(line, "arguments").and_then(|a| {
                                extract_json_str(&a, "input")
                                    .or_else(|| extract_json_str(&a, "patch"))
                            })
                        });
                        if let Some((fp, ln)) =
                            patch.as_deref().and_then(extract_apply_patch_target)
                        {
                            tr.last_tool = "Edit".to_string();
                            tr.last_file = fp;
                            tr.last_line = ln;
                            tr.last_span = 1;
                            tr.seq += 1;
                        }
                    }
                }
                tr.offset += (nl + 1) as u64;
            }
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

pub fn codex_subagents(_home: &str, main_sid: &str, main_path: &Path) -> Vec<SubagentItem> {
    if main_sid.is_empty() {
        return vec![];
    }
    let mut dirs: Vec<PathBuf> = main_path
        .parent()
        .map(|p| p.to_path_buf())
        .into_iter()
        .collect();
    dirs.extend(recent_day_dirs(&codex_home()));
    let (mut seen, mut items, mut budget) = (HashSet::new(), Vec::new(), 400usize);
    for dir in dirs {
        if !seen.insert(dir.clone()) {
            continue;
        }
        for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
            if budget == 0 {
                break;
            }
            let p = e.path();
            if p == main_path || p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            budget -= 1;
            let Some(l1) = read_first_line(&p) else {
                continue;
            };
            if extract_json_str(&l1, "parent_thread_id").as_deref() != Some(main_sid) {
                continue;
            }
            let Some(sid) = extract_json_str(&l1, "session_id")
                .or_else(|| extract_json_str(&l1, "id"))
                .filter(|s| is_safe_id(s))
            else {
                continue;
            };
            let nick = extract_json_str(&l1, "agent_nickname").unwrap_or_default();
            let apath = extract_json_str(&l1, "agent_path").unwrap_or_else(|| "subagent".into());
            let label = if !nick.is_empty() {
                nick
            } else {
                apath.clone()
            };
            let (mut model_raw, mut effort, mut done, mut ctime) = (None, None, false, 0u64);
            if let Ok(mut f) = File::open(&p) {
                if let Ok(m) = f.metadata() {
                    ctime = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_millis() as u64);
                    let _ = f.seek(SeekFrom::Start(m.len().saturating_sub(65_536)));
                }
                let mut buf = Vec::new();
                if f.read_to_end(&mut buf).is_ok() {
                    for ln in String::from_utf8_lossy(&buf).lines() {
                        if ln.contains("\"turn_context\"") {
                            if let Some(m) =
                                extract_json_str(ln, "model").filter(|s| is_valid_model_name(s))
                            {
                                model_raw = Some(m);
                            }
                            if let Some(ef) = extract_json_str(ln, "effort") {
                                effort = Some(ef);
                            }
                        } else if ln.contains("\"task_complete\"") {
                            done = true;
                        }
                    }
                }
            }
            items.push(SubagentItem {
                full_id: sid,
                raw_type: apath,
                short_type: label.chars().take(12).collect(),
                desc: label,
                status: if done { "done" } else { "running" },
                model_short: model_raw.as_deref().map(short_model_label),
                model_raw,
                effort,
                tool: String::new(),
                file: String::new(),
                line: 0,
                span: 0,
                file_op_seq: 0,
                ctime,
                footer_rank: items.len() + 1,
            });
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_0144_0167_and_subagent_fixtures() {
        let base = std::env::temp_dir().join(format!("ccide_codex_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let (p144, p167, psub) = (
            base.join("r0144.jsonl"),
            base.join("r0167.jsonl"),
            base.join("rsub.jsonl"),
        );
        fs::write(
            &p144,
            include_str!("../../tests/fixtures/codex/main_0144.jsonl"),
        )
        .unwrap();
        fs::write(
            &p167,
            include_str!("../../tests/fixtures/codex/main_0167.jsonl"),
        )
        .unwrap();
        fs::write(
            &psub,
            include_str!("../../tests/fixtures/codex/subagent_0144.jsonl"),
        )
        .unwrap();
        let mut tr1 = JsonlTracker::from_tail(Some(p144.clone()));
        tr1.offset = 0;
        let ops1 = codex_main_file_ops(&p144, &mut tr1).unwrap();
        assert_eq!(
            (ops1.tool.as_str(), ops1.file.as_str(), ops1.line),
            ("Edit", "src/lib.rs", 10)
        );
        assert_eq!(ops1.model, "gpt-5.4");
        let mut tr2 = JsonlTracker::from_tail(Some(p167.clone()));
        tr2.offset = 0;
        let ops2 = codex_main_file_ops(&p167, &mut tr2).unwrap();
        assert_eq!(
            (ops2.tool.as_str(), ops2.file.as_str(), ops2.line),
            ("Edit", "src/new.rs", 1)
        );
        assert_eq!(ops2.model, "gpt-6-codex");
        let subs = codex_subagents("", "0194b8c2-1111-7000-8000-000000000144", &p144);
        assert_eq!(subs.len(), 1);
        assert_eq!(
            (subs[0].desc.as_str(), subs[0].status),
            ("worker-1", "done")
        );
        assert_eq!(subs[0].model_raw.as_deref(), Some("o4-mini"));
        let _ = fs::remove_dir_all(&base);
    }
}
