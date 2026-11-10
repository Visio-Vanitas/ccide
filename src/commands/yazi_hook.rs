//! `ccide-core yazi-hook`: Ultra-Fast Native `PostToolUse` Hook.
//!
//! 1. Resolves the exact `yazi_id` belonging ONLY to the calling Claude session (`CC_YAZI_ID`,
//!    `session_<sessionId>.yazi`, or `getppid()` -> `tab_shell_<yazi_id>.txt`).
//! 2. Records `main`'s latest operated file in `main_last_file_<yazi_id>.json`.
//! 3. Emits `ya emit-to <yazi_id> reveal <file>` ONLY when `main` is the active selected session.

use std::env;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ffi::notify_sidebar;
use crate::json::extract_json_str;
use crate::session::resolve_yazi_id_from_ppid;

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
        .or_else(|| extract_json_str(&input, "notebook_path"))
        .or_else(|| extract_json_str(&input, "path"))
        .unwrap_or_default();

    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let cache_dir = format!("{}/.cache/cc-ide", home);
    let mut target_yazi_id = env::var("CC_YAZI_ID").unwrap_or_default();
    if target_yazi_id.is_empty() && !session_id.is_empty() {
        target_yazi_id = fs::read_to_string(format!("{}/session_{}.yazi", cache_dir, session_id))
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    if target_yazi_id.is_empty() {
        target_yazi_id = resolve_yazi_id_from_ppid(&cache_dir);
        if !target_yazi_id.is_empty() && !session_id.is_empty() {
            let _ = fs::write(
                format!("{}/session_{}.yazi", cache_dir, session_id),
                &target_yazi_id,
            );
        }
    }

    // Always wake the Right Sidebar immediately on ANY hook event (SubagentStart, SubagentStop, PostToolUse)
    notify_sidebar(&session_id, &target_yazi_id);

    if raw_file.is_empty() {
        return;
    }
    let file_path = if Path::new(&raw_file).is_absolute() || cwd_path.is_empty() {
        raw_file
    } else {
        format!("{}/{}", cwd_path.trim_end_matches('/'), raw_file)
    };
    if !Path::new(&file_path).exists() {
        return;
    }

    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    if file_path.starts_with(&format!("{}/.claude/projects/", home))
        || file_path.starts_with("/tmp/")
        || file_path.starts_with("/private/tmp/")
    {
        return;
    }

    let cache_dir = format!("{}/.cache/cc-ide", home);
    let _ = fs::create_dir_all(format!("{}/status", cache_dir));

    if !perm_mode.is_empty() && !session_id.is_empty() {
        let _ = fs::write(
            format!("{}/status/mode_{}.cache", cache_dir, session_id),
            &perm_mode,
        );
    }

    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if target_yazi_id.is_empty() || !target_yazi_id.chars().all(|c| c.is_ascii_digit()) {
        return;
    }

    let safe_fp = file_path.replace('\\', "\\\\").replace('"', "\\\"");
    let main_json = format!(
        "{{\"file\":\"{}\",\"tool\":\"{}\",\"ts\":{}}}\n",
        safe_fp, tool_name, now_s
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
    if !session_id.is_empty()
        && let Ok(cur_native) = fs::read_to_string(format!("/tmp/.cc-ide/cur_{}.txt", session_id))
    {
        let cn = cur_native.lines().next().unwrap_or("").trim();
        if !cn.is_empty() {
            active_view = cn.to_string();
        }
    }

    if !active_view.is_empty() && active_view != "main" {
        return;
    }

    let ya_bin = if Path::new("/opt/homebrew/bin/ya").exists() {
        "/opt/homebrew/bin/ya"
    } else {
        "ya"
    };
    let _ = Command::new(ya_bin)
        .args(["emit-to", &target_yazi_id, "reveal", &file_path])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
