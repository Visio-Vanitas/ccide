//! OpenAI Codex CLI (`rollout-*.jsonl`) Context & Cost Parser (`src/tui/context/codex_parser.rs`).

use std::path::Path;

use crate::json::{extract_json_object, extract_json_str, extract_json_u64};
use crate::session::is_valid_model_name;
use crate::tui::context::agent_ctx::{read_new_jsonl_lines, AgentCtxTracker};
use crate::tui::context::perf::RawPerfSample;

pub fn update_codex_ctx(path: &Path, tr: &mut AgentCtxTracker) {
    let lines = read_new_jsonl_lines(path, tr);
    if lines.is_empty() {
        return;
    }
    for line in &lines {
        let rec_type = extract_json_str(line, "type").unwrap_or_default();
        let Some(payload) = extract_json_object(line, "payload") else {
            continue;
        };

        match rec_type.as_str() {
            "session_meta" => {
                if tr.s.ctx_window == 0 {
                    if let Some(w) = extract_json_u64(payload, "context_window").filter(|&w| w > 0)
                    {
                        tr.s.ctx_window = w;
                    }
                }
            }
            "turn_context" => {
                if let Some(m) =
                    extract_json_str(payload, "model").filter(|m| is_valid_model_name(m))
                {
                    tr.s.model = m;
                }
                tr.s.turns += 1;
            }
            "event_msg" => {
                let ev_type = extract_json_str(payload, "type").unwrap_or_default();
                match ev_type.as_str() {
                    "task_started" => {
                        if let Some(w) =
                            extract_json_u64(payload, "model_context_window").filter(|&w| w > 0)
                        {
                            tr.s.ctx_window = w;
                        }
                    }
                    "token_count" => {
                        let Some(info) = extract_json_object(payload, "info") else {
                            continue;
                        };
                        if let Some(w) =
                            extract_json_u64(info, "model_context_window").filter(|&w| w > 0)
                        {
                            tr.s.ctx_window = w;
                        }
                        if let Some(last) = extract_json_object(info, "last_token_usage") {
                            let lin = extract_json_u64(last, "input_tokens").unwrap_or(0);
                            let lcr = extract_json_u64(last, "cached_input_tokens").unwrap_or(0);
                            let lout = extract_json_u64(last, "output_tokens").unwrap_or(0);
                            let lreas =
                                extract_json_u64(last, "reasoning_output_tokens").unwrap_or(0);
                            tr.s.last_input = lin;
                            tr.s.last_cached = lcr;
                            tr.s.last_output = lout;
                            tr.s.last_reasoning = lreas;
                            tr.s.occupancy = lin;
                        }
                        let (d_in, d_cr, d_out) = if let Some(tot) =
                            extract_json_object(info, "total_token_usage")
                        {
                            let tin =
                                extract_json_u64(tot, "input_tokens").unwrap_or(tr.s.cum_input);
                            let tcr = extract_json_u64(tot, "cached_input_tokens")
                                .unwrap_or(tr.s.cum_cached);
                            let tcw = extract_json_u64(tot, "cache_write_input_tokens")
                                .unwrap_or(tr.s.cum_cache_write);
                            let tout =
                                extract_json_u64(tot, "output_tokens").unwrap_or(tr.s.cum_output);
                            tr.s.cum_input = tin;
                            tr.s.cum_cached = tcr;
                            tr.s.cum_cache_write = tcw;
                            tr.s.cum_output = tout;
                            let (p_in, p_cr, p_out) = tr.prev_codex_totals;
                            tr.prev_codex_totals = (tin.max(p_in), tcr.max(p_cr), tout.max(p_out));
                            (
                                tin.saturating_sub(p_in),
                                tcr.saturating_sub(p_cr),
                                tout.saturating_sub(p_out),
                            )
                        } else {
                            let cur = (tr.s.last_input, tr.s.last_cached, tr.s.last_output);
                            if cur != tr.prev_codex_totals {
                                tr.prev_codex_totals = cur;
                                tr.s.cum_input = tr.s.cum_input.saturating_add(cur.0);
                                tr.s.cum_cached = tr.s.cum_cached.saturating_add(cur.1);
                                tr.s.cum_output = tr.s.cum_output.saturating_add(cur.2);
                                cur
                            } else {
                                (0, 0, 0)
                            }
                        };
                        if (d_in | d_cr | d_out) != 0 {
                            tr.s.per_model.push((
                                tr.s.model.clone(),
                                d_in.saturating_sub(d_cr),
                                d_cr,
                                d_out,
                            ));
                        }
                    }
                    "task_complete" => {
                        let dur_ms = extract_json_u64(payload, "duration_ms").unwrap_or(0);
                        let ttft_ms =
                            extract_json_u64(payload, "time_to_first_token_ms").unwrap_or(0);
                        if dur_ms > 0 || ttft_ms > 0 {
                            tr.s.perf.push(RawPerfSample {
                                model: tr.s.model.clone(),
                                out_tok: tr.s.last_output,
                                dur_ms,
                                ttft_ms,
                                first_content_ms: ttft_ms,
                            });
                        }
                    }
                    _ => {}
                }
            }
            "response_item" => {
                let ptype = extract_json_str(payload, "type").unwrap_or_default();
                let plen = payload.len() as u64;
                match ptype.as_str() {
                    "message" => {
                        tr.s.weights[4] = tr.s.weights[4].saturating_add(plen);
                    }
                    "reasoning" => {
                        tr.s.weights[3] = tr.s.weights[3].saturating_add(plen);
                    }
                    "function_call"
                    | "function_call_output"
                    | "custom_tool_call"
                    | "custom_tool_call_output" => {
                        tr.s.weights[2] = tr.s.weights[2].saturating_add(plen);
                    }
                    _ => {}
                }
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

    const CODEX_0144: &str = include_str!("../../../tests/fixtures/codex/main_0144.jsonl");
    const CODEX_0167: &str = include_str!("../../../tests/fixtures/codex/main_0167.jsonl");

    #[test]
    fn parses_codex_0144_and_0167_rollout_variants_and_dedups_token_count() {
        let p1 = env::temp_dir().join(format!("ccide_cdx144_{}.jsonl", std::process::id()));
        let dup_line = CODEX_0144
            .lines()
            .find(|l| l.contains("\"token_count\""))
            .unwrap();
        fs::write(&p1, format!("{CODEX_0144}\n{dup_line}\n")).unwrap();
        let mut tr1 = AgentCtxTracker::default();
        update_codex_ctx(&p1, &mut tr1);
        let _ = fs::remove_file(&p1);

        assert_eq!(tr1.s.model, "gpt-5.4");
        assert_eq!(tr1.s.ctx_window, 272_000);
        assert_eq!(tr1.s.occupancy, 18_500);
        assert_eq!(tr1.s.last_cached, 12_000);
        assert_eq!(tr1.s.last_output, 340);
        assert_eq!(tr1.s.last_reasoning, 120);
        assert_eq!(tr1.s.turns, 1);
        assert_eq!(tr1.s.per_model.len(), 1);
        assert_eq!(
            tr1.s.per_model[0],
            ("gpt-5.4".to_string(), 6_500, 12_000, 340)
        );
        assert_eq!(tr1.s.perf.len(), 1);
        assert_eq!(tr1.s.perf[0].ttft_ms, 850);
        assert_eq!(tr1.s.perf[0].dur_ms, 4_200);

        let p2 = env::temp_dir().join(format!("ccide_cdx167_{}.jsonl", std::process::id()));
        fs::write(&p2, CODEX_0167).unwrap();
        let mut tr2 = AgentCtxTracker::default();
        update_codex_ctx(&p2, &mut tr2);
        let _ = fs::remove_file(&p2);

        assert_eq!(tr2.s.model, "gpt-6-codex");
        assert_eq!(tr2.s.ctx_window, 1_000_000);
        assert_eq!(tr2.s.occupancy, 9_200);
        assert_eq!(tr2.s.last_cached, 4_000);
        assert_eq!(tr2.s.last_output, 210);
        assert_eq!(tr2.s.per_model.len(), 1);
        assert_eq!(tr2.s.perf.len(), 1);
        assert_eq!(tr2.s.perf[0].ttft_ms, 610);
    }
}
