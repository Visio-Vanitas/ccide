//! Per-Model Monotonic Stream (`pf_<pid>.jsonl`) & `.jsonl` Fallback `TPS` / `TTFT` Tracker.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};

use crate::json::{extract_json_str, extract_json_u64};
use crate::platform::ipc_path;
use crate::tui::context::json_scan::parse_iso8601_secs;

/// Parses ISO-8601 UTC timestamps (`YYYY-MM-DDTHH:MM:SS.sssZ`) to epoch milliseconds.
pub fn parse_iso8601_ms(ts: &str) -> Option<u64> {
    let secs = parse_iso8601_secs(ts)?;
    let mut frac_ms = 0u64;
    if let Some(dot) = ts.get(19..).and_then(|s| s.strip_prefix('.')) {
        let digits: String = dot.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            let mut padded = digits;
            padded.truncate(3);
            while padded.len() < 3 {
                padded.push('0');
            }
            frac_ms = padded.parse::<u64>().unwrap_or(0);
        }
    }
    Some(secs.saturating_mul(1_000).saturating_add(frac_ms))
}

#[derive(Clone, Debug)]
pub struct RawPerfSample {
    pub model: String,
    pub out_tok: u64,
    pub dur_ms: u64,
    pub ttft_ms: u64,
    pub first_content_ms: u64,
}

#[derive(Default)]
pub struct PerfStreamTracker {
    pid: u32,
    offset: u64,
    pub samples: Vec<RawPerfSample>,
}

impl PerfStreamTracker {
    pub fn clear(&mut self) {
        self.pid = 0;
        self.offset = 0;
        self.samples.clear();
    }

    pub fn update(&mut self, claude_pid: u32) {
        if claude_pid <= 1 {
            return;
        }
        if self.pid != claude_pid {
            self.pid = claude_pid;
            self.offset = 0;
            self.samples.clear();
        }
        let path = ipc_path(&format!("pf_{}.jsonl", claude_pid));
        let Ok(meta) = fs::metadata(&path) else {
            return;
        };
        let len = meta.len();
        if len < self.offset {
            self.offset = 0;
            self.samples.clear();
        }
        if len == self.offset {
            return;
        }
        let Ok(mut f) = File::open(&path) else {
            return;
        };
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut raw = Vec::new();
        if f.read_to_end(&mut raw).is_err() {
            return;
        }
        let Some(nl_pos) = raw.iter().rposition(|&b| b == b'\n') else {
            return;
        };
        let consumed = (nl_pos + 1) as u64;
        for line in String::from_utf8_lossy(&raw[..=nl_pos]).lines() {
            let Some(m) = extract_json_str(line, "m") else {
                continue;
            };
            let o = extract_json_u64(line, "o").unwrap_or(0);
            let d = extract_json_u64(line, "d").unwrap_or(0);
            let t = extract_json_u64(line, "t").unwrap_or(0);
            let fc = extract_json_u64(line, "f").unwrap_or(0);
            if o > 0 || t > 0 || fc > 0 {
                self.samples.push(RawPerfSample {
                    model: m,
                    out_tok: o,
                    dur_ms: d,
                    ttft_ms: t,
                    first_content_ms: fc,
                });
            }
        }
        self.offset += consumed;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ModelPerfAccum {
    exact_out_tok: u64,
    exact_gen_ms: u64,
    ttft_sum_ms: u64,
    ttft_cnt: u64,
    fb_out_tok: u64,
    fb_dur_ms: u64,
    fb_ttft_sum_ms: u64,
    fb_ttft_cnt: u64,
}

impl ModelPerfAccum {
    pub fn record_exact(&mut self, s: &RawPerfSample) {
        let eff_ttft = if s.ttft_ms > 0 {
            s.ttft_ms
        } else {
            s.first_content_ms
        };
        if (1..=300_000).contains(&eff_ttft) {
            self.ttft_sum_ms = self.ttft_sum_ms.saturating_add(eff_ttft);
            self.ttft_cnt = self.ttft_cnt.saturating_add(1);
        }
        let gen_ms = s.dur_ms.saturating_sub(eff_ttft);
        // Guard buffered proxies (`gen_ms < 250ms` or `> 800 tok/s` burst) -> fall back to `dur_ms`
        let span_ms = if gen_ms >= 250 && (s.out_tok.saturating_mul(1_000) / gen_ms) <= 800 {
            gen_ms
        } else {
            s.dur_ms
        };
        if s.out_tok > 0 && (20..=600_000).contains(&span_ms) {
            self.exact_out_tok = self.exact_out_tok.saturating_add(s.out_tok);
            self.exact_gen_ms = self.exact_gen_ms.saturating_add(span_ms);
        }
    }

    pub fn record_fallback(
        &mut self,
        out_tok: u64,
        first_ms: u64,
        dur_ms: u64,
        first_bytes: u64,
        total_bytes: u64,
    ) {
        if out_tok > 0 && (50..=600_000).contains(&dur_ms) {
            self.fb_out_tok = self.fb_out_tok.saturating_add(out_tok);
            self.fb_dur_ms = self.fb_dur_ms.saturating_add(dur_ms);
        }
        if (20..=300_000).contains(&first_ms) {
            let tail_ms = dur_ms.saturating_sub(first_ms);
            let tail_b = total_bytes.saturating_sub(first_bytes);
            let est = if tail_ms >= 200 && tail_b >= 64 {
                let b1_ms = ((first_bytes as f64) * (tail_ms as f64) / (tail_b as f64)) as u64;
                first_ms.saturating_sub(b1_ms).max(50)
            } else {
                first_ms
            };
            self.fb_ttft_sum_ms = self.fb_ttft_sum_ms.saturating_add(est);
            self.fb_ttft_cnt = self.fb_ttft_cnt.saturating_add(1);
        }
    }

    pub fn resolve(&self) -> (Option<f64>, Option<u64>) {
        let tps = if self.exact_gen_ms > 0 && self.exact_out_tok > 0 {
            Some((self.exact_out_tok as f64) * 1000.0 / (self.exact_gen_ms as f64))
        } else if self.fb_dur_ms > 0 && self.fb_out_tok > 0 {
            Some((self.fb_out_tok as f64) * 1000.0 / (self.fb_dur_ms as f64))
        } else {
            None
        };
        let ttft_ms = self
            .ttft_sum_ms
            .checked_div(self.ttft_cnt)
            .or_else(|| self.fb_ttft_sum_ms.checked_div(self.fb_ttft_cnt));
        (tps, ttft_ms)
    }
}

pub type PerfAccumMap = HashMap<String, ModelPerfAccum>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::statusline::{compute_context_budget, CompactConfig};
    use crate::tui::context::bar::compute_cache_hit_pct;
    use crate::tui::context::{
        build_context_panel, build_cost_panel, ContextSnapshot, ModelCostEntry,
    };
    use crate::tui::palette::Palette;

    #[test]
    fn computes_exact_tps_ttft_cache_tenths_and_3_2_1_cost_modes() {
        assert_eq!(compute_cache_hit_pct(1_000_000, 1_000_000), 99.9);
        assert_eq!(compute_cache_hit_pct(999_990, 1_000_000), 99.9);
        assert_eq!(compute_cache_hit_pct(894_400, 1_000_000), 89.4);
        assert_eq!(
            parse_iso8601_ms("2026-11-13T16:58:11.464Z").unwrap()
                - parse_iso8601_ms("2026-11-13T16:58:11.000Z").unwrap(),
            464
        );

        let mut acc = ModelPerfAccum::default();
        acc.record_exact(&RawPerfSample {
            model: "gemini-4-carbon".into(),
            out_tok: 200,
            dur_ms: 2_500,
            ttft_ms: 500,
            first_content_ms: 520,
        });
        let (tps, ttft) = acc.resolve();
        assert!((tps.unwrap() - 100.0).abs() < 0.01);
        assert_eq!(ttft, Some(500));

        let pal = Palette::default_palette();
        let snap = ContextSnapshot {
            is_subagent: false,
            model_win_tokens: 1_000_000,
            budget: compute_context_budget(445_000, 1_000_000, &CompactConfig::default()),
            total_in_tokens: 445_000,
            sys_md_tokens: 12_000,
            tools_tokens: 41_000,
            tool_io_tokens: 183_000,
            thinking_tokens: 190_000,
            chat_tokens: 19_000,
            cum_cache_read_tokens: 130_250_000,
            session_hit_pct: 99.9,
            last_hit_pct: 98.4,
            turns: 391,
            session_turns: 536,
            cost_usd: 12.48,
            saved_usd: 36.12,
            saved_pct: 75,
            burn_usd_per_hr: Some(5.14),
            rl_5h_pct: None,
            rl_7d_pct: None,
            lines_added: 0,
            lines_removed: 0,
            model_costs: vec![
                ModelCostEntry {
                    model_short: "gemini-4-carbon".into(),
                    family: "gemini",
                    turns: 10,
                    input_tokens: 50_000,
                    cache_read_tokens: 900_000,
                    output_tokens: 8_000,
                    cost_usd: 12.0,
                    saved_usd: 0.0,
                    share_pct: 96,
                    tps: Some(68.4),
                    ttft_ms: Some(420),
                },
                ModelCostEntry {
                    model_short: "deepseek-v4.1-flash".into(),
                    family: "deepseek",
                    turns: 4,
                    input_tokens: 10_000,
                    cache_read_tokens: 100_000,
                    output_tokens: 2_000,
                    cost_usd: 0.48,
                    saved_usd: 0.0,
                    share_pct: 4,
                    tps: Some(115.0),
                    ttft_ms: Some(1240),
                },
            ],
        };
        let ctx_p = build_context_panel(&snap, 42, 11, false, false, &pal);
        assert!(ctx_p
            .rows
            .iter()
            .any(|r| r.text.contains("99.9%") && r.text.contains("98.4%")));
        let (c3, max_sc) = build_cost_panel(&snap, 42, 9, 0, 3, true, &pal);
        assert!(
            max_sc > 0
                && c3.rows.iter().any(|r| r.text.contains("TPS")
                    && r.text.contains("HIT")
                    && r.text.contains("TTFT")
                    && !r.text.contains('⚡'))
        );
        let (c2, _) = build_cost_panel(&snap, 42, 14, 0, 2, false, &pal);
        assert!(c2.rows.iter().any(|r| r.text.contains("50.00k")
            && r.text.contains("900.0k")
            && r.text.contains("8.00k")
            && !r.text.contains("TPS")));
        let (c1, _) = build_cost_panel(&snap, 42, 14, 0, 1, false, &pal);
        assert!(c1
            .rows
            .iter()
            .all(|r| !r.text.contains("50.00k") && !r.text.contains("TPS")));
    }
}
