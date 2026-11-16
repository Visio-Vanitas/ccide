//! Auto-Compaction Configuration & Window Spec Parser (`src/budget/config.rs`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::json::{extract_json_bool, extract_json_object, extract_json_str, extract_json_u64};

pub(super) const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;
const MIN_AUTO_COMPACT_WINDOW: u64 = 100_000;
const MAX_AUTO_COMPACT_WINDOW: u64 = 1_000_000;
pub(super) const MAX_OUTPUT_RESERVE_CAP: u64 = 20_000;
pub(super) const SUMMARY_BUFFER_TOKENS: u64 = 13_000;

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
