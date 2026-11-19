//! Multi-Agent (`agy` / `codex`) Context & Cost Snapshot Builder (`src/tui/context/agent_ctx.rs`).

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

use crate::budget::{compute_context_budget, CompactConfig};
use crate::session::{classify_model_family_for, short_model_label, AgentKind, AgentSession};
use crate::tui::context::agy_parser::update_agy_ctx;
use crate::tui::context::agy_swarm_ctx::{pick_eff_model, resolve_agent_window};
use crate::tui::context::analyzer::apportion_tokens;
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

impl AgentCtxSample {
    /// Aggregates `(uncached_in, cached_in, out_tok)` in-place so `per_model` holds at most one entry per model.
    pub fn aggregate_per_model(
        &mut self,
        model: &str,
        uncached_in: u64,
        cached_in: u64,
        out_tok: u64,
    ) {
        if let Some(entry) = self.per_model.iter_mut().find(|e| e.0 == model) {
            entry.1 = entry.1.saturating_add(uncached_in);
            entry.2 = entry.2.saturating_add(cached_in);
            entry.3 = entry.3.saturating_add(out_tok);
        } else {
            self.per_model
                .push((model.to_string(), uncached_in, cached_in, out_tok));
        }
    }
}

#[derive(Default)]
pub struct AgentCtxTracker {
    pub offset: u64,
    pub partial: String,
    pub s: AgentCtxSample,
    pub prev_codex_totals: (u64, u64, u64),
    pub last_seen: Option<Instant>,
}

/// Reads newly appended complete JSONL lines from `path` in `O(N)` up to the last `\n` byte.
pub(crate) fn read_new_jsonl_lines(path: &Path, tr: &mut AgentCtxTracker) -> Vec<String> {
    tr.last_seen = Some(Instant::now());
    let Ok(meta) = fs::metadata(path) else {
        return Vec::new();
    };
    let len = meta.len();
    if len < tr.offset {
        *tr = AgentCtxTracker {
            last_seen: Some(Instant::now()),
            ..AgentCtxTracker::default()
        };
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

impl SessionContextAnalyzer {
    pub(crate) fn snapshot_from(
        &mut self,
        primary: &AgentCtxSample,
        kind: AgentKind,
        primary_model: &str,
        is_sub: bool,
        cost_sources: &[(AgentCtxSample, String)],
    ) -> ContextSnapshot {
        let model_win_tokens = resolve_agent_window(kind, primary.ctx_window, primary_model);
        let total_in = primary.occupancy;
        let apportioned = apportion_tokens(total_in, &primary.weights);
        let budget = compute_context_budget(total_in, model_win_tokens, &CompactConfig::default());

        let mut perf_map: PerfAccumMap = HashMap::new();
        let mut model_map: HashMap<String, ModelCostEntry> = HashMap::new();
        let (mut cost_usd, mut uncached_usd, mut session_turns) = (0.0f64, 0.0f64, 0usize);

        for (src, fb_model) in cost_sources {
            session_turns = session_turns.saturating_add(src.turns);
            for ps in &src.perf {
                if ps.out_tok == 0 && ps.ttft_ms == 0 {
                    continue;
                }
                let m_raw = pick_eff_model(&ps.model, fb_model);
                perf_map
                    .entry(short_model_label(&m_raw))
                    .or_default()
                    .record_exact(ps);
            }
            let add_turns = if src.per_model.len() <= 1 {
                src.turns.max(1)
            } else {
                1
            };
            for (m_raw, uncached_in, cached_in, out_tok) in &src.per_model {
                let m_use = pick_eff_model(m_raw, fb_model);
                let short = short_model_label(&m_use);
                let (act, unc) = self.pricing.resolve(&m_use).compute_turn_cost(
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
                    family: classify_model_family_for(kind, &m_use),
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
                entry.turns = entry.turns.saturating_add(add_turns);
                entry.input_tokens = entry.input_tokens.saturating_add(*uncached_in);
                entry.cache_read_tokens = entry.cache_read_tokens.saturating_add(*cached_in);
                entry.output_tokens = entry.output_tokens.saturating_add(*out_tok);
                entry.cost_usd += act;
                entry.saved_usd += (unc - act).max(0.0);
            }
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
            cum_cache_read_tokens: primary.cum_cached,
            session_hit_pct: compute_cache_hit_pct(primary.cum_cached, primary.cum_input),
            last_hit_pct: compute_cache_hit_pct(primary.last_cached, total_in),
            turns: primary.turns,
            session_turns: session_turns.max(primary.turns),
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

    pub fn sample_agent(
        &mut self,
        home: &str,
        _cwd: &str,
        sess: &AgentSession,
        view_id: &str,
    ) -> ContextSnapshot {
        self.pricing.reload_if_changed(home);
        let is_sub = !view_id.is_empty() && view_id != "main";
        let (primary, eff_model) = {
            let tr = self
                .agent_trackers
                .entry(sess.transcript_path.clone())
                .or_default();
            match sess.kind {
                AgentKind::Agy => update_agy_ctx(&sess.transcript_path, tr),
                AgentKind::Codex => update_codex_ctx(&sess.transcript_path, tr),
                AgentKind::Claude => {}
            }
            (tr.s.clone(), pick_eff_model(&tr.s.model, &sess.model_id))
        };
        let sources = [(primary.clone(), eff_model.clone())];
        self.snapshot_from(&primary, sess.kind, &eff_model, is_sub, &sources)
    }
}
