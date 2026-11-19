//! Multi-Agent Tool Payload Normalization (`to_claude_shape`), Stdout Policy & Live Hook Records.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::yazi_hook::hook_input::{first_esc, first_str, HookEvent, HookEventKind};
use crate::ffi::{current_ppid, get_ppid_of, ProcToken};
use crate::json::{extract_json_object, extract_json_str, extract_json_u64, json_escape};
use crate::platform::atomic_write;
use crate::session::{agy, codex, write_tab_agent, AgentKind};

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
    let prev = fs::read_to_string(cdir.join(format!("live_tab_{yid}.json")))
        .or_else(|_| fs::read_to_string(cdir.join(format!("live_sess_{}.json", ev.session_id))))
        .unwrap_or_default();
    let eff_mod = if !ev.model.is_empty() {
        ev.model.clone()
    } else {
        first_str(&prev, &["model_id", "model"]).unwrap_or_default()
    };
    let tp = if !ev.transcript_path.is_empty() {
        ev.transcript_path.clone()
    } else if ev.agent == AgentKind::Agy && !ev.session_id.is_empty() {
        agy::agy_transcript_path(&ev.session_id)
            .to_string_lossy()
            .into_owned()
    } else if ev.agent == AgentKind::Codex && !ev.session_id.is_empty() {
        codex::find_rollout_by_id(&ev.session_id)
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| first_str(&prev, &["transcript_path"]))
            .unwrap_or_default()
    } else {
        first_str(&prev, &["transcript_path"]).unwrap_or_default()
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
    if !yid.is_empty() {
        let _ = atomic_write(&cdir.join(format!("live_tab_{yid}.json")), rec.as_bytes());
        if !cdir.join(format!("tab_agent_{yid}.txt")).is_file() {
            write_tab_agent(home, yid, ev.agent);
        }
    }
}
