//! `ccide-core yazi-hook`: Ultra-Fast Native `PostToolUse` / `SubagentStart` / `SubagentStop` Hook.
//!
//! 1. Resolves the live `yazi_id` belonging ONLY to the calling Claude session (`CC_YAZI_ID`,
//!    newest `tab_shell_<yazi_id>.txt` via `getppid()` ancestry, or `session_<sessionId>.yazi`).
//! 2. Wakes `ccide-core sidebar` immediately via `SIGUSR2` (`< 0.05ms`) on every tool/task/subagent event.
//! 3. Extracts the exact modified/read `(line, span)` target (`structuredPatch` / `new_string` / `offset` / `line`),
//!    persists `main_last_file_<yazi_id>.json`, and auto-scrolls + highlights the hunk in `Yazi`.

use std::env;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ffi::notify_sidebar;
use crate::json::extract_json_str;
use crate::session::{
    emit_yazi_reveal_at, extract_tool_line_target, normalize_path_under_cwd, resolve_live_yazi_tab,
};

pub fn run() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if input.is_empty() {
        return;
    }

    let tool_name = extract_json_str(&input, "tool_name").unwrap_or_default();
    let session_id = extract_json_str(&input, "session_id").unwrap_or_default();
    let perm_mode = extract_json_str(&input, "permission_mode").unwrap_or_default();
    let cwd_path = extract_json_str(&input, "cwd").unwrap_or_default();
    let raw_file = extract_json_str(&input, "file_path")
        .or_else(|| extract_json_str(&input, "filePath"))
        .or_else(|| extract_json_str(&input, "notebook_path"))
        .or_else(|| extract_json_str(&input, "path"))
        .unwrap_or_default();

    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let cache_dir = format!("{}/.cache/cc-ide", home);

    let (target_yazi_id, tab_cwd) = resolve_live_yazi_tab(&cache_dir, &session_id, &cwd_path);
    if !target_yazi_id.is_empty() && !session_id.is_empty() {
        let _ = fs::write(
            format!("{}/session_{}.yazi", cache_dir, session_id),
            &target_yazi_id,
        );
    }

    notify_sidebar(&session_id, &target_yazi_id);

    if raw_file.is_empty() {
        return;
    }
    let resolved_path = if Path::new(&raw_file).is_absolute() || cwd_path.is_empty() {
        raw_file
    } else {
        format!("{}/{}", cwd_path.trim_end_matches('/'), raw_file)
    };
    if !Path::new(&resolved_path).is_file() {
        return;
    }
    let norm_base = if tab_cwd.is_empty() {
        &cwd_path
    } else {
        &tab_cwd
    };
    let file_path = normalize_path_under_cwd(&resolved_path, norm_base);

    if file_path.starts_with(&format!("{}/.claude/projects/", home))
        || file_path.starts_with("/tmp/")
        || file_path.starts_with("/private/tmp/")
        || file_path.contains("/.lib/")
    {
        return;
    }

    let _ = fs::create_dir_all(format!("{}/status", cache_dir));
    if !perm_mode.is_empty() && !session_id.is_empty() {
        let _ = fs::write(
            format!("{}/status/mode_{}.cache", cache_dir, session_id),
            &perm_mode,
        );
    }

    if target_yazi_id.is_empty() || !target_yazi_id.chars().all(|c| c.is_ascii_digit()) {
        return;
    }

    let (target_line, target_span) = extract_tool_line_target(&input, &tool_name, &file_path);
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let safe_fp = file_path.replace('\\', "\\\\").replace('"', "\\\"");
    let main_json = format!(
        "{{\"file\":\"{}\",\"tool\":\"{}\",\"line\":{},\"span\":{},\"ts\":{}}}\n",
        safe_fp, tool_name, target_line, target_span, now_s
    );
    let _ = fs::write(
        format!("{}/main_last_file_{}.json", cache_dir, target_yazi_id),
        main_json,
    );

    let mut active_view =
        fs::read_to_string(format!("{}/active_view_{}.txt", cache_dir, target_yazi_id))
            .unwrap_or_else(|_| "main".to_string())
            .trim()
            .to_string();
    let ppid = unsafe { crate::ffi::getppid() as u32 };
    let pid_cur = if ppid > 1 {
        fs::read_to_string(format!("/tmp/.cc-ide/cur_pid_{}.txt", ppid)).ok()
    } else {
        None
    };
    let sess_cur = if !session_id.is_empty() {
        fs::read_to_string(format!("/tmp/.cc-ide/cur_{}.txt", session_id)).ok()
    } else {
        None
    };
    if let Some(cur_native) = pid_cur.or(sess_cur) {
        let cn = cur_native.lines().next().unwrap_or("").trim();
        if !cn.is_empty() {
            active_view = cn.to_string();
        }
    }

    if !active_view.is_empty() && active_view != "main" {
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
