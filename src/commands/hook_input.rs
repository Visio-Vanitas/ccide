//! Multi-Agent (`claude` · `agy` · `codex`) Hook Input Detection & Event Parsing.

#[path = "hook_records.rs"]
mod hook_records;

use std::env;
use std::fs;
use std::path::{Component, Path};

use crate::ffi::{current_ppid, get_ppid_of};
use crate::json::{
    extract_json_array_slice, extract_json_object, extract_json_str, extract_string_list_in_array,
    json_escape,
};
use crate::platform::ipc_path;
use crate::session::{codex_home, is_safe_id, AgentKind};

pub use hook_records::{
    agy_event_is_worker, emit_required_stdout, should_reveal_agent_view, to_claude_shape,
    write_agent_hook_records,
};

pub(crate) fn should_reveal_claude_view(cdir: &Path, yid: &str, sid: &str, agent_id: &str) -> bool {
    let mut active_view = fs::read_to_string(cdir.join(format!("active_view_{yid}.txt")))
        .unwrap_or_else(|_| "main".to_string())
        .trim()
        .to_string();
    let mut pid_cur = None;
    let mut pid = current_ppid();
    for _ in 0..4 {
        if pid <= 1 {
            break;
        }
        if let Ok(txt) = fs::read_to_string(ipc_path(&format!("cur_pid_{pid}.txt"))) {
            pid_cur = Some(txt);
            break;
        }
        match get_ppid_of(pid) {
            Some(parent) if parent > 1 && parent != pid => pid = parent,
            _ => break,
        }
    }
    let sess_cur = (!sid.is_empty())
        .then(|| fs::read_to_string(ipc_path(&format!("cur_{sid}.txt"))).ok())
        .flatten();
    if let Some(cur_native) = pid_cur.or(sess_cur) {
        let cn = cur_native.lines().next().unwrap_or("").trim();
        if !cn.is_empty() {
            active_view = cn.to_string();
        }
    }
    let norm_view = if active_view.is_empty() {
        "main"
    } else {
        active_view.as_str()
    };
    if agent_id.is_empty() {
        norm_view == "main"
    } else {
        norm_view == agent_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEventKind {
    SessionStart,
    PreToolUse,
    PostToolUse,
    SubagentStart,
    SubagentStop,
    PostModelSwitch,
    Stop,
    Other,
}

#[derive(Clone, Debug)]
pub struct HookEvent {
    pub agent: AgentKind,
    pub event: HookEventKind,
    pub session_id: String,
    pub cwd: String,
    pub transcript_path: String,
    pub model: String,
    pub tool_name: String,
    pub tool_input: String,
    pub agent_id: String,
    pub perm_mode: String,
}

pub(crate) fn first_str(src: &str, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| extract_json_str(src, k).filter(|s| !s.is_empty()))
}

pub(crate) fn first_esc(src: &str, keys: &[&str]) -> String {
    json_escape(&first_str(src, keys).unwrap_or_default())
}

fn has_dir_prefix(path: &str, prefix: &str) -> bool {
    !prefix.is_empty()
        && path
            .strip_prefix(prefix)
            .map(|r| r.is_empty() || r.starts_with('/') || r.starts_with('\\'))
            .unwrap_or(false)
}

pub fn detect_agent(input: &str, forced: Option<AgentKind>) -> AgentKind {
    if let Some(k) = forced {
        return k;
    }
    if extract_json_str(input, "conversationId").is_some()
        || extract_json_str(input, "hookEventName").is_some()
    {
        return AgentKind::Agy;
    }
    let tp = first_str(input, &["transcript_path", "transcriptPath"]).unwrap_or_default();
    let codex_dir = codex_home().to_string_lossy().into_owned();
    if has_dir_prefix(&tp, &codex_dir)
        || tp.contains("/.codex/")
        || tp.contains("\\.codex\\")
        || extract_json_str(input, "workspace_dir").is_some()
    {
        return AgentKind::Codex;
    }
    let claude_dir = env::var("CLAUDE_CONFIG_DIR").unwrap_or_default();
    if has_dir_prefix(&tp, &claude_dir)
        || tp.contains("/.claude/")
        || tp.contains("\\.claude\\")
        || extract_json_str(input, "permission_mode").is_some()
    {
        return AgentKind::Claude;
    }
    if extract_json_object(input, "toolCall").is_some() {
        return AgentKind::Agy;
    }
    env::var("CCIDE_AGENT")
        .ok()
        .and_then(|v| AgentKind::from_name(&v))
        .unwrap_or(AgentKind::Claude)
}

pub fn parse_hook_event(input: &str, forced: Option<AgentKind>) -> HookEvent {
    let agent = detect_agent(input, forced);
    let raw_ev = first_str(input, &["hook_event_name", "hookEventName", "event"]);
    let event = match raw_ev.as_deref().unwrap_or("") {
        "SessionStart" | "session_start" => HookEventKind::SessionStart,
        "PreToolUse" | "pre_tool_use" => HookEventKind::PreToolUse,
        "PostToolUse" | "post_tool_use" => HookEventKind::PostToolUse,
        "SubagentStart" | "subagent_start" => HookEventKind::SubagentStart,
        "SubagentStop" | "subagent_stop" => HookEventKind::SubagentStop,
        "PostModelSwitch" | "post_model_switch" => HookEventKind::PostModelSwitch,
        "Stop" | "stop" => HookEventKind::Stop,
        _ => HookEventKind::Other,
    };
    let cwd = first_str(input, &["cwd", "workspace_dir"])
        .or_else(|| {
            extract_json_array_slice(input, "workspacePaths")
                .map(extract_string_list_in_array)?
                .into_iter()
                .find(|s| !s.is_empty())
        })
        .unwrap_or_default();
    let transcript_path = first_str(input, &["transcript_path", "transcriptPath"])
        .filter(|p| {
            !p.contains('\0')
                && Path::new(p).is_absolute()
                && !Path::new(p).components().any(|c| c == Component::ParentDir)
        })
        .unwrap_or_default();
    let (tool_name, tool_input) = if let Some(tc) = extract_json_object(input, "toolCall") {
        let args = extract_json_object(tc, "args")
            .or_else(|| extract_json_object(tc, "arguments"))
            .unwrap_or(tc);
        (
            extract_json_str(tc, "name").unwrap_or_default(),
            args.to_string(),
        )
    } else {
        let ti = extract_json_object(input, "tool_input")
            .map(String::from)
            .or_else(|| extract_json_str(input, "tool_input"))
            .unwrap_or_else(|| input.to_string());
        (extract_json_str(input, "tool_name").unwrap_or_default(), ti)
    };
    HookEvent {
        agent,
        event,
        session_id: first_str(input, &["session_id", "conversationId"])
            .filter(|s| is_safe_id(s))
            .unwrap_or_default(),
        cwd,
        transcript_path,
        model: first_str(input, &["to_model", "model", "modelName"]).unwrap_or_default(),
        tool_name,
        tool_input,
        agent_id: extract_json_str(input, "agent_id")
            .filter(|s| is_safe_id(s))
            .unwrap_or_default(),
        perm_mode: first_str(input, &["permission_mode", "mode"]).unwrap_or_default(),
    }
}
