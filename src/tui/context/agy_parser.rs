//! Google Antigravity CLI (`agy`) `transcript.jsonl` Context & Cost Parser (`src/tui/context/agy_parser.rs`).

use std::path::Path;

use crate::json::{extract_json_array_slice, extract_json_str, extract_json_u64};
use crate::session::{classify_model_family_for, is_valid_model_name, AgentKind};
use crate::tui::context::agent_ctx::{read_new_jsonl_lines, AgentCtxTracker};
use crate::tui::context::perf::{parse_iso8601_ms, RawPerfSample};

/// Computes context window occupancy for `agy` steps:
/// Anthropic (`classify_model_family_for(AgentKind::Agy, model) == "claude"`) reports
/// `input_tokens` and `cache_read_tokens` exclusively (`input + cache_read`), whereas
/// Gemini/OpenAI (and empty/`"auto"` default Agy family) report `input_tokens` inclusive of cache.
pub fn agy_occupancy(model: &str, input_tokens: u64, cache_read_tokens: u64) -> u64 {
    if classify_model_family_for(AgentKind::Agy, model) == "claude" {
        input_tokens.saturating_add(cache_read_tokens)
    } else {
        input_tokens
    }
}

pub fn update_agy_ctx(path: &Path, tr: &mut AgentCtxTracker) {
    let lines = read_new_jsonl_lines(path, tr);
    if lines.is_empty() {
        return;
    }
    for line in &lines {
        let step_type = extract_json_str(line, "type").unwrap_or_default();
        let ts_ms = extract_json_str(line, "created_at").and_then(|s| parse_iso8601_ms(&s));

        match step_type.as_str() {
            "USER_INPUT" => {
                if let Some(c) = extract_json_str(line, "content") {
                    tr.s.weights[4] = tr.s.weights[4].saturating_add(c.len() as u64);
                }
                if let Some(ms) = ts_ms {
                    if tr
                        .s
                        .perf
                        .last()
                        .is_some_and(|p| p.out_tok == 0 && p.ttft_ms == 0)
                    {
                        tr.s.perf.pop();
                    }
                    tr.s.perf.push(RawPerfSample {
                        model: String::new(),
                        out_tok: 0,
                        dur_ms: ms,
                        ttft_ms: 0,
                        first_content_ms: 0,
                    });
                }
            }
            "PLANNER_RESPONSE" => {
                if let Some(m) = extract_json_str(line, "model")
                    .or_else(|| extract_json_str(line, "modelName"))
                    .filter(|m| is_valid_model_name(m))
                {
                    tr.s.model = m;
                }
                if let Some(th) = extract_json_str(line, "thinking") {
                    tr.s.weights[3] = tr.s.weights[3].saturating_add(th.len() as u64);
                }
                if let Some(c) = extract_json_str(line, "content") {
                    tr.s.weights[4] = tr.s.weights[4].saturating_add(c.len() as u64);
                }
                if let Some(tc) = extract_json_array_slice(line, "tool_calls") {
                    tr.s.weights[2] = tr.s.weights[2].saturating_add(tc.len() as u64);
                }

                let in_t = extract_json_u64(line, "input_tokens").unwrap_or(0);
                let out_t = extract_json_u64(line, "output_tokens").unwrap_or(0);
                let cr_t = extract_json_u64(line, "cache_read_tokens").unwrap_or(0);
                if in_t > 0 || out_t > 0 || cr_t > 0 {
                    let occ = agy_occupancy(&tr.s.model, in_t, cr_t);
                    let uncached_in = occ.saturating_sub(cr_t);
                    tr.s.turns += 1;
                    tr.s.last_input = in_t;
                    tr.s.last_cached = cr_t;
                    tr.s.last_output = out_t;
                    tr.s.occupancy = occ;
                    tr.s.cum_input = tr.s.cum_input.saturating_add(occ);
                    tr.s.cum_cached = tr.s.cum_cached.saturating_add(cr_t);
                    tr.s.cum_output = tr.s.cum_output.saturating_add(out_t);
                    let m_key = tr.s.model.clone();
                    tr.s.aggregate_per_model(&m_key, uncached_in, cr_t, out_t);
                }

                if let Some(ms) = ts_ms {
                    if let Some(last) = tr.s.perf.last_mut() {
                        if last.out_tok == 0 && last.ttft_ms == 0 && last.dur_ms > 0 {
                            let user_ms = last.dur_ms;
                            let ttft = ms.saturating_sub(user_ms).max(1);
                            last.model = tr.s.model.clone();
                            last.out_tok = out_t;
                            last.dur_ms = ttft;
                            last.ttft_ms = ttft;
                            last.first_content_ms = user_ms;
                        } else if last.first_content_ms > 1_000_000_000_000 {
                            let user_ms = last.first_content_ms;
                            last.model = tr.s.model.clone();
                            last.out_tok = last.out_tok.saturating_add(out_t);
                            last.dur_ms = ms.saturating_sub(user_ms).max(last.ttft_ms);
                        }
                    }
                }
            }
            "GENERIC" => {
                let len = extract_json_str(line, "content")
                    .map(|c| c.len() as u64)
                    .unwrap_or(line.len() as u64);
                tr.s.weights[2] = tr.s.weights[2].saturating_add(len);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    const AGY_FIXTURE: &str = include_str!("../../../tests/fixtures/agy/main.jsonl");

    #[test]
    fn parses_agy_fixture_occupancy_weights_and_perf() {
        assert_eq!(
            agy_occupancy("Claude Opus 4.6 (Thinking)", 14_500, 18_000),
            32_500
        );
        assert_eq!(agy_occupancy("gemini-3.8-flash", 14_500, 8_000), 14_500);
        assert_eq!(agy_occupancy("", 14_500, 8_000), 14_500);
        assert_eq!(agy_occupancy("auto", 14_500, 8_000), 14_500);

        let p = env::temp_dir().join(format!("ccide_agy_ctx_{}.jsonl", std::process::id()));
        fs::write(&p, AGY_FIXTURE).unwrap();
        let mut tr = AgentCtxTracker::default();
        update_agy_ctx(&p, &mut tr);
        let _ = fs::remove_file(&p);

        assert_eq!(tr.s.model, "Claude Opus 4.6 (Thinking)");
        assert_eq!(tr.s.turns, 2);
        assert_eq!(tr.s.occupancy, 32_500);
        assert_eq!(tr.s.last_input, 14_500);
        assert_eq!(tr.s.last_cached, 18_000);
        assert_eq!(tr.s.last_output, 620);
        assert_eq!(tr.s.cum_input, 52_500);
        assert_eq!(tr.s.cum_cached, 26_000);
        assert_eq!(tr.s.cum_output, 1_070);
        assert!(tr.s.weights[2] > 0 && tr.s.weights[3] > 0 && tr.s.weights[4] > 0);
        assert_eq!(tr.s.perf.len(), 1);
        assert_eq!(tr.s.perf[0].ttft_ms, 2_000);
        assert_eq!(tr.s.perf[0].dur_ms, 6_000);
        assert_eq!(tr.s.perf[0].out_tok, 1_070);
        assert_eq!(
            crate::tui::pricing::lookup::normalize_model_key("Claude Opus 4.6 (Thinking)"),
            "claude-opus-4-6"
        );
        let cat = crate::tui::pricing::PricingCatalog::new("/nonexistent");
        assert!((cat.resolve("Claude Opus 4.6 (Thinking)").input - 5.0).abs() < 1e-6);
    }
}
