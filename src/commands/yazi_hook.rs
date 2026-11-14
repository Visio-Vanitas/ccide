//! `ccide-core yazi-hook`: Cross-Platform Native `PostToolUse` / `Subagent*` Hook.
//!
//! Fixes and enhancements for Claude Code 2.1.3xx & Windows/Linux/macOS:
//! 1. Caches `permission_mode` BEFORE `raw_file.is_empty()` early return so `statusline` always
//!    knows the active mode even when `statusLine` stdin omits `permission_mode`.
//! 2. Distinguishes subagent tool calls (`agent_id` present) from main-thread calls: background
//!    subagents NEVER overwrite `main_last_file_<yazi_id>.json` or hijack Yazi while viewing `main`.
//! 3. Supports `filenames[0]` fallback for `Glob`/`Grep` tool responses and ignores `staged: true`
//!    unapplied file previews.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ffi::{current_ppid, notify_sidebar};
use crate::json::{extract_json_bool, extract_json_object, extract_json_str};
use crate::platform::{cache_dir, ipc_path, is_internal_or_temp_path, user_home_dir};
use crate::session::{
    emit_yazi_reveal_at, extract_tool_line_target, is_valid_model_name, normalize_path_under_cwd,
    resolve_live_yazi_tab,
};

fn extract_first_filename_from_array(src: &str) -> Option<String> {
    let pos = src.find("\"filenames\"")?;
    let rest = src[pos + 11..].trim_start().strip_prefix(':')?.trim_start();
    let arr = rest.strip_prefix('[')?.trim_start();
    let q1 = arr.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = q1.chars();
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
    None
}

pub fn run() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if input.is_empty() {
        return;
    }

    let hook_event = extract_json_str(&input, "hook_event_name").unwrap_or_default();
    let tool_name = extract_json_str(&input, "tool_name").unwrap_or_default();
    let session_id = extract_json_str(&input, "session_id").unwrap_or_default();
    let agent_id = extract_json_str(&input, "agent_id").unwrap_or_default();
    let perm_mode = extract_json_str(&input, "permission_mode").unwrap_or_default();
    let cwd_path = extract_json_str(&input, "cwd").unwrap_or_default();

    let home = user_home_dir();
    let cdir = cache_dir(&home);
    let cdir_str = cdir.to_string_lossy().into_owned();
    let _ = fs::create_dir_all(cdir.join("status"));

    // Always persist `permission_mode` upfront on EVERY hook (`SessionStart`, `SubagentStart`, `PostToolUse`, etc.)
    if !perm_mode.is_empty() && !session_id.is_empty() {
        let _ = fs::write(
            cdir.join("status")
                .join(format!("mode_{}.cache", session_id)),
            &perm_mode,
        );
    }

    // Official cross-version `PostModelSwitch` / `SessionStart` plugin model sync (zero binary patch required!)
    if matches!(hook_event.as_str(), "PostModelSwitch" | "SessionStart") && !session_id.is_empty() {
        let switched_model = extract_json_str(&input, "to_model")
            .or_else(|| extract_json_str(&input, "model"))
            .unwrap_or_default();
        if is_valid_model_name(&switched_model) {
            let _ = fs::write(
                cdir.join("status")
                    .join(format!("model_{}.cache", session_id)),
                switched_model.trim(),
            );
        }
    }

    let (target_yazi_id, tab_cwd) = resolve_live_yazi_tab(&cdir_str, &session_id, &cwd_path);
    if !target_yazi_id.is_empty() && !session_id.is_empty() {
        let _ = fs::write(
            cdir.join(format!("session_{}.yazi", session_id)),
            &target_yazi_id,
        );
    }

    if hook_event == "SubagentStop" && !session_id.is_empty() && !agent_id.is_empty() {
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(cdir.join(format!("stopped_{}.txt", session_id)))
        {
            let _ = writeln!(f, "{}", agent_id);
        }
    }

    // Always wake the Right Sidebar immediately on ANY hook event
    notify_sidebar(&session_id, &target_yazi_id);

    if hook_event == "SessionStart" {
        let _ = crate::orchestrator::OrchestratorWatcher::new(&home);
        let agents_dir = Path::new(&home)
            .join(".claude/agents")
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        println!(
            "{{\"hookSpecificOutput\":{{\"hookEventName\":\"SessionStart\",\"watchPaths\":[\"{}\"],\"additionalContext\":\"cc-ide Orchestrator active: prefix Agent description/prompt with [@<model>:<effort>] (e.g. [@kimi-k3:high], [@:low]) to override routing, and use '@scout'/'@coder'/'@reviewer'/'@plan' macros in Workflow scripts.\"}}}}",
            agents_dir
        );
    }

    // Skip staged-only (unwritten) remote/review edits where disk file was not modified
    if extract_json_bool(&input, "staged") == Some(true) {
        return;
    }

    let tool_in = extract_json_object(&input, "tool_input").unwrap_or(&input);
    let mut raw_file = extract_json_str(tool_in, "file_path")
        .or_else(|| extract_json_str(tool_in, "filePath"))
        .or_else(|| extract_json_str(tool_in, "notebook_path"))
        .or_else(|| extract_json_str(tool_in, "path"))
        .or_else(|| extract_json_str(&input, "file_path"))
        .or_else(|| extract_json_str(&input, "filePath"))
        .unwrap_or_default();

    let to_abs = |f: &str| -> String {
        if Path::new(f).is_absolute() || cwd_path.is_empty() {
            f.to_string()
        } else {
            Path::new(&cwd_path).join(f).to_string_lossy().into_owned()
        }
    };

    let mut resolved_path = if raw_file.is_empty() {
        String::new()
    } else {
        to_abs(&raw_file)
    };
    if (resolved_path.is_empty() || !Path::new(&resolved_path).is_file())
        && matches!(tool_name.as_str(), "Glob" | "Grep")
    {
        if let Some(first_fn) = extract_first_filename_from_array(&input) {
            raw_file = first_fn;
            resolved_path = to_abs(&raw_file);
        }
    }

    if resolved_path.is_empty() || !Path::new(&resolved_path).is_file() {
        return;
    }

    let norm_base = if tab_cwd.is_empty() {
        &cwd_path
    } else {
        &tab_cwd
    };
    let file_path = normalize_path_under_cwd(&resolved_path, norm_base);
    if is_internal_or_temp_path(&file_path, &home) {
        return;
    }

    if target_yazi_id.is_empty() || !target_yazi_id.chars().all(|c| c.is_ascii_digit()) {
        return;
    }

    let (target_line, target_span) = extract_tool_line_target(&input, &tool_name, &file_path);
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Only write `main_last_file_<yazi_id>.json` when `agent_id` is empty (genuine Main thread!)
    if agent_id.is_empty() {
        let safe_fp = file_path.replace('\\', "\\\\").replace('"', "\\\"");
        let main_json = format!(
            "{{\"file\":\"{}\",\"tool\":\"{}\",\"line\":{},\"span\":{},\"ts\":{}}}\n",
            safe_fp, tool_name, target_line, target_span, now_s
        );
        let _ = fs::write(
            cdir.join(format!("main_last_file_{}.json", target_yazi_id)),
            main_json,
        );
    }

    let mut active_view =
        fs::read_to_string(cdir.join(format!("active_view_{}.txt", target_yazi_id)))
            .unwrap_or_else(|_| "main".to_string())
            .trim()
            .to_string();
    let ppid = current_ppid();
    let pid_cur = if ppid > 1 {
        fs::read_to_string(ipc_path(&format!("cur_pid_{}.txt", ppid))).ok()
    } else {
        None
    };
    let sess_cur = if !session_id.is_empty() {
        fs::read_to_string(ipc_path(&format!("cur_{}.txt", session_id))).ok()
    } else {
        None
    };
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

    let should_reveal = if agent_id.is_empty() {
        norm_view == "main"
    } else {
        norm_view == agent_id
    };
    if !should_reveal {
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
