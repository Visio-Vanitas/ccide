//! Live Per-Session `CONTEXT` Window & Per-Model `COST` Breakdown Analyzer (`src/tui/context/`).

pub mod bar;
pub mod cost_panel;
pub mod parser;
pub mod render;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::commands::statusline::{
    compute_context_budget, load_compact_config, CompactConfig, ContextBudget,
};
use crate::json::{extract_json_str, extract_json_u64};
use crate::session::{classify_model_family, map_alias_with_settings, short_model_label};
use crate::tui::pricing::PricingCatalog;
pub use cost_panel::build_cost_panel;
use parser::{update_transcript_ctx_tracker, TranscriptCtxTracker};
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
    pub session_hit_pct: u64,
    pub last_hit_pct: u64,
    pub turns: usize,
    pub session_turns: usize,
    pub cost_usd: f64,
    pub saved_usd: f64,
    pub saved_pct: u64,
    pub burn_usd_per_hr: Option<f64>,
    pub model_costs: Vec<ModelCostEntry>,
}

pub struct SessionContextAnalyzer {
    trackers: HashMap<PathBuf, TranscriptCtxTracker>,
    path_cache: HashMap<String, PathBuf>,
    sub_dir_cache: HashMap<String, (Option<SystemTime>, Vec<PathBuf>)>,
    last_cfg_refresh: Option<SystemTime>,
    cached_cfg: CompactConfig,
    cached_cfg_key: String,
    cached_settings_txt: String,
    pricing: PricingCatalog,
}

impl SessionContextAnalyzer {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            trackers: HashMap::new(),
            path_cache: HashMap::new(),
            sub_dir_cache: HashMap::new(),
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
    }

    fn resolve_transcript_path(&mut self, home: &str, sid: &str, view_id: &str) -> Option<PathBuf> {
        if sid.is_empty() {
            return None;
        }
        let cache_key = format!("{}:{}", sid, view_id);
        if let Some(p) = self.path_cache.get(&cache_key) {
            if p.is_file() {
                return Some(p.clone());
            }
        }
        let rd = fs::read_dir(format!("{}/.claude/projects", home)).ok()?;
        let is_sub = !view_id.is_empty() && view_id != "main";
        for entry in rd.flatten() {
            let base = entry.path();
            let cand = if is_sub {
                base.join(sid)
                    .join("subagents")
                    .join(format!("agent-{}.jsonl", view_id))
            } else {
                base.join(format!("{}.jsonl", sid))
            };
            if cand.is_file() {
                self.path_cache.insert(cache_key, cand.clone());
                return Some(cand);
            }
        }
        None
    }

    fn list_subagent_transcripts(&mut self, main_jpath: &Path, sid: &str) -> Vec<PathBuf> {
        let Some(proj_dir) = main_jpath.parent() else {
            return Vec::new();
        };
        let sub_dir = proj_dir.join(sid).join("subagents");
        let cur_mt = fs::metadata(&sub_dir).and_then(|m| m.modified()).ok();
        if let Some((prev_mt, cached)) = self.sub_dir_cache.get(sid) {
            if *prev_mt == cur_mt {
                return cached.clone();
            }
        }
        let mut files = Vec::new();
        if let Ok(rd) = fs::read_dir(&sub_dir) {
            for e in rd.flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("agent-") && name.ends_with(".jsonl") && p.is_file() {
                    files.push(p);
                }
            }
        }
        files.sort();
        self.sub_dir_cache
            .insert(sid.to_string(), (cur_mt, files.clone()));
        files
    }

    fn refresh_compact_cfg(&mut self, home: &str, cwd: &str, model_label: &str) {
        let st_path = Path::new(home).join(".claude/settings.json");
        let mtime = fs::metadata(&st_path).and_then(|m| m.modified()).ok();
        let key = format!("{}|{}", cwd, model_label);
        if self.cached_cfg_key != key || mtime != self.last_cfg_refresh {
            self.cached_settings_txt = fs::read_to_string(&st_path).unwrap_or_default();
            self.cached_cfg = load_compact_config(home, cwd, cwd, model_label, model_label);
            self.cached_cfg_key = key;
            self.last_cfg_refresh = mtime;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        &mut self,
        home: &str,
        cwd: &str,
        session_id: &str,
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
        let mut model_win_tokens =
            resolve_model_window_hint(&self.cached_settings_txt, active_label, is_sub);

        let mut live_tokens_k = 0u64;
        if !is_sub && !session_id.is_empty() {
            let spath = format!("{}/.cache/cc-ide/live_sess_{}.json", home, session_id);
            if let Ok(stxt) = fs::read_to_string(&spath) {
                live_tokens_k = extract_json_u64(&stxt, "tokens_k").unwrap_or(0);
                if extract_json_str(&stxt, "model")
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains("[1m]")
                {
                    model_win_tokens = 1_000_000;
                }
            }
        }

        self.refresh_compact_cfg(home, cwd, active_label);
        if model_win_tokens == 200_000 {
            model_win_tokens =
                resolve_model_window_hint(&self.cached_settings_txt, active_label, is_sub);
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

        if let Some(active_jpath) = self.resolve_transcript_path(home, session_id, active_view_id) {
            let tr = self
                .trackers
                .entry(active_jpath.clone())
                .or_insert_with(TranscriptCtxTracker::new);
            update_transcript_ctx_tracker(&active_jpath, is_sub, tr);
            in_tok = tr.in_tok;
            cc_tok = tr.cc_tok;
            cr_tok = tr.cr_tok;
            turns = tr.turns_by_id.len();
            for rec in tr.turns_by_id.values() {
                cum_in = cum_in.saturating_add(rec.in_tok);
                cum_cc = cum_cc.saturating_add(rec.cc_tok);
                cum_cr = cum_cr.saturating_add(rec.cr_tok);
            }
            if tr.has_api_requests && !tr.live_hashes.is_empty() {
                sys_b = tr.active_shape.0;
                tools_b = tr.active_shape.1;
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
                tio_b = tr.fb_weights.tool_io;
                thk_b = tr.fb_weights.thinking;
                chat_b = tr.fb_weights.chat;
            }
        }

        // Aggregate session-wide per-model consumption across Main + All Subagents
        let mut all_paths: Vec<(PathBuf, bool)> = Vec::new();
        if let Some(main_jp) = self.resolve_transcript_path(home, session_id, "main") {
            let subs = self.list_subagent_transcripts(&main_jp, session_id);
            all_paths.push((main_jp, false));
            for sp in subs {
                all_paths.push((sp, true));
            }
        }

        let mut model_map: HashMap<String, ModelCostEntry> = HashMap::new();
        let (mut cost_usd, mut uncached_usd) = (0.0f64, 0.0f64);
        let mut session_turns = 0usize;
        let (mut min_ts, mut max_ts): (Option<u64>, Option<u64>) = (None, None);

        for (jp, file_is_sub) in all_paths {
            let tr = self
                .trackers
                .entry(jp.clone())
                .or_insert_with(TranscriptCtxTracker::new);
            update_transcript_ctx_tracker(&jp, file_is_sub, tr);
            if let Some(t0) = tr.first_ts_s {
                min_ts = Some(min_ts.map_or(t0, |cur| cur.min(t0)));
            }
            if let Some(t1) = tr.last_ts_s {
                max_ts = Some(max_ts.map_or(t1, |cur| cur.max(t1)));
            }
            let fb_model = if file_is_sub {
                sub_model_label
            } else {
                main_model_label
            };
            for rec in tr.turns_by_id.values() {
                session_turns += 1;
                let raw_m = if rec.model.is_empty() || rec.model.starts_with('<') {
                    fb_model
                } else {
                    &rec.model
                };
                let mapped = map_alias_with_settings(raw_m, &self.cached_settings_txt);
                let short = short_model_label(&mapped);
                let fam = classify_model_family(&mapped);
                let (act, unc) = self.pricing.resolve(&mapped).compute_turn_cost(
                    rec.in_tok,
                    rec.cc_tok,
                    rec.cc_1h_tok,
                    rec.cr_tok,
                    rec.out_tok,
                );
                cost_usd += act;
                uncached_usd += unc;
                let entry = model_map.entry(short.clone()).or_insert(ModelCostEntry {
                    model_short: short,
                    family: fam,
                    turns: 0,
                    input_tokens: 0,
                    cache_read_tokens: 0,
                    output_tokens: 0,
                    cost_usd: 0.0,
                    saved_usd: 0.0,
                    share_pct: 0,
                });
                entry.turns += 1;
                entry.input_tokens = entry
                    .input_tokens
                    .saturating_add(rec.in_tok.saturating_add(rec.cc_tok));
                entry.cache_read_tokens = entry.cache_read_tokens.saturating_add(rec.cr_tok);
                entry.output_tokens = entry.output_tokens.saturating_add(rec.out_tok);
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
            m.share_pct = if cost_usd > 1e-7 {
                ((m.cost_usd / cost_usd) * 100.0).round().clamp(0.0, 100.0) as u64
            } else if tok_sum_all > 0 {
                ((m.total_tokens() as f64 / tok_sum_all as f64) * 100.0)
                    .round()
                    .clamp(0.0, 100.0) as u64
            } else {
                0
            };
        }

        let burn_usd_per_hr = match (min_ts, max_ts) {
            (Some(t0), Some(t1))
                if t1.saturating_sub(t0) >= 90 && session_turns >= 2 && cost_usd > 0.0 =>
            {
                Some(cost_usd * 3600.0 / (t1.saturating_sub(t0) as f64))
            }
            _ => None,
        };

        let mut total_in = in_tok.saturating_add(cc_tok).saturating_add(cr_tok);
        if !is_sub && live_tokens_k > 0 && (total_in == 0 || live_tokens_k > (total_in / 1000) + 8)
        {
            total_in = live_tokens_k * 1_000;
        }
        let cum_prompt = cum_in.saturating_add(cum_cc).saturating_add(cum_cr);
        let session_hit_pct = if cum_prompt > 0 {
            ((cum_cr as f64 / cum_prompt as f64) * 100.0)
                .round()
                .clamp(0.0, 100.0) as u64
        } else {
            0
        };
        let last_prompt = in_tok.saturating_add(cc_tok).saturating_add(cr_tok);
        let last_hit_pct = if last_prompt > 0 {
            ((cr_tok as f64 / last_prompt as f64) * 100.0)
                .round()
                .clamp(0.0, 100.0) as u64
        } else {
            0
        };
        let saved_usd = (uncached_usd - cost_usd).max(0.0);
        let saved_pct = if uncached_usd > 0.0001 {
            ((saved_usd / uncached_usd) * 100.0)
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
            session_turns,
            cost_usd,
            saved_usd,
            saved_pct,
            burn_usd_per_hr,
            model_costs,
        }
    }
}

fn resolve_model_window_hint(settings_txt: &str, model_label: &str, is_sub: bool) -> u64 {
    if model_label.to_ascii_lowercase().contains("[1m]") {
        return 1_000_000;
    }
    if !settings_txt.is_empty() {
        let check = if is_sub {
            extract_json_str(settings_txt, "CLAUDE_CODE_SUBAGENT_MODEL")
        } else {
            let raw_m = extract_json_str(settings_txt, "model").unwrap_or_default();
            let low = raw_m.to_ascii_lowercase();
            if low.contains("fable") {
                extract_json_str(settings_txt, "ANTHROPIC_DEFAULT_FABLE_MODEL")
            } else if low.contains("opus") {
                extract_json_str(settings_txt, "ANTHROPIC_DEFAULT_OPUS_MODEL")
            } else if low.contains("sonnet") {
                extract_json_str(settings_txt, "ANTHROPIC_DEFAULT_SONNET_MODEL")
            } else if low.contains("haiku") {
                extract_json_str(settings_txt, "ANTHROPIC_DEFAULT_HAIKU_MODEL")
            } else {
                Some(raw_m)
            }
        };
        if check
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("[1m]")
        {
            return 1_000_000;
        }
    }
    200_000
}

pub fn apportion_tokens(total: u64, weights: &[u64; 5]) -> [u64; 5] {
    if total == 0 {
        return [0; 5];
    }
    let sum_w: u128 = weights.iter().map(|&w| u128::from(w)).sum();
    if sum_w == 0 {
        return [0, 0, 0, 0, total];
    }
    let tot_128 = u128::from(total);
    let mut out = [0u64; 5];
    let mut rems = [(0usize, 0u128); 5];
    let mut assigned = 0u64;
    for i in 0..5 {
        let num = u128::from(weights[i]) * tot_128;
        let q = (num / sum_w) as u64;
        out[i] = q;
        assigned += q;
        rems[i] = (i, num % sum_w);
    }
    rems.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut rem_tok = total.saturating_sub(assigned);
    for &(idx, _) in &rems {
        if rem_tok == 0 {
            break;
        }
        out[idx] += 1;
        rem_tok -= 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::palette::Palette;
    use parser::{classify_message_bytes, IMAGE_EQUIV_BYTES};

    #[test]
    fn renders_split_context_and_per_model_cost_panels() {
        let out = apportion_tokens(445_016, &[36_947, 125_084, 548_620, 570_432, 55_493]);
        assert_eq!(out.iter().sum::<u64>(), 445_016);

        let w = classify_message_bytes(
            r#"{"role":"user","content":[{"type":"image","data":"AAAA"},{"type":"tool_result","content":"ok"}]}"#,
        );
        assert!(w.chat >= IMAGE_EQUIV_BYTES && w.tool_io > 0);

        let pal = Palette::default_palette();
        let budget = compute_context_budget(445_000, 1_000_000, &CompactConfig::default());
        let snap = ContextSnapshot {
            is_subagent: false,
            model_win_tokens: 1_000_000,
            budget,
            total_in_tokens: 445_000,
            sys_md_tokens: 12_000,
            tools_tokens: 41_000,
            tool_io_tokens: 183_000,
            thinking_tokens: 190_000,
            chat_tokens: 19_000,
            cum_cache_read_tokens: 12_400_000,
            session_hit_pct: 89,
            last_hit_pct: 98,
            turns: 20,
            session_turns: 28,
            cost_usd: 2.24,
            saved_usd: 14.2,
            saved_pct: 86,
            burn_usd_per_hr: Some(3.6),
            model_costs: vec![
                ModelCostEntry {
                    model_short: "claude-fable-5-1".to_string(),
                    family: "claude",
                    turns: 20,
                    input_tokens: 310_000,
                    cache_read_tokens: 12_400_000,
                    output_tokens: 48_000,
                    cost_usd: 1.84,
                    saved_usd: 13.5,
                    share_pct: 82,
                },
                ModelCostEntry {
                    model_short: "deepseek-v4.1".to_string(),
                    family: "deepseek",
                    turns: 8,
                    input_tokens: 120_000,
                    cache_read_tokens: 900_000,
                    output_tokens: 22_000,
                    cost_usd: 0.40,
                    saved_usd: 0.7,
                    share_pct: 18,
                },
            ],
        };
        let ctx_spec = build_context_panel(&snap, 40, 14, false, &pal);
        let (cost_spec, max_sc) = build_cost_panel(&snap, 40, 14, 0, &pal);
        assert_eq!(ctx_spec.title, "CONTEXT");
        assert_eq!(cost_spec.title, "COST");
        assert_eq!(max_sc, 0);
        assert!(ctx_spec.rows.iter().all(|r| !r.text.contains('░')));
        assert!(cost_spec
            .rows
            .iter()
            .any(|r| r.text.contains("claude-fable-5-1")));
        assert!(cost_spec
            .rows
            .iter()
            .any(|r| r.text.contains("deepseek-v4.1")));
    }
}
