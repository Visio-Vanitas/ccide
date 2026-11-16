//! Claude Code Auto-Compaction Window & Threshold Budget Resolver (`src/budget/`).
//!
//! Leaf module shared by `crate::tui::context` and `crate::commands::statusline`.

pub mod config;

#[allow(unused_imports)]
pub use config::{load_compact_config, parse_compact_window_spec, CompactConfig};

use crate::budget::config::{
    DEFAULT_CONTEXT_WINDOW, MAX_OUTPUT_RESERVE_CAP, SUMMARY_BUFFER_TOKENS,
};
use crate::json::{extract_json_f64, extract_json_object, extract_json_u64};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtxTier {
    Ok,
    Warn,
    Crit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextBudget {
    pub limit_tokens: u64,
    pub tokens_k: u64,
    pub compact_k: u64,
    pub used_pct: u64,
    pub tier: CtxTier,
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
