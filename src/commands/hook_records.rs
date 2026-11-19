//! Multi-Agent Tool Payload Normalization (`to_claude_shape`), Stdout Policy & Live Hook Records.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::yazi_hook::hook_input::{
    first_esc, first_str, should_reveal_claude_view, HookEvent, HookEventKind,
};
use crate::ffi::{current_ppid, get_ppid_of, ProcToken};
use crate::json::{extract_json_object, extract_json_str, extract_json_u64, json_escape};
use crate::platform::atomic_write;
use crate::session::{
    agy, agy_is_member_of, agy_parent_coordinator, codex, is_safe_id, write_tab_agent, AgentKind,
};

fn extract_last_patch_path(patch_txt: &str) -> Option<String> {
    let norm = if !patch_txt.contains('\n') && patch_txt.contains("\\n") {
        patch_txt.replace("\\r\\n", "\n").replace("\\n", "\n")
    } else {
        patch_txt.to_string()
    };
    norm.lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("*** Update File: ")
                .or_else(|| l.trim().strip_prefix("*** Add File: "))
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .next_back()
        .map(String::from)
}

pub fn to_claude_shape(ev: &HookEvent) -> Option<(String, String)> {
    let ti = &ev.tool_input;
    match ev.agent {
        AgentKind::Claude => (!ev.tool_name.is_empty()).then(|| (ev.tool_name.clone(), ti.clone())),
        AgentKind::Agy => {
            if matches!(
                ev.tool_name.as_str(),
                "swarm" | "post_message" | "invoke_subagent" | "swarm_wait"
            ) {
                return None;
            }
            let keys = ["Path", "AbsolutePath", "TargetFile", "file_path", "path"];
            let ef = json_escape(&first_str(ti, &keys)?);
            let (kind, body) = match ev.tool_name.as_str() {
                "read_file" | "view_file" | "Read" => {
                    let s = extract_json_u64(ti, "StartLine")
                        .or_else(|| extract_json_u64(ti, "offset"));
                    let lim = match (s, extract_json_u64(ti, "EndLine")) {
                        (Some(a), Some(b)) if b >= a => b.saturating_sub(a).saturating_add(1),
                        _ => extract_json_u64(ti, "limit").unwrap_or(0),
                    };
                    let off = s.unwrap_or(0);
                    (
                        "Read",
                        format!("{{\"file_path\":\"{ef}\",\"offset\":{off},\"limit\":{lim}}}"),
                    )
                }
                "edit_file" | "replace_file_content" | "multi_replace_file_content" | "Edit" => {
                    let o = first_esc(ti, &["Target", "TargetContent", "old_string"]);
                    let n = first_esc(ti, &["Replacement", "ReplacementContent", "new_string"]);
                    (
                        "Edit",
                        format!(
                            "{{\"file_path\":\"{ef}\",\"old_string\":\"{o}\",\"new_string\":\"{n}\"}}"
                        ),
                    )
                }
                "write_file" | "write_to_file" | "Write" => {
                    ("Write", format!("{{\"file_path\":\"{ef}\"}}"))
                }
                _ => ("Read", format!("{{\"file_path\":\"{ef}\"}}")),
            };
            Some((kind.to_string(), body))
        }
        AgentKind::Codex => {
            if ev.tool_name == "apply_patch" {
                let args = extract_json_object(ti, "arguments")
                    .map(String::from)
                    .or_else(|| extract_json_str(ti, "arguments"));
                let scope = args.as_deref().unwrap_or(ti);
                let raw = first_str(ti, &["input", "patch"])
                    .or_else(|| first_str(scope, &["input", "patch"]))
                    .unwrap_or_else(|| scope.to_string());
                let fp = json_escape(&extract_last_patch_path(&raw)?);
                return Some(("Edit".to_string(), format!("{{\"file_path\":\"{fp}\"}}")));
            }
            let ef = json_escape(&first_str(ti, &["file_path", "path"])?);
            Some(("Read".to_string(), format!("{{\"file_path\":\"{ef}\"}}")))
        }
    }
}

pub fn emit_required_stdout(agent: AgentKind, ev: HookEventKind) {
    let mut out = std::io::stdout();
    if agent == AgentKind::Agy {
        let msg: &[u8] = if ev == HookEventKind::PreToolUse {
            b"{\"decision\":\"ask\"}\n"
        } else {
            b"{}\n"
        };
        let _ = out.write_all(msg);
    }
    let _ = out.flush();
}

fn resolve_agent_tp(ev: &HookEvent, prev: &str) -> String {
    if !ev.transcript_path.is_empty() {
        ev.transcript_path.clone()
    } else if ev.agent == AgentKind::Agy && !ev.session_id.is_empty() {
        agy::agy_transcript_for_cid(&ev.session_id)
            .unwrap_or_else(|| agy::agy_transcript_path(&ev.session_id))
            .to_string_lossy()
            .into_owned()
    } else if ev.agent == AgentKind::Codex && !ev.session_id.is_empty() {
        codex::find_rollout_by_id(&ev.session_id)
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| first_str(prev, &["transcript_path"]))
            .unwrap_or_default()
    } else {
        first_str(prev, &["transcript_path"]).unwrap_or_default()
    }
}

fn codex_rollout_is_worker(tp: &str) -> bool {
    !tp.is_empty()
        && codex::read_first_line(Path::new(tp))
            .and_then(|l1| extract_json_str(&l1, "parent_thread_id"))
            .filter(|s| !s.is_empty())
            .is_some()
}

pub fn agy_event_is_worker(cdir: &Path, yid: &str, ev: &HookEvent) -> bool {
    if ev.agent != AgentKind::Agy || ev.session_id.is_empty() {
        return false;
    }
    if is_safe_id(yid) {
        if let Ok(tc) = fs::read_to_string(cdir.join(format!("tab_conv_{yid}.txt"))) {
            let tc_cid = tc.trim();
            if is_safe_id(tc_cid) && tc_cid != ev.session_id {
                return true;
            }
        }
        if let Ok(prev) = fs::read_to_string(cdir.join(format!("live_tab_{yid}.json"))) {
            let is_agy = extract_json_str(&prev, "agent").as_deref() == Some("agy");
            let prev_sid = extract_json_str(&prev, "session_id").unwrap_or_default();
            let prev_tp = extract_json_str(&prev, "transcript_path").unwrap_or_default();
            if is_agy
                && !prev_sid.is_empty()
                && prev_sid != ev.session_id
                && !prev_tp.is_empty()
                && agy_is_member_of(Path::new(&prev_tp), &ev.session_id)
            {
                return true;
            }
        }
    }
    let tp = resolve_agent_tp(ev, "");
    agy_parent_coordinator(&ev.session_id, Path::new(&tp)).is_some()
}

pub fn should_reveal_agent_view(cdir: &Path, yid: &str, ev: &HookEvent, is_worker: bool) -> bool {
    match ev.agent {
        AgentKind::Claude => should_reveal_claude_view(cdir, yid, &ev.session_id, &ev.agent_id),
        AgentKind::Agy => {
            let raw =
                fs::read_to_string(cdir.join(format!("active_view_{yid}.txt"))).unwrap_or_default();
            let t = raw.trim();
            let active_view = if t.is_empty() { "main" } else { t };
            if is_worker {
                !ev.session_id.is_empty() && active_view == ev.session_id
            } else {
                active_view == "main" || (!ev.session_id.is_empty() && active_view == ev.session_id)
            }
        }
        AgentKind::Codex => !is_worker && !codex_rollout_is_worker(&ev.transcript_path),
    }
}

pub fn write_agent_hook_records(home: &str, cdir: &Path, ev: &HookEvent, yid: &str) {
    let shell_pid = fs::read_to_string(cdir.join(format!("tab_shell_{yid}.txt")))
        .ok()
        .and_then(|s| ProcToken::parse(&s).map(|t| t.pid))
        .unwrap_or(0);
    let mut cur = current_ppid();
    let mut host_pid = cur;
    for _ in 0..33 {
        if cur <= 1 {
            break;
        }
        match get_ppid_of(cur) {
            Some(p) if shell_pid > 1 && p == shell_pid => {
                host_pid = cur;
                break;
            }
            Some(p) if p > 1 && p != cur => cur = p,
            _ => break,
        }
    }
    let pid_tok = ProcToken::current(host_pid)
        .map(|t| t.encode())
        .unwrap_or_else(|| host_pid.to_string());
    let is_agy_worker = agy_event_is_worker(cdir, yid, ev);
    let prev = if is_agy_worker {
        fs::read_to_string(cdir.join(format!("live_sess_{}.json", ev.session_id)))
            .unwrap_or_default()
    } else {
        fs::read_to_string(cdir.join(format!("live_tab_{yid}.json")))
            .or_else(|_| fs::read_to_string(cdir.join(format!("live_sess_{}.json", ev.session_id))))
            .unwrap_or_default()
    };
    let eff_mod = if !ev.model.is_empty() {
        ev.model.clone()
    } else {
        first_str(&prev, &["model_id", "model"]).unwrap_or_default()
    };
    let tp = resolve_agent_tp(ev, &prev);
    let is_worker = match ev.agent {
        AgentKind::Agy => is_agy_worker,
        AgentKind::Codex => codex_rollout_is_worker(&tp),
        AgentKind::Claude => false,
    };
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (ag, sid, em) = (
        ev.agent.as_str(),
        json_escape(&ev.session_id),
        json_escape(&eff_mod),
    );
    let (cwd, etp, pt) = (
        json_escape(&ev.cwd),
        json_escape(&tp),
        json_escape(&pid_tok),
    );
    let rec = format!(
        "{{\"agent\":\"{ag}\",\"session_id\":\"{sid}\",\"model_id\":\"{em}\",\"model\":\"{em}\",\"cwd\":\"{cwd}\",\"transcript_path\":\"{etp}\",\"agent_pid\":\"{pt}\",\"updated\":{now_s},\"ts\":{now_s}}}"
    );
    if !ev.session_id.is_empty() {
        let p = cdir.join(format!("live_sess_{}.json", ev.session_id));
        let _ = atomic_write(&p, rec.as_bytes());
    }
    if !is_worker && !yid.is_empty() {
        let _ = atomic_write(&cdir.join(format!("live_tab_{yid}.json")), rec.as_bytes());
        if !cdir.join(format!("tab_agent_{yid}.txt")).is_file() {
            write_tab_agent(home, yid, ev.agent);
        }
    }
}
