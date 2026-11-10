//! `ccide-core statusline`: Sub-Millisecond Native Powerline Status Bar Renderer.
//!
//! Reads `.git/HEAD` and `/tmp/.cc-ide/sys_telemetry.cache` directly without spawning
//! `git`, `ps`, `vm_stat`, or `awk`, and exports per-session isolated model state
//! (`live_sess_<sessionId>.json` and `live_tab_<yaziId>.json`) so the Right Sidebar
//! updates its model logo instantaneously when `/model` is switched.

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ffi::notify_sidebar;
use crate::json::{extract_json_str, extract_json_u64};
use crate::session::resolve_yazi_id_from_ppid;

fn fast_git_summary(cwd: &str) -> String {
    let mut cur = PathBuf::from(cwd);
    for _ in 0..8 {
        let head = cur.join(".git/HEAD");
        if let Ok(htxt) = fs::read_to_string(&head) {
            let trimmed = htxt.trim();
            let branch = trimmed
                .strip_prefix("ref: refs/heads/")
                .unwrap_or_else(|| &trimmed[..trimmed.floor_char_boundary(trimmed.len().min(7))]);
            return format!("\u{e0a0} {}", branch);
        }
        if !cur.pop() {
            break;
        }
    }
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("~")
        .to_string()
}

pub fn run() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);

    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let base_cache = format!("{}/.cache/cc-ide", home);
    let status_cache = format!("{}/status", base_cache);
    let _ = fs::create_dir_all(&status_cache);

    let session_id = extract_json_str(&input, "session_id").unwrap_or_default();
    let model = extract_json_str(&input, "display_name")
        .or_else(|| extract_json_str(&input, "id"))
        .unwrap_or_else(|| "claude".to_string());
    let cwd = extract_json_str(&input, "current_dir")
        .or_else(|| extract_json_str(&input, "cwd"))
        .unwrap_or_else(|| {
            env::current_dir()
                .ok()
                .and_then(|p| p.to_str().map(String::from))
                .unwrap_or_default()
        });
    let mut raw_mode = extract_json_str(&input, "permission_mode")
        .or_else(|| extract_json_str(&input, "mode"))
        .unwrap_or_default();

    if raw_mode.is_empty() && !session_id.is_empty() {
        raw_mode = fs::read_to_string(format!("{}/mode_{}.cache", status_cache, session_id))
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    if raw_mode.is_empty() {
        raw_mode = "bypass".to_string();
    }

    let (mode_word, bg0, fg0) = if raw_mode.contains("bypass") || raw_mode.contains("skip") {
        ("bypass", 131, 255)
    } else if raw_mode.contains("auto") {
        ("auto", 61, 255)
    } else if raw_mode.contains("plan") {
        ("plan", 30, 255)
    } else if raw_mode.contains("accept") {
        ("accept", 29, 255)
    } else {
        ("manual", 239, 252)
    };

    let used_pct = extract_json_u64(&input, "used_percentage").unwrap_or(0);
    let in_tok = extract_json_u64(&input, "input_tokens").unwrap_or(0);
    let cc_tok = extract_json_u64(&input, "cache_creation_input_tokens").unwrap_or(0);
    let cr_tok = extract_json_u64(&input, "cache_read_input_tokens").unwrap_or(0);
    let win_size = extract_json_u64(&input, "context_window_size").unwrap_or(950_000);

    let mut total_in = in_tok + cc_tok + cr_tok;
    if total_in == 0 && used_pct > 0 {
        total_in = win_size * used_pct / 100;
    }
    let tokens_k = total_in / 1000;
    let compact_k = win_size * 50 / 100_000;

    let (bg2, fg2) = if used_pct >= 40 {
        (131, 255)
    } else if used_pct >= 25 {
        (136, 255)
    } else {
        (240, 253)
    };
    let ctx_text = format!("CTX {}k/{}k ({}%)", tokens_k, compact_k, used_pct);
    let git_text = fast_git_summary(&cwd);
    let sys_text = fs::read_to_string("/tmp/.cc-ide/sys_telemetry.cache")
        .unwrap_or_else(|_| "CPU --% \u{e0b1} RAM --%".to_string());

    // Resolve per-tab `yazi_id` and export strictly isolated per-session & per-tab live model state
    let mut yid = env::var("CC_YAZI_ID").unwrap_or_default();
    if yid.is_empty() && !session_id.is_empty() {
        yid = fs::read_to_string(format!("{}/session_{}.yazi", base_cache, session_id))
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    if yid.is_empty() {
        yid = resolve_yazi_id_from_ppid(&base_cache);
    }
    if !yid.is_empty() && !session_id.is_empty() {
        let _ = fs::write(format!("{}/session_{}.yazi", base_cache, session_id), &yid);
    }

    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe_cwd = cwd.replace('\\', "\\\\").replace('"', "\\\"");
    let live_json = format!(
        "{{\"session_id\":\"{}\",\"model\":\"{}\",\"mode\":\"{}\",\"used_pct\":{},\"tokens_k\":{},\"compact_k\":{},\"cwd\":\"{}\",\"ts\":{}}}",
        session_id, model, mode_word, used_pct, tokens_k, compact_k, safe_cwd, now_s
    );
    if !session_id.is_empty() {
        let _ = fs::write(
            format!("{}/live_sess_{}.json", base_cache, session_id),
            &live_json,
        );
    }
    if !yid.is_empty() {
        let _ = fs::write(format!("{}/live_tab_{}.json", base_cache, yid), &live_json);
    }
    notify_sidebar(&session_id, &yid);

    println!(
        "\x1b[48;5;{bg0}m\x1b[38;5;{fg0}m \x1b[1m{mode_word} \x1b[22m\x1b[48;5;24m\x1b[38;5;{bg0}m\u{e0b0}\x1b[48;5;24m\x1b[38;5;255m \x1b[1m{model} \x1b[22m\x1b[48;5;{bg2}m\x1b[38;5;24m\u{e0b0}\x1b[48;5;{bg2}m\x1b[38;5;{fg2}m {ctx_text} \x1b[48;5;237m\x1b[38;5;{bg2}m\u{e0b0}\x1b[48;5;237m\x1b[38;5;250m {git_text} \x1b[48;5;235m\x1b[38;5;237m\u{e0b0}\x1b[48;5;235m\x1b[38;5;247m {sys_text} \x1b[0m\x1b[38;5;235m\u{e0b0}\x1b[0m"
    );
}
