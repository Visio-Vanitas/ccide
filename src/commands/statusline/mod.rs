//! `ccide-core statusline`: Sub-Millisecond Native Powerline Status Bar Renderer.

pub mod compact;

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ffi::notify_sidebar;
use crate::json::{extract_json_object, extract_json_str};
use crate::session::resolve_yazi_id_from_ppid;
use crate::tui::palette::Palette;
use crate::tui::width::floor_char_boundary;

pub use compact::{
    compute_context_budget, extract_statusline_context, load_compact_config, CompactConfig,
    ContextBudget, CtxTier,
};

fn fast_git_summary(cwd: &str) -> String {
    let mut cur = PathBuf::from(cwd);
    for _ in 0..8 {
        let head = cur.join(".git/HEAD");
        if let Ok(htxt) = fs::read_to_string(&head) {
            let trimmed = htxt.trim();
            let branch = trimmed
                .strip_prefix("ref: refs/heads/")
                .unwrap_or_else(|| &trimmed[..floor_char_boundary(trimmed, trimmed.len().min(7))]);
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
    let model_scope = extract_json_object(&input, "model").unwrap_or(&input);
    let model_id = extract_json_str(model_scope, "id").unwrap_or_default();
    let model = extract_json_str(model_scope, "display_name")
        .or_else(|| {
            if !model_id.is_empty() {
                Some(model_id.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "claude".to_string());

    let ws_scope = extract_json_object(&input, "workspace").unwrap_or(&input);
    let project_dir = extract_json_str(ws_scope, "project_dir").unwrap_or_default();
    let cwd = extract_json_str(ws_scope, "current_dir")
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

    let pal = Palette::load(&home);
    let sl = &pal.statusline;

    let (mode_word, c_mode_bg, c_mode_fg) =
        if raw_mode.contains("bypass") || raw_mode.contains("skip") {
            ("bypass", sl.bypass_bg, sl.bypass_fg)
        } else if raw_mode.contains("auto") {
            ("auto", sl.auto_bg, sl.auto_fg)
        } else if raw_mode.contains("plan") {
            ("plan", sl.plan_bg, sl.plan_fg)
        } else if raw_mode.contains("accept") {
            ("accept", sl.accept_bg, sl.accept_fg)
        } else {
            ("manual", sl.manual_bg, sl.manual_fg)
        };

    let (total_in, win_size) = extract_statusline_context(&input, &model_id, &model);
    let compact_cfg = load_compact_config(&home, &cwd, &project_dir, &model_id, &model);
    let budget = compute_context_budget(total_in, win_size, &compact_cfg);

    let (c_ctx_bg, c_ctx_fg) = match budget.tier {
        CtxTier::Crit => (sl.ctx_crit_bg, sl.ctx_crit_fg),
        CtxTier::Warn => (sl.ctx_warn_bg, sl.ctx_warn_fg),
        CtxTier::Ok => (sl.ctx_ok_bg, sl.ctx_ok_fg),
    };
    let ctx_text = format!(
        "CTX {}k/{}k ({}%)",
        budget.tokens_k, budget.compact_k, budget.used_pct
    );
    let git_text = fast_git_summary(&cwd);
    let sys_text = fs::read_to_string("/tmp/.cc-ide/sys_telemetry.cache")
        .unwrap_or_else(|_| "CPU --% \u{e0b1} RAM --%".to_string());

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
        session_id, model, mode_word, budget.used_pct, budget.tokens_k, budget.compact_k, safe_cwd, now_s
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

    let s0_bg = c_mode_bg.bg_ansi();
    let s0_fg = c_mode_fg.fg_ansi();
    let s0_sep = c_mode_bg.fg_ansi();
    let s1_bg = sl.model_bg.bg_ansi();
    let s1_fg = sl.model_fg.fg_ansi();
    let s1_sep = sl.model_bg.fg_ansi();
    let s2_bg = c_ctx_bg.bg_ansi();
    let s2_fg = c_ctx_fg.fg_ansi();
    let s2_sep = c_ctx_bg.fg_ansi();
    let s3_bg = sl.git_bg.bg_ansi();
    let s3_fg = sl.git_fg.fg_ansi();
    let s3_sep = sl.git_bg.fg_ansi();
    let s4_bg = sl.sys_bg.bg_ansi();
    let s4_fg = sl.sys_fg.fg_ansi();
    let s4_sep = sl.sys_bg.fg_ansi();

    println!(
        "{s0_bg}{s0_fg} \x1b[1m{mode_word} \x1b[22m{s1_bg}{s0_sep}\u{e0b0}{s1_bg}{s1_fg} \x1b[1m{model} \x1b[22m{s2_bg}{s1_sep}\u{e0b0}{s2_bg}{s2_fg} {ctx_text} {s3_bg}{s2_sep}\u{e0b0}{s3_bg}{s3_fg} {git_text} {s4_bg}{s3_sep}\u{e0b0}{s4_bg}{s4_fg} {sys_text} \x1b[0m{s4_sep}\u{e0b0}\x1b[0m"
    );
}
