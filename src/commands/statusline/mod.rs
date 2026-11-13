//! `ccide-core statusline`: Sub-Millisecond Native Powerline Status Bar Renderer.
//!
//! Parses Claude Code 2.1.3xx `statusLine` JSON including `context_window.context_window_size`,
//! `rate_limits.{five_hour,seven_day}`, `cost.{total_cost_usd,total_lines_added,total_lines_removed}`,
//! `fast_mode`, `effort.level`, and linked git worktrees (`workspace.git_worktree` / `.git` file).

pub mod compact;

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ffi::notify_sidebar;
use crate::json::{
    extract_json_bool, extract_json_f64, extract_json_object, extract_json_str, extract_json_u64,
};
use crate::platform::{cache_dir, ipc_path, user_home_dir};
use crate::session::resolve_yazi_id_from_ppid;
use crate::tui::palette::Palette;
use crate::tui::width::floor_char_boundary;

pub use compact::{
    compute_context_budget, extract_statusline_context, load_compact_config, CompactConfig,
    ContextBudget, CtxTier,
};

fn read_git_head_branch(dot_git: &Path) -> Option<String> {
    let head_path = if dot_git.is_dir() {
        dot_git.join("HEAD")
    } else if dot_git.is_file() {
        let ptr = fs::read_to_string(dot_git).ok()?;
        let gdir = ptr.trim().strip_prefix("gitdir:")?.trim();
        let resolved = if Path::new(gdir).is_absolute() {
            PathBuf::from(gdir)
        } else {
            dot_git.parent()?.join(gdir)
        };
        resolved.join("HEAD")
    } else {
        return None;
    };
    let htxt = fs::read_to_string(head_path).ok()?;
    let trimmed = htxt.trim();
    let br = trimmed
        .strip_prefix("ref: refs/heads/")
        .unwrap_or_else(|| &trimmed[..floor_char_boundary(trimmed, trimmed.len().min(7))]);
    Some(br.to_string())
}

fn fast_git_summary(cwd: &str, wt_hint: &str) -> String {
    let mut cur = PathBuf::from(cwd);
    for _ in 0..8 {
        let dg = cur.join(".git");
        if let Some(branch) = read_git_head_branch(&dg) {
            if !wt_hint.is_empty() && wt_hint != branch {
                return format!("\u{e0a0} {}[{}]", branch, wt_hint);
            }
            return format!("\u{e0a0} {}", branch);
        }
        if !cur.pop() {
            break;
        }
    }
    if !wt_hint.is_empty() {
        return format!("\u{e0a0} {}", wt_hint);
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

    let home = user_home_dir();
    let base_cache = cache_dir(&home);
    let base_cache_s = base_cache.to_string_lossy().into_owned();
    let status_cache = base_cache.join("status");
    let _ = fs::create_dir_all(&status_cache);

    let session_id = extract_json_str(&input, "session_id").unwrap_or_default();
    let model_scope = extract_json_object(&input, "model").unwrap_or(&input);
    let model_id = extract_json_str(model_scope, "id").unwrap_or_default();
    let model = extract_json_str(model_scope, "display_name")
        .or_else(|| (!model_id.is_empty()).then(|| model_id.clone()))
        .unwrap_or_else(|| "claude".to_string());

    let ws_scope = extract_json_object(&input, "workspace").unwrap_or(&input);
    let project_dir = extract_json_str(ws_scope, "project_dir").unwrap_or_default();
    let git_wt = extract_json_str(ws_scope, "git_worktree")
        .or_else(|| {
            extract_json_object(&input, "worktree").and_then(|w| extract_json_str(w, "name"))
        })
        .unwrap_or_default();
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
        raw_mode = fs::read_to_string(status_cache.join(format!("mode_{}.cache", session_id)))
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    if raw_mode.is_empty() {
        raw_mode = "bypass".to_string();
    }

    let fast_mode = extract_json_bool(&input, "fast_mode").unwrap_or(false);
    let effort = extract_json_object(&input, "effort")
        .and_then(|o| extract_json_str(o, "level"))
        .unwrap_or_default();

    let (mut cc_cost_usd, mut lines_add, mut lines_del) = (0.0f64, 0u64, 0u64);
    if let Some(cost_obj) = extract_json_object(&input, "cost") {
        cc_cost_usd = extract_json_f64(cost_obj, "total_cost_usd").unwrap_or(0.0);
        lines_add = extract_json_u64(cost_obj, "total_lines_added").unwrap_or(0);
        lines_del = extract_json_u64(cost_obj, "total_lines_removed").unwrap_or(0);
    }

    let (mut rl_5h_pct, mut rl_7d_pct): (Option<u64>, Option<u64>) = (None, None);
    if let Some(rl_obj) = extract_json_object(&input, "rate_limits") {
        if let Some(fh) = extract_json_object(rl_obj, "five_hour") {
            rl_5h_pct =
                extract_json_f64(fh, "used_percentage").map(|v| v.round().clamp(0.0, 100.0) as u64);
        }
        if let Some(sd) = extract_json_object(rl_obj, "seven_day") {
            rl_7d_pct =
                extract_json_f64(sd, "used_percentage").map(|v| v.round().clamp(0.0, 100.0) as u64);
        }
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
    let mut ctx_text = format!(
        "CTX {}k/{}k ({}%)",
        budget.tokens_k, budget.compact_k, budget.used_pct
    );
    match (rl_5h_pct, rl_7d_pct) {
        (Some(h5), Some(d7)) => ctx_text.push_str(&format!(" \u{e0b1} 5h:{h5}% 7d:{d7}%")),
        (Some(h5), None) => ctx_text.push_str(&format!(" \u{e0b1} 5h:{h5}%")),
        (None, Some(d7)) => ctx_text.push_str(&format!(" \u{e0b1} 7d:{d7}%")),
        (None, None) => {}
    }

    let mut git_text = fast_git_summary(&cwd, &git_wt);
    if lines_add > 0 || lines_del > 0 {
        git_text.push_str(&format!(" +{}/-{}", lines_add, lines_del));
    }

    let model_seg = match (fast_mode, !effort.is_empty()) {
        (true, true) => format!("{}·fast·{}", model, effort),
        (true, false) => format!("{}·fast", model),
        (false, true) => format!("{}·{}", model, effort),
        (false, false) => model.clone(),
    };

    let sys_text = fs::read_to_string(ipc_path("sys_telemetry.cache"))
        .unwrap_or_else(|_| "CPU --% \u{e0b1} RAM --%".to_string());

    let mut yid = env::var("CC_YAZI_ID").unwrap_or_default();
    if yid.is_empty() && !session_id.is_empty() {
        yid = fs::read_to_string(base_cache.join(format!("session_{}.yazi", session_id)))
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    if yid.is_empty() {
        yid = resolve_yazi_id_from_ppid(&base_cache_s);
    }
    if !yid.is_empty() && !session_id.is_empty() {
        let _ = fs::write(
            base_cache.join(format!("session_{}.yazi", session_id)),
            &yid,
        );
    }

    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe_cwd = cwd.replace('\\', "\\\\").replace('"', "\\\"");
    let safe_mod = model.replace('\\', "\\\\").replace('"', "\\\"");
    let safe_mid = model_id.replace('\\', "\\\\").replace('"', "\\\"");
    let rl5_json = rl_5h_pct
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string());
    let rl7_json = rl_7d_pct
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string());
    let live_json = format!(
        "{{\"session_id\":\"{}\",\"model\":\"{}\",\"model_id\":\"{}\",\"mode\":\"{}\",\"win_tokens\":{},\"used_pct\":{},\"tokens_k\":{},\"compact_k\":{},\"cc_cost_usd\":{:.6},\"lines_add\":{},\"lines_del\":{},\"rl_5h_pct\":{},\"rl_7d_pct\":{},\"fast_mode\":{},\"effort\":\"{}\",\"cwd\":\"{}\",\"ts\":{}}}",
        session_id,
        safe_mod,
        safe_mid,
        mode_word,
        win_size,
        budget.used_pct,
        budget.tokens_k,
        budget.compact_k,
        cc_cost_usd,
        lines_add,
        lines_del,
        rl5_json,
        rl7_json,
        if fast_mode { "true" } else { "false" },
        effort,
        safe_cwd,
        now_s
    );
    if !session_id.is_empty() {
        let _ = fs::write(
            base_cache.join(format!("live_sess_{}.json", session_id)),
            &live_json,
        );
    }
    if !yid.is_empty() {
        let _ = fs::write(
            base_cache.join(format!("live_tab_{}.json", yid)),
            &live_json,
        );
    }
    notify_sidebar(&session_id, &yid);

    let (s0_bg, s0_fg, s0_sep) = (
        c_mode_bg.bg_ansi(),
        c_mode_fg.fg_ansi(),
        c_mode_bg.fg_ansi(),
    );
    let (s1_bg, s1_fg, s1_sep) = (
        sl.model_bg.bg_ansi(),
        sl.model_fg.fg_ansi(),
        sl.model_bg.fg_ansi(),
    );
    let (s2_bg, s2_fg, s2_sep) = (c_ctx_bg.bg_ansi(), c_ctx_fg.fg_ansi(), c_ctx_bg.fg_ansi());
    let (s3_bg, s3_fg, s3_sep) = (
        sl.git_bg.bg_ansi(),
        sl.git_fg.fg_ansi(),
        sl.git_bg.fg_ansi(),
    );
    let (s4_bg, s4_fg, s4_sep) = (
        sl.sys_bg.bg_ansi(),
        sl.sys_fg.fg_ansi(),
        sl.sys_bg.fg_ansi(),
    );

    println!(
        "{s0_bg}{s0_fg} \x1b[1m{mode_word} \x1b[22m{s1_bg}{s0_sep}\u{e0b0}{s1_bg}{s1_fg} \x1b[1m{model_seg} \x1b[22m{s2_bg}{s1_sep}\u{e0b0}{s2_bg}{s2_fg} {ctx_text} {s3_bg}{s2_sep}\u{e0b0}{s3_bg}{s3_fg} {git_text} {s4_bg}{s3_sep}\u{e0b0}{s4_bg}{s4_fg} {sys_text} \x1b[0m{s4_sep}\u{e0b0}\x1b[0m"
    );
}
