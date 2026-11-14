//! Token Window Sizing, Largest-Remainder Apportionment & Cost Aggregation Helpers.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::json::extract_json_str;
use crate::session::{
    classify_model_family, is_valid_model_name, map_alias_with_settings, short_model_label,
};
use crate::tui::context::parser::{update_transcript_ctx_tracker, TranscriptCtxTracker};
use crate::tui::context::perf::{PerfAccumMap, RawPerfSample};
use crate::tui::context::ModelCostEntry;
use crate::tui::pricing::PricingCatalog;

pub fn is_native_1m_model_id(raw: &str) -> bool {
    let low = raw.to_ascii_lowercase();
    low.contains("[1m]")
        || low.contains("fable-5")
        || low.contains("mythos-5")
        || low.contains("opus-5")
}

pub fn resolve_transcript_file(
    path_cache: &mut HashMap<String, PathBuf>,
    home: &str,
    sid: &str,
    view_id: &str,
) -> Option<PathBuf> {
    if sid.is_empty() {
        return None;
    }
    let cache_key = format!("{}:{}", sid, view_id);
    if let Some(p) = path_cache.get(&cache_key).filter(|p| p.is_file()) {
        return Some(p.clone());
    }
    let rd = fs::read_dir(Path::new(home).join(".claude/projects")).ok()?;
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
            path_cache.insert(cache_key, cand.clone());
            return Some(cand);
        }
    }
    None
}

pub fn list_subagent_files(
    sub_dir_cache: &mut HashMap<String, (Option<SystemTime>, Vec<PathBuf>)>,
    main_jpath: &Path,
    sid: &str,
) -> Vec<PathBuf> {
    let Some(proj_dir) = main_jpath.parent() else {
        return Vec::new();
    };
    let sub_dir = proj_dir.join(sid).join("subagents");
    let cur_mt = fs::metadata(&sub_dir).and_then(|m| m.modified()).ok();
    if let Some((prev_mt, cached)) = sub_dir_cache.get(sid) {
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
    sub_dir_cache.insert(sid.to_string(), (cur_mt, files.clone()));
    files
}

pub fn resolve_model_window_hint(settings_txt: &str, model_label: &str, is_sub: bool) -> u64 {
    if is_native_1m_model_id(model_label) {
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
        if is_native_1m_model_id(&check.unwrap_or_default()) {
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

pub struct AggregatedCosts {
    pub cost_usd: f64,
    pub uncached_usd: f64,
    pub session_turns: usize,
    pub burn_usd_per_hr: Option<f64>,
    pub model_costs: Vec<ModelCostEntry>,
}

#[allow(clippy::too_many_arguments)]
pub fn aggregate_session_model_costs(
    all_paths: Vec<(PathBuf, bool)>,
    trackers: &mut HashMap<PathBuf, TranscriptCtxTracker>,
    perf_samples: &[RawPerfSample],
    pricing: &PricingCatalog,
    settings_txt: &str,
    main_model_label: &str,
    sub_model_label: &str,
    cc_reported_cost: f64,
) -> AggregatedCosts {
    let mut model_map: HashMap<String, ModelCostEntry> = HashMap::new();
    let mut perf_map: PerfAccumMap = HashMap::new();
    let (mut cost_usd, mut uncached_usd) = (0.0f64, 0.0f64);
    let mut session_turns = 0usize;
    let (mut min_ts, mut max_ts): (Option<u64>, Option<u64>) = (None, None);

    for s in perf_samples {
        let mapped = map_alias_with_settings(&s.model, settings_txt);
        let short = short_model_label(&mapped);
        perf_map.entry(short).or_default().record_exact(s);
    }

    for (jp, file_is_sub) in all_paths {
        let tr = trackers.entry(jp.clone()).or_default();
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
            let raw_m = if is_valid_model_name(&rec.model) {
                &rec.model
            } else {
                fb_model
            };
            let mapped = map_alias_with_settings(raw_m, settings_txt);
            let short = short_model_label(&mapped);
            let fam = classify_model_family(&mapped);
            let (act, unc) = pricing.resolve(&mapped).compute_turn_cost_full(
                rec.in_tok,
                rec.cc_tok,
                rec.cc_1h_tok,
                rec.cr_tok,
                rec.out_tok,
                rec.fast_mode,
                rec.us_geo,
                rec.web_search_reqs,
            );
            cost_usd += act;
            uncached_usd += unc;
            perf_map.entry(short.clone()).or_default().record_fallback(
                rec.out_tok,
                rec.first_ms,
                rec.dur_ms,
                rec.first_bytes,
                rec.total_bytes,
            );
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
                tps: None,
                ttft_ms: None,
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

    if cost_usd == 0.0 && cc_reported_cost > 0.0 {
        cost_usd = cc_reported_cost;
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
            let (tps, ttft) = pa.resolve();
            m.tps = tps;
            m.ttft_ms = ttft;
        }
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

    AggregatedCosts {
        cost_usd,
        uncached_usd,
        session_turns,
        burn_usd_per_hr,
        model_costs,
    }
}
