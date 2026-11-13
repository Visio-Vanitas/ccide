//! Claude Code Auto-Compaction Window & Threshold Budget Resolver.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::json::{
    extract_json_bool, extract_json_f64, extract_json_object, extract_json_str, extract_json_u64,
};

const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;
const MIN_AUTO_COMPACT_WINDOW: u64 = 100_000;
const MAX_AUTO_COMPACT_WINDOW: u64 = 1_000_000;
const MAX_OUTPUT_RESERVE_CAP: u64 = 20_000;
const SUMMARY_BUFFER_TOKENS: u64 = 13_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtxTier {
    Ok,
    Warn,
    Crit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowSetting {
    Auto,
    Tokens(u64),
}

#[derive(Clone, Debug)]
pub struct CompactConfig {
    pub auto_compact_enabled: bool,
    pub env_auto_compact_window: Option<u64>,
    pub settings_auto_compact_window: Option<u64>,
    pub pct_override: Option<f64>,
    pub max_output_tokens: Option<u64>,
    pub blocking_limit_override: Option<u64>,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            auto_compact_enabled: true,
            env_auto_compact_window: None,
            settings_auto_compact_window: None,
            pct_override: None,
            max_output_tokens: None,
            blocking_limit_override: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextBudget {
    pub limit_tokens: u64,
    pub tokens_k: u64,
    pub compact_k: u64,
    pub used_pct: u64,
    pub tier: CtxTier,
}

fn is_truthy_env(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub fn parse_compact_window_spec(
    raw: &str,
    bare_thousands: bool,
    clamp_out_of_range: bool,
) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        return None;
    }
    let last = trimmed.as_bytes()[trimmed.len() - 1].to_ascii_lowercase();
    let (num_part, mult) = match last {
        b'm' => (&trimmed[..trimmed.len() - 1], Some(1_000_000.0)),
        b'k' => (&trimmed[..trimmed.len() - 1], Some(1_000.0)),
        _ => (trimmed, None),
    };
    let parsed = num_part.trim().parse::<f64>().ok()?;
    if !parsed.is_finite() {
        return None;
    }
    let mut val = mult.map(|m| parsed * m).unwrap_or(parsed);
    if mult.is_none() && bare_thousands && (100.0..=1_000.0).contains(&val) {
        val *= 1_000.0;
    }
    if val < 1.0 {
        return None;
    }
    let rounded = val.round() as u64;
    if rounded < MIN_AUTO_COMPACT_WINDOW {
        return if clamp_out_of_range {
            Some(MIN_AUTO_COMPACT_WINDOW)
        } else {
            None
        };
    }
    if rounded > MAX_AUTO_COMPACT_WINDOW {
        return if clamp_out_of_range {
            Some(MAX_AUTO_COMPACT_WINDOW)
        } else {
            None
        };
    }
    Some(rounded)
}

fn parse_window_setting_from_scope(scope: &str) -> Option<WindowSetting> {
    if let Some(s) = extract_json_str(scope, "autoCompactWindow") {
        if s.trim().eq_ignore_ascii_case("auto") {
            return Some(WindowSetting::Auto);
        }
        return parse_compact_window_spec(&s, true, false).map(WindowSetting::Tokens);
    }
    extract_json_u64(scope, "autoCompactWindow")
        .and_then(|n| parse_compact_window_spec(&n.to_string(), true, false))
        .map(WindowSetting::Tokens)
}

fn extract_file_window_setting(settings_txt: &str, model_keys: &[String]) -> Option<WindowSetting> {
    if let Some(ms_obj) = extract_json_object(settings_txt, "modelSettings") {
        for key in model_keys {
            if !key.is_empty() {
                if let Some(m_obj) = extract_json_object(ms_obj, key) {
                    if let Some(ws) = parse_window_setting_from_scope(m_obj) {
                        return Some(ws);
                    }
                }
            }
        }
    }
    parse_window_setting_from_scope(settings_txt)
}

pub fn extract_statusline_context(input: &str, model_id: &str, model_display: &str) -> (u64, u64) {
    let default_win = if model_id.to_ascii_lowercase().contains("[1m]")
        || model_display.to_ascii_lowercase().contains("[1m]")
    {
        1_000_000
    } else {
        DEFAULT_CONTEXT_WINDOW
    };
    if let Some(ctx_scope) = extract_json_object(input, "context_window") {
        let win_size = extract_json_u64(ctx_scope, "context_window_size").unwrap_or(default_win);
        let mut total_in = if let Some(u) = extract_json_object(ctx_scope, "current_usage") {
            extract_json_u64(u, "input_tokens").unwrap_or(0)
                + extract_json_u64(u, "cache_creation_input_tokens").unwrap_or(0)
                + extract_json_u64(u, "cache_read_input_tokens").unwrap_or(0)
        } else {
            extract_json_u64(ctx_scope, "total_input_tokens").unwrap_or(0)
        };
        if total_in == 0 {
            if let Some(raw_pct) = extract_json_f64(ctx_scope, "used_percentage") {
                if raw_pct > 0.0 {
                    total_in = ((win_size as f64) * (raw_pct / 100.0)).round() as u64;
                }
            }
        }
        (total_in, win_size)
    } else {
        let win_size = extract_json_u64(input, "context_window_size").unwrap_or(default_win);
        let mut total_in = extract_json_u64(input, "input_tokens").unwrap_or(0)
            + extract_json_u64(input, "cache_creation_input_tokens").unwrap_or(0)
            + extract_json_u64(input, "cache_read_input_tokens").unwrap_or(0);
        if total_in == 0 {
            if let Some(raw_pct) = extract_json_f64(input, "used_percentage") {
                if raw_pct > 0.0 {
                    total_in = ((win_size as f64) * (raw_pct / 100.0)).round() as u64;
                }
            }
        }
        (total_in, win_size)
    }
}

pub fn load_compact_config(
    home: &str,
    cwd: &str,
    project_dir: &str,
    model_id: &str,
    model_display: &str,
) -> CompactConfig {
    let mut paths: Vec<PathBuf> = Vec::new();
    for dir in [cwd, project_dir] {
        if !dir.is_empty() {
            for rel in [".claude/settings.local.json", ".claude/settings.json"] {
                let p = Path::new(dir).join(rel);
                if !paths.iter().any(|x| x == &p) {
                    paths.push(p);
                }
            }
        }
    }
    if !home.is_empty() {
        paths.push(Path::new(home).join(".claude/settings.json"));
    }
    let contents: Vec<String> = paths
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .collect();

    let resolve_var = |name: &str| -> Option<String> {
        if let Ok(v) = env::var(name) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
        for txt in &contents {
            if let Some(env_obj) = extract_json_object(txt, "env") {
                if let Some(v) = extract_json_str(env_obj, name) {
                    if !v.trim().is_empty() {
                        return Some(v.trim().to_string());
                    }
                }
                if let Some(n) = extract_json_u64(env_obj, name) {
                    return Some(n.to_string());
                }
            }
        }
        None
    };

    let disabled = resolve_var("DISABLE_COMPACT")
        .map(|v| is_truthy_env(&v))
        .unwrap_or(false)
        || resolve_var("DISABLE_AUTO_COMPACT")
            .map(|v| is_truthy_env(&v))
            .unwrap_or(false);

    let auto_compact_enabled = if disabled {
        false
    } else {
        contents
            .iter()
            .find_map(|t| extract_json_bool(t, "autoCompactEnabled"))
            .or_else(|| {
                fs::read_to_string(Path::new(home).join(".claude.json"))
                    .ok()
                    .and_then(|t| extract_json_bool(&t, "autoCompactEnabled"))
            })
            .unwrap_or(true)
    };

    let mut model_keys = Vec::new();
    for raw in [model_id, model_display] {
        let t = raw.trim();
        if !t.is_empty() {
            model_keys.push(t.to_string());
            let base = t.split('[').next().unwrap_or(t).trim();
            if !base.is_empty() && base != t {
                model_keys.push(base.to_string());
            }
        }
    }
    let settings_auto_compact_window = contents
        .iter()
        .find_map(|txt| extract_file_window_setting(txt, &model_keys))
        .and_then(|ws| match ws {
            WindowSetting::Auto => None,
            WindowSetting::Tokens(t) => Some(t),
        });

    CompactConfig {
        auto_compact_enabled,
        env_auto_compact_window: resolve_var("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
            .and_then(|s| parse_compact_window_spec(&s, false, true)),
        settings_auto_compact_window,
        pct_override: resolve_var("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE")
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0 && *v <= 100.0),
        max_output_tokens: resolve_var("CLAUDE_CODE_MAX_OUTPUT_TOKENS")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&v| v > 0),
        blocking_limit_override: resolve_var("CLAUDE_CODE_BLOCKING_LIMIT_OVERRIDE")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&v| v > 0),
    }
}

pub fn compute_context_budget(
    total_in: u64,
    model_win_size: u64,
    cfg: &CompactConfig,
) -> ContextBudget {
    let base_win = if model_win_size > 0 {
        model_win_size
    } else {
        DEFAULT_CONTEXT_WINDOW
    };
    let resolved_win = if let Some(ew) = cfg.env_auto_compact_window {
        base_win.min(ew)
    } else if cfg.auto_compact_enabled {
        cfg.settings_auto_compact_window
            .map(|sw| base_win.min(sw))
            .unwrap_or(base_win)
    } else {
        base_win
    };

    let reserve = cfg
        .max_output_tokens
        .unwrap_or(MAX_OUTPUT_RESERVE_CAP)
        .min(MAX_OUTPUT_RESERVE_CAP);
    let eff_win = resolved_win.saturating_sub(reserve);

    let limit_tokens = if cfg.auto_compact_enabled {
        let base_thr = eff_win.saturating_sub(SUMMARY_BUFFER_TOKENS);
        cfg.pct_override
            .map(|pct| (((eff_win as f64) * (pct / 100.0)).floor() as u64).min(base_thr))
            .unwrap_or(base_thr)
    } else {
        cfg.blocking_limit_override.unwrap_or(eff_win)
    };

    let tokens_k = total_in / 1_000;
    let compact_k = (limit_tokens / 1_000).max(1);
    let used_pct = if limit_tokens > 0 {
        ((total_in as f64 / limit_tokens as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u64
    } else {
        0
    };
    let tier = if used_pct >= 80 {
        CtxTier::Crit
    } else if used_pct >= 50 {
        CtxTier::Warn
    } else {
        CtxTier::Ok
    };

    ContextBudget {
        limit_tokens,
        tokens_k,
        compact_k,
        used_pct,
        tier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_exact_thresholds_and_isolates_rate_limits() {
        let b200 = compute_context_budget(83_500, 200_000, &CompactConfig::default());
        assert_eq!(
            (b200.compact_k, b200.used_pct, b200.tier),
            (167, 50, CtxTier::Warn)
        );

        let cfg = CompactConfig {
            env_auto_compact_window: Some(950_000),
            pct_override: Some(90.0),
            ..CompactConfig::default()
        };
        let b1m = compute_context_budget(677_000, 1_000_000, &cfg);
        assert_eq!(
            (b1m.compact_k, b1m.used_pct, b1m.tier),
            (837, 81, CtxTier::Crit)
        );

        let payload = r#"{"context_window":{"context_window_size":1000000,"current_usage":null,"used_percentage":null},"rate_limits":{"five_hour":{"used_percentage":68}}}"#;
        assert_eq!(
            extract_statusline_context(payload, "fable[1m]", "Fable"),
            (0, 1_000_000)
        );
    }
}
