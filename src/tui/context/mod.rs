//! Live Per-Session `CONTEXT` Window & Per-Model `COST` Breakdown Analyzer (`src/tui/context/`).

pub mod analyzer;
pub mod bar;
pub mod cost_panel;
pub mod json_scan;
pub mod parser;
pub mod perf;
pub mod render;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::commands::statusline::{
    compute_context_budget, load_compact_config, CompactConfig, ContextBudget,
};
use crate::json::{extract_json_f64, extract_json_str, extract_json_u64};
use crate::platform::{cache_dir, user_home_dir};
use crate::tui::pricing::PricingCatalog;
use analyzer::{aggregate_session_model_costs, list_subagent_files, resolve_transcript_file};
pub use analyzer::{apportion_tokens, resolve_model_window_hint};
use bar::compute_cache_hit_pct;
pub use cost_panel::build_cost_panel;
use parser::{update_transcript_ctx_tracker, TranscriptCtxTracker};
use perf::PerfStreamTracker;
pub use render::build_context_panel;

pub const TAG_CTX_TOGGLE: u8 = 12;

#[derive(Clone, Debug)]
pub struct ModelCostEntry {
    pub model_short: String,
    pub family: &'static str,
    pub turns: usize,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub saved_usd: f64,
    pub share_pct: u64,
    pub tps: Option<f64>,
    pub ttft_ms: Option<u64>,
}

impl ModelCostEntry {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.output_tokens)
    }
}

#[derive(Clone, Debug)]
pub struct ContextSnapshot {
    pub is_subagent: bool,
    pub model_win_tokens: u64,
    pub budget: ContextBudget,
    pub total_in_tokens: u64,
    pub sys_md_tokens: u64,
    pub tools_tokens: u64,
    pub tool_io_tokens: u64,
    pub thinking_tokens: u64,
    pub chat_tokens: u64,
    pub cum_cache_read_tokens: u64,
    pub session_hit_pct: f64,
    pub last_hit_pct: f64,
    pub turns: usize,
    pub session_turns: usize,
    pub cost_usd: f64,
    pub saved_usd: f64,
    pub saved_pct: u64,
    pub burn_usd_per_hr: Option<f64>,
    pub rl_5h_pct: Option<u64>,
    pub rl_7d_pct: Option<u64>,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub model_costs: Vec<ModelCostEntry>,
}

pub struct SessionContextAnalyzer {
    trackers: HashMap<PathBuf, TranscriptCtxTracker>,
    path_cache: HashMap<String, PathBuf>,
    sub_dir_cache: HashMap<String, (Option<SystemTime>, Vec<PathBuf>)>,
    perf_tracker: PerfStreamTracker,
    last_cfg_refresh: Option<SystemTime>,
    cached_cfg: CompactConfig,
    cached_cfg_key: String,
    cached_settings_txt: String,
    pricing: PricingCatalog,
}

impl SessionContextAnalyzer {
    pub fn new() -> Self {
        let home = user_home_dir();
        Self {
            trackers: HashMap::new(),
            path_cache: HashMap::new(),
            sub_dir_cache: HashMap::new(),
            perf_tracker: PerfStreamTracker::default(),
            last_cfg_refresh: None,
            cached_cfg: CompactConfig::default(),
            cached_cfg_key: String::new(),
            cached_settings_txt: String::new(),
            pricing: PricingCatalog::new(&home),
        }
    }

    pub fn clear(&mut self) {
        self.trackers.clear();
        self.path_cache.clear();
        self.sub_dir_cache.clear();
        self.perf_tracker.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        &mut self,
        home: &str,
        cwd: &str,
        session_id: &str,
        claude_pid: u32,
        active_view_id: &str,
        main_model_label: &str,
        sub_model_label: &str,
    ) -> ContextSnapshot {
        let is_sub = !active_view_id.is_empty() && active_view_id != "main";
        let active_label = if is_sub {
            sub_model_label
        } else {
            main_model_label
        };
        let st_path = Path::new(home).join(".claude/settings.json");
        let mtime = fs::metadata(&st_path).and_then(|m| m.modified()).ok();
        let key = format!("{}|{}", cwd, active_label);
        if self.cached_cfg_key != key || mtime != self.last_cfg_refresh {
            self.cached_settings_txt = fs::read_to_string(&st_path).unwrap_or_default();
            self.cached_cfg = load_compact_config(home, cwd, cwd, active_label, active_label);
            self.cached_cfg_key = key;
            self.last_cfg_refresh = mtime;
        }
        let mut model_win_tokens =
            resolve_model_window_hint(&self.cached_settings_txt, active_label, is_sub);

        let (mut live_tokens_k, mut lines_added, mut lines_removed) = (0u64, 0u64, 0u64);
        let (mut rl_5h_pct, mut rl_7d_pct) = (None, None);
        let mut cc_reported_cost = 0.0f64;

        if !session_id.is_empty() {
            if let Ok(stxt) =
                fs::read_to_string(cache_dir(home).join(format!("live_sess_{}.json", session_id)))
            {
                rl_5h_pct = extract_json_u64(&stxt, "rl_5h_pct");
                rl_7d_pct = extract_json_u64(&stxt, "rl_7d_pct");
                lines_added = extract_json_u64(&stxt, "lines_add").unwrap_or(0);
                lines_removed = extract_json_u64(&stxt, "lines_del").unwrap_or(0);
                cc_reported_cost = extract_json_f64(&stxt, "cc_cost_usd").unwrap_or(0.0);
                if !is_sub {
                    live_tokens_k = extract_json_u64(&stxt, "tokens_k").unwrap_or(0);
                    if let Some(wt) =
                        extract_json_u64(&stxt, "win_tokens").filter(|&w| w >= 100_000)
                    {
                        model_win_tokens = wt;
                    } else if extract_json_str(&stxt, "model")
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains("[1m]")
                    {
                        model_win_tokens = 1_000_000;
                    }
                }
            }
        }

        self.pricing.reload_if_changed(home);
        let eff_cfg = if is_sub {
            CompactConfig::default()
        } else {
            self.cached_cfg.clone()
        };
        let (mut sys_b, mut tools_b, mut tio_b, mut thk_b, mut chat_b) =
            (0u64, 0u64, 0u64, 0u64, 0u64);
        let (mut in_tok, mut cc_tok, mut cr_tok, mut turns) = (0u64, 0u64, 0u64, 0usize);
        let (mut cum_in, mut cum_cc, mut cum_cr) = (0u64, 0u64, 0u64);

        if let Some(active_jpath) =
            resolve_transcript_file(&mut self.path_cache, home, session_id, active_view_id)
        {
            let tr = self
                .trackers
                .entry(active_jpath.clone())
                .or_insert_with(TranscriptCtxTracker::new);
            update_transcript_ctx_tracker(&active_jpath, is_sub, tr);
            (in_tok, cc_tok, cr_tok, turns) =
                (tr.in_tok, tr.cc_tok, tr.cr_tok, tr.turns_by_id.len());
            for rec in tr.turns_by_id.values() {
                cum_in = cum_in.saturating_add(rec.in_tok);
                cum_cc = cum_cc.saturating_add(rec.cc_tok);
                cum_cr = cum_cr.saturating_add(rec.cr_tok);
            }
            if tr.has_api_requests && !tr.live_hashes.is_empty() {
                (sys_b, tools_b) = tr.active_shape;
                for h in &tr.live_hashes {
                    if let Some(bw) = tr.blobs.get(h) {
                        sys_b = sys_b.saturating_add(bw.sys_md);
                        tio_b = tio_b.saturating_add(bw.tool_io);
                        thk_b = thk_b.saturating_add(bw.thinking);
                        chat_b = chat_b.saturating_add(bw.chat);
                    }
                }
            } else {
                sys_b = tr.active_shape.0.saturating_add(tr.fb_weights.sys_md);
                tools_b = tr.active_shape.1;
                (tio_b, thk_b, chat_b) = (
                    tr.fb_weights.tool_io,
                    tr.fb_weights.thinking,
                    tr.fb_weights.chat,
                );
            }
        }

        let mut all_paths: Vec<(PathBuf, bool)> = Vec::new();
        if let Some(main_jp) =
            resolve_transcript_file(&mut self.path_cache, home, session_id, "main")
        {
            let subs = list_subagent_files(&mut self.sub_dir_cache, &main_jp, session_id);
            all_paths.push((main_jp, false));
            all_paths.extend(subs.into_iter().map(|sp| (sp, true)));
        }

        self.perf_tracker.update(claude_pid);
        let agg = aggregate_session_model_costs(
            all_paths,
            &mut self.trackers,
            &self.perf_tracker.samples,
            &self.pricing,
            &self.cached_settings_txt,
            main_model_label,
            sub_model_label,
            cc_reported_cost,
        );

        let mut total_in = in_tok.saturating_add(cc_tok).saturating_add(cr_tok);
        if !is_sub && live_tokens_k > 0 && (total_in == 0 || live_tokens_k > (total_in / 1000) + 8)
        {
            total_in = live_tokens_k * 1_000;
        }
        let cum_prompt = cum_in.saturating_add(cum_cc).saturating_add(cum_cr);
        let session_hit_pct = compute_cache_hit_pct(cum_cr, cum_prompt);
        let last_prompt = in_tok.saturating_add(cc_tok).saturating_add(cr_tok);
        let last_hit_pct = compute_cache_hit_pct(cr_tok, last_prompt);
        let saved_usd = (agg.uncached_usd - agg.cost_usd).max(0.0);
        let saved_pct = if agg.uncached_usd > 0.0001 {
            ((saved_usd / agg.uncached_usd) * 100.0)
                .round()
                .clamp(0.0, 99.0) as u64
        } else {
            0
        };
        let apportioned = apportion_tokens(total_in, &[sys_b, tools_b, tio_b, thk_b, chat_b]);
        let budget = compute_context_budget(total_in, model_win_tokens, &eff_cfg);

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
            cum_cache_read_tokens: cum_cr,
            session_hit_pct,
            last_hit_pct,
            turns,
            session_turns: agg.session_turns,
            cost_usd: agg.cost_usd,
            saved_usd,
            saved_pct,
            burn_usd_per_hr: agg.burn_usd_per_hr,
            rl_5h_pct,
            rl_7d_pct,
            lines_added,
            lines_removed,
            model_costs: agg.model_costs,
        }
    }
}
