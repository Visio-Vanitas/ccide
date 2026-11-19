//! `ccide-core yazi-hook [--agent claude|agy|codex]`: Cross-Platform Native `PostToolUse` / `Subagent*` Hook.

#[path = "hook_input.rs"]
pub mod hook_input;

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ffi::{current_ppid, get_ppid_of, notify_sidebar};
use crate::json::{extract_json_bool, extract_json_object, extract_json_str, json_escape};
use crate::platform::{atomic_write, cache_dir, ipc_path, is_internal_or_temp_path, user_home_dir};
use crate::session::{
    emit_yazi_reveal_at, extract_tool_line_target, is_valid_model_name, normalize_path_under_cwd,
    resolve_live_yazi_tab, AgentKind,
};

use hook_input::{
    emit_required_stdout, parse_hook_event, to_claude_shape, write_agent_hook_records,
    HookEventKind,
};

fn extract_first_filename_from_array(src: &str) -> Option<String> {
    let mut from = 0;
    while let Some(rel) = src[from..].find("\"filenames\"") {
        from += rel + 11;
        let Some(arr) = src[from..]
            .trim_start()
            .strip_prefix(':')
            .and_then(|s| s.trim_start().strip_prefix('['))
            .and_then(|s| s.trim_start().strip_prefix('"'))
        else {
            continue;
        };
        let mut out = String::new();
        let mut chars = arr.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(nc) = chars.next() {
                    out.push(nc);
                }
            } else if c == '"' {
                return (!out.is_empty()).then_some(out);
            } else {
                out.push(c);
            }
        }
        return None;
    }
    None
}

fn should_reveal_claude_view(cdir: &Path, yid: &str, sid: &str, agent_id: &str) -> bool {
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

pub fn run_with_agent(forced: Option<AgentKind>) {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if input.trim().is_empty() {
        emit_required_stdout(
            hook_input::detect_agent(&input, forced),
            HookEventKind::Other,
        );
        return;
    }

    let ev = parse_hook_event(&input, forced);
    if ev.agent != AgentKind::Claude {
        emit_required_stdout(ev.agent, ev.event);
    }

    let home = user_home_dir();
    let cdir = cache_dir(&home);
    let cdir_str = cdir.to_string_lossy().into_owned();
    let status_dir = cdir.join("status");
    let _ = fs::create_dir_all(&status_dir);

    if !ev.perm_mode.is_empty() && !ev.session_id.is_empty() {
        let _ = atomic_write(
            &status_dir.join(format!("mode_{}.cache", ev.session_id)),
            ev.perm_mode.as_bytes(),
        );
    }
    if matches!(
        ev.event,
        HookEventKind::PostModelSwitch | HookEventKind::SessionStart
    ) && !ev.session_id.is_empty()
        && is_valid_model_name(&ev.model)
    {
        let _ = atomic_write(
            &status_dir.join(format!("model_{}.cache", ev.session_id)),
            ev.model.trim().as_bytes(),
        );
    }

    let (target_yazi_id, tab_cwd) = resolve_live_yazi_tab(&cdir_str, &ev.session_id, &ev.cwd);
    if !target_yazi_id.is_empty() && !ev.session_id.is_empty() {
        let _ = atomic_write(
            &cdir.join(format!("session_{}.yazi", ev.session_id)),
            target_yazi_id.as_bytes(),
        );
    }
    if ev.agent != AgentKind::Claude {
        write_agent_hook_records(&home, &cdir, &ev, &target_yazi_id);
    }
    if ev.event == HookEventKind::SubagentStop
        && !ev.session_id.is_empty()
        && !ev.agent_id.is_empty()
    {
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(cdir.join(format!("stopped_{}.txt", ev.session_id)))
        {
            let _ = f.write_all(format!("{}\n", ev.agent_id).as_bytes());
        }
    }

    notify_sidebar(&ev.session_id, &target_yazi_id);

    if ev.agent == AgentKind::Claude && ev.event == HookEventKind::SessionStart {
        let _ = crate::orchestrator::OrchestratorWatcher::new(&home);
        let agents_dir = json_escape(&Path::new(&home).join(".claude/agents").to_string_lossy());
        println!(
            "{{\"hookSpecificOutput\":{{\"hookEventName\":\"SessionStart\",\"watchPaths\":[\"{agents_dir}\"],\"additionalContext\":\"cc-ide Orchestrator active: prefix Agent description/prompt with [@<model>:<effort>] (e.g. [@kimi-k3:high], [@:low]) to override routing, and use '@scout'/'@coder'/'@reviewer'/'@plan' macros in Workflow scripts.\"}}}}"
        );
        let _ = std::io::stdout().flush();
    }

    if extract_json_bool(&input, "staged") == Some(true)
        || (ev.agent == AgentKind::Agy
            && matches!(
                ev.tool_name.as_str(),
                "swarm" | "post_message" | "invoke_subagent" | "swarm_wait"
            ))
    {
        return;
    }

    let shaped = to_claude_shape(&ev);
    let (eff_tool, tool_in_str) = match &shaped {
        Some((tn, ti)) => (tn.as_str(), ti.as_str()),
        None => (ev.tool_name.as_str(), ev.tool_input.as_str()),
    };
    let mut raw_file = extract_json_str(tool_in_str, "file_path")
        .or_else(|| extract_json_str(tool_in_str, "filePath"))
        .or_else(|| extract_json_str(tool_in_str, "notebook_path"))
        .or_else(|| extract_json_str(tool_in_str, "path"))
        .or_else(|| extract_json_str(&input, "file_path"))
        .or_else(|| extract_json_str(&input, "filePath"))
        .unwrap_or_default();

    let to_abs = |f: &str| -> String {
        if Path::new(f).is_absolute() || ev.cwd.is_empty() {
            f.to_string()
        } else {
            Path::new(&ev.cwd).join(f).to_string_lossy().into_owned()
        }
    };

    let mut resolved_path = if raw_file.is_empty() {
        String::new()
    } else {
        to_abs(&raw_file)
    };
    if (resolved_path.is_empty() || !Path::new(&resolved_path).is_file())
        && matches!(eff_tool, "Glob" | "Grep")
    {
        if let Some(first_fn) = extract_first_filename_from_array(
            extract_json_object(&input, "tool_response").unwrap_or(&input),
        ) {
            raw_file = first_fn;
            resolved_path = to_abs(&raw_file);
        }
    }
    if resolved_path.is_empty() || !Path::new(&resolved_path).is_file() {
        return;
    }

    let norm_base = if tab_cwd.is_empty() {
        &ev.cwd
    } else {
        &tab_cwd
    };
    let file_path = normalize_path_under_cwd(&resolved_path, norm_base);
    if is_internal_or_temp_path(&file_path, &home)
        || target_yazi_id.is_empty()
        || !target_yazi_id.chars().all(|c| c.is_ascii_digit())
    {
        return;
    }

    let line_src = if ev.agent == AgentKind::Claude {
        input.clone()
    } else {
        format!("{{\"tool_input\":{tool_in_str}}}")
    };
    let (target_line, target_span) = extract_tool_line_target(&line_src, eff_tool, &file_path);
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if ev.agent_id.is_empty() {
        let main_json = format!(
            "{{\"file\":\"{}\",\"tool\":\"{}\",\"line\":{target_line},\"span\":{target_span},\"ts\":{now_s}}}\n",
            json_escape(&file_path),
            json_escape(eff_tool),
        );
        let _ = atomic_write(
            &cdir.join(format!("main_last_file_{target_yazi_id}.json")),
            main_json.as_bytes(),
        );
    }

    if ev.agent == AgentKind::Claude
        && !should_reveal_claude_view(&cdir, &target_yazi_id, &ev.session_id, &ev.agent_id)
    {
        return;
    }

    emit_yazi_reveal_at(
        &home,
        &target_yazi_id,
        norm_base,
        &file_path,
        target_line,
        target_span,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_first_glob_filename_cleanly() {
        let js =
            r#"{"tool_name":"Glob","tool_response":{"filenames":["src/main.rs","src/lib.rs"]}}"#;
        assert_eq!(
            extract_first_filename_from_array(js),
            Some("src/main.rs".to_string())
        );
    }
}
