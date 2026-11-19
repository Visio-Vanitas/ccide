//! Multi-Agent (`agy` / `codex`) Context & Cost Snapshot Builder (`src/tui/context/agent_ctx.rs`).

use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::budget::{compute_context_budget, CompactConfig};
use crate::session::{
    classify_model_family_for, codex_config, is_valid_model_name, short_model_label, AgentKind,
    AgentSession,
};
use crate::tui::context::agy_parser::update_agy_ctx;
use crate::tui::context::analyzer::{
    apportion_tokens, is_native_1m_model_id, resolve_model_window_hint,
};
use crate::tui::context::bar::compute_cache_hit_pct;
use crate::tui::context::codex_parser::update_codex_ctx;
use crate::tui::context::perf::{PerfAccumMap, RawPerfSample};
use crate::tui::context::{ContextSnapshot, ModelCostEntry, SessionContextAnalyzer};

#[derive(Clone, Debug, Default)]
pub struct AgentCtxSample {
    pub model: String,
    pub ctx_window: u64,
    pub occupancy: u64,
    pub last_input: u64,
    pub last_cached: u64,
    pub last_output: u64,
    pub last_reasoning: u64,
    pub cum_input: u64,
    pub cum_cached: u64,
    pub cum_cache_write: u64,
    pub cum_output: u64,
    pub turns: usize,
    pub weights: [u64; 5],
    pub perf: Vec<RawPerfSample>,
    pub per_model: Vec<(String, u64, u64, u64)>,
}

#[derive(Default)]
pub struct AgentCtxTracker {
    pub offset: u64,
    pub partial: String,
    pub s: AgentCtxSample,
    pub prev_codex_totals: (u64, u64, u64),
}

/// Reads newly appended complete JSONL lines from `path` in `O(N)` up to the last `\n` byte,
/// preserving multi-byte UTF-8 boundaries and resetting `tr` if the file shrank.
pub(crate) fn read_new_jsonl_lines(path: &Path, tr: &mut AgentCtxTracker) -> Vec<String> {
    let Ok(meta) = fs::metadata(path) else {
        return Vec::new();
    };
    let len = meta.len();
    if len < tr.offset {
        *tr = AgentCtxTracker::default();
    }
    if len == tr.offset {
        return Vec::new();
    }
    let mut raw = Vec::new();
    if File::open(path)
        .and_then(|mut f| {
            f.seek(SeekFrom::Start(tr.offset))
                .and_then(|_| f.read_to_end(&mut raw))
        })
        .is_err()
    {
        return Vec::new();
    }
    let Some(last_nl) = raw.iter().rposition(|&b| b == b'\n') else {
        return Vec::new();
    };
    tr.offset += (last_nl + 1) as u64;
    tr.partial.clear();
    String::from_utf8_lossy(&raw[..=last_nl])
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn resolve_agent_window(kind: AgentKind, sample_win: u64, model: &str) -> u64 {
    match kind {
        AgentKind::Codex if sample_win > 0 => sample_win,
        AgentKind::Codex => codex_config()
            .context_window
            .filter(|&w| w > 0)
            .unwrap_or_else(|| resolve_model_window_hint("", model, false)),
        AgentKind::Agy => {
            if let Some(env_w) = env::var("CCIDE_AGY_CONTEXT_WINDOW")
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .filter(|&w| w >= 16_000)
            {
                return env_w;
            }
            if sample_win > 0 {
                return sample_win;
            }
            match classify_model_family_for(AgentKind::Agy, model) {
                "gemini" => 1_048_576,
                "claude" if is_native_1m_model_id(model) => 1_000_000,
                "openai" => 400_000,
                _ => 200_000,
            }
        }
        AgentKind::Claude => resolve_model_window_hint("", model, false),
    }
}

impl SessionContextAnalyzer {
    pub fn sample_agent(
        &mut self,
        home: &str,
        _cwd: &str,
        sess: &AgentSession,
        view_id: &str,
    ) -> ContextSnapshot {
        self.pricing.reload_if_changed(home);
        let is_sub = !view_id.is_empty() && view_id != "main";
        let tr = self
            .agent_trackers
            .entry(sess.transcript_path.clone())
            .or_default();
        match sess.kind {
            AgentKind::Agy => update_agy_ctx(&sess.transcript_path, tr),
            AgentKind::Codex => update_codex_ctx(&sess.transcript_path, tr),
            AgentKind::Claude => {}
        }

        let eff_model = if is_valid_model_name(&tr.s.model)
            && !tr.s.model.eq_ignore_ascii_case("auto")
        {
            tr.s.model.clone()
        } else if is_valid_model_name(&sess.model_id) && !sess.model_id.eq_ignore_ascii_case("auto")
        {
            sess.model_id.clone()
        } else {
            "auto".to_string()
        };

        let model_win_tokens = resolve_agent_window(sess.kind, tr.s.ctx_window, &eff_model);
        let total_in = tr.s.occupancy;
        let apportioned = apportion_tokens(total_in, &tr.s.weights);
        let budget = compute_context_budget(total_in, model_win_tokens, &CompactConfig::default());

        let mut perf_map: PerfAccumMap = HashMap::new();
        for ps in &tr.s.perf {
            if ps.out_tok == 0 && ps.ttft_ms == 0 {
                continue;
            }
            let m_raw = if is_valid_model_name(&ps.model) && !ps.model.eq_ignore_ascii_case("auto")
            {
                &ps.model
            } else {
                &eff_model
            };
            perf_map
                .entry(short_model_label(m_raw))
                .or_default()
                .record_exact(ps);
        }

        let mut model_map: HashMap<String, ModelCostEntry> = HashMap::new();
        let (mut cost_usd, mut uncached_usd) = (0.0f64, 0.0f64);
        for (m_raw, uncached_in, cached_in, out_tok) in &tr.s.per_model {
            let m_use = if is_valid_model_name(m_raw) && !m_raw.eq_ignore_ascii_case("auto") {
                m_raw.as_str()
            } else {
                eff_model.as_str()
            };
            let short = short_model_label(m_use);
            let (act, unc) = self.pricing.resolve(m_use).compute_turn_cost(
                *uncached_in,
                0,
                0,
                *cached_in,
                *out_tok,
            );
            cost_usd += act;
            uncached_usd += unc;
            let entry = model_map.entry(short.clone()).or_insert(ModelCostEntry {
                model_short: short,
                family: classify_model_family_for(sess.kind, m_use),
                turns: 0,
                input_tokens: 0,
                cache_read_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
                saved_usd: 0.0,
                share_pct: 0,
                tps: None,
                ttft_ms: None,
            });
            entry.turns += 1;
            entry.input_tokens = entry.input_tokens.saturating_add(*uncached_in);
            entry.cache_read_tokens = entry.cache_read_tokens.saturating_add(*cached_in);
            entry.output_tokens = entry.output_tokens.saturating_add(*out_tok);
            entry.cost_usd += act;
            entry.saved_usd += (unc - act).max(0.0);
        }

        let mut model_costs: Vec<ModelCostEntry> = model_map.into_values().collect();
        model_costs.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.total_tokens().cmp(&a.total_tokens()))
                .then_with(|| a.model_short.cmp(&b.model_short))
        });
        let tok_sum_all: u64 = model_costs.iter().map(|m| m.total_tokens()).sum();
        for m in &mut model_costs {
            if let Some(pa) = perf_map.get(&m.model_short) {
                (m.tps, m.ttft_ms) = pa.resolve();
            }
            let r = if cost_usd > 1e-7 {
                m.cost_usd / cost_usd
            } else if tok_sum_all > 0 {
                (m.total_tokens() as f64) / (tok_sum_all as f64)
            } else {
                0.0
            };
            m.share_pct = (r * 100.0).round().clamp(0.0, 100.0) as u64;
        }
        let saved_usd = (uncached_usd - cost_usd).max(0.0);
        let saved_pct = if uncached_usd > 0.0001 {
            ((saved_usd / uncached_usd) * 100.0)
                .round()
                .clamp(0.0, 99.0) as u64
        } else {
            0
        };

        ContextSnapshot {
            is_subagent: is_sub,
            model_win_tokens,
            budget,
            total_in_tokens: total_in,
            sys_md_tokens: apportioned[0],
            tools_tokens: apportioned[1],
            tool_io_tokens: apportioned[2],
            thinking_tokens: apportioned[3],
            chat_tokens: apportioned[4],
            cum_cache_read_tokens: tr.s.cum_cached,
            session_hit_pct: compute_cache_hit_pct(tr.s.cum_cached, tr.s.cum_input),
            last_hit_pct: compute_cache_hit_pct(tr.s.last_cached, total_in),
            turns: tr.s.turns,
            session_turns: tr.s.turns,
            cost_usd,
            saved_usd,
            saved_pct,
            burn_usd_per_hr: None,
            rl_5h_pct: None,
            rl_7d_pct: None,
            lines_added: 0,
            lines_removed: 0,
            model_costs,
        }
    }
}
