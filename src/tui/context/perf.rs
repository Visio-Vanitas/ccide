//! Per-Model Monotonic Stream (`pf_<pid>.jsonl`) & Inter-Block `.jsonl` `TPS` / `TTFT` Tracker.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};

use crate::json::{extract_json_str, extract_json_u64};
use crate::platform::ipc_path;
use crate::tui::context::json_scan::parse_iso8601_secs;
use crate::tui::context::parser::TurnRecord;

/// Parses ISO-8601 UTC timestamps (`YYYY-MM-DDTHH:MM:SS.sssZ`) to epoch milliseconds.
pub fn parse_iso8601_ms(ts: &str) -> Option<u64> {
    let secs = parse_iso8601_secs(ts)?;
    let frac_ms = ts
        .get(19..)
        .and_then(|s| s.strip_prefix('.'))
        .map(|dot| {
            let mut p: String = dot
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .take(3)
                .collect();
            while !p.is_empty() && p.len() < 3 {
                p.push('0');
            }
            p.parse::<u64>().unwrap_or(0)
        })
        .unwrap_or(0);
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
    source_key: String,
    offset: u64,
    pub samples: Vec<RawPerfSample>,
}

impl PerfStreamTracker {
    pub fn clear(&mut self) {
        self.source_key.clear();
        self.offset = 0;
        self.samples.clear();
    }

    pub fn update(&mut self, session_id: &str, claude_pid: u32) {
        let sid_path =
            (!session_id.is_empty()).then(|| ipc_path(&format!("pf_sid_{session_id}.jsonl")));
        let path = match sid_path.filter(|p| p.is_file()) {
            Some(p) => p,
            None if claude_pid > 1 => ipc_path(&format!("pf_{claude_pid}.jsonl")),
            None => return,
        };
        let key = path.to_string_lossy().into_owned();
        if self.source_key != key {
            self.source_key = key;
            self.offset = 0;
            self.samples.clear();
        }
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
        let mut raw = Vec::new();
        if File::open(&path)
            .and_then(|mut f| {
                f.seek(SeekFrom::Start(self.offset))
                    .and_then(|_| f.read_to_end(&mut raw))
            })
            .is_err()
        {
            return;
        }
        let Some(nl_pos) = raw.iter().rposition(|&b| b == b'\n') else {
            return;
        };
        for line in String::from_utf8_lossy(&raw[..=nl_pos]).lines() {
            let Some(m) = extract_json_str(line, "m") else {
                continue;
            };
            let (o, d) = (
                extract_json_u64(line, "o").unwrap_or(0),
                extract_json_u64(line, "d").unwrap_or(0),
            );
            let (t, fc) = (
                extract_json_u64(line, "t").unwrap_or(0),
                extract_json_u64(line, "f").unwrap_or(0),
            );
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
        self.offset += (nl_pos + 1) as u64;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ModelPerfAccum {
    exact_out_tok: u64,
    exact_gen_ms: u64,
    ttft_sum_ms: u64,
    ttft_cnt: u64,
    stream_out_tok: f64,
    stream_ms: f64,
    fb_out_tok: u64,
    fb_gen_ms: u64,
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
        if s.out_tok < 24 {
            return;
        }
        let gen_t = s.dur_ms.saturating_sub(eff_ttft);
        let gen_f = if s.first_content_ms > eff_ttft {
            s.dur_ms.saturating_sub(s.first_content_ms)
        } else {
            0
        };
        let span_ms = if gen_t >= 40
            && (s.out_tok.saturating_mul(1_000) / gen_t) < 20
            && gen_f >= 40
            && (s.out_tok.saturating_mul(1_000) / gen_f) <= 1_000
        {
            gen_f
        } else {
            gen_t
        };
        if (40..=600_000).contains(&span_ms) && (s.out_tok.saturating_mul(1_000) / span_ms) <= 1_000
        {
            self.exact_out_tok = self.exact_out_tok.saturating_add(s.out_tok);
            self.exact_gen_ms = self.exact_gen_ms.saturating_add(span_ms);
        }
    }

    pub fn record_fallback(&mut self, rec: &TurnRecord) {
        if rec.stream_ms >= 100 && rec.out_tok >= 20 {
            let (b0, btot) = if rec.first_vis_bytes > 0
                && rec.total_vis_bytes > rec.first_vis_bytes
                && rec.out_tok.saturating_mul(2) <= rec.total_vis_bytes
            {
                (rec.first_vis_bytes, rec.total_vis_bytes)
            } else if rec.total_bytes > rec.first_bytes {
                (rec.first_bytes, rec.total_bytes)
            } else {
                (0, 0)
            };
            if btot > b0 {
                let tail_tok = (rec.out_tok as f64) * ((btot - b0) as f64) / (btot as f64);
                let tps_i = tail_tok * 1000.0 / (rec.stream_ms as f64);
                if (5.0..=1_000.0).contains(&tps_i) {
                    self.stream_out_tok += tail_tok;
                    self.stream_ms += rec.stream_ms as f64;
                }
            }
        }
        let gen_fb = rec.dur_ms.saturating_sub(rec.first_ms);
        if rec.out_tok >= 24
            && (40..=600_000).contains(&gen_fb)
            && (rec.out_tok.saturating_mul(1_000) / gen_fb) <= 1_000
        {
            self.fb_out_tok = self.fb_out_tok.saturating_add(rec.out_tok);
            self.fb_gen_ms = self.fb_gen_ms.saturating_add(gen_fb);
        }
        if (20..=300_000).contains(&rec.first_ms) {
            let tail_ms = rec.dur_ms.saturating_sub(rec.first_ms);
            let tail_b = rec.total_bytes.saturating_sub(rec.first_bytes);
            let est = if tail_ms >= 200 && tail_b >= 64 {
                let b1_ms = ((rec.first_bytes as f64) * (tail_ms as f64) / (tail_b as f64)) as u64;
                rec.first_ms.saturating_sub(b1_ms).max(50)
            } else {
                rec.first_ms
            };
            self.fb_ttft_sum_ms = self.fb_ttft_sum_ms.saturating_add(est);
            self.fb_ttft_cnt = self.fb_ttft_cnt.saturating_add(1);
        }
    }

    pub fn resolve(&self) -> (Option<f64>, Option<u64>) {
        let exact_tps = (self.exact_gen_ms > 0 && self.exact_out_tok > 0)
            .then(|| (self.exact_out_tok as f64) * 1000.0 / (self.exact_gen_ms as f64));
        let stream_tps = (self.stream_ms > 0.0 && self.stream_out_tok > 0.0)
            .then(|| self.stream_out_tok * 1000.0 / self.stream_ms);
        let fb_tps = (self.fb_gen_ms > 0 && self.fb_out_tok > 0)
            .then(|| (self.fb_out_tok as f64) * 1000.0 / (self.fb_gen_ms as f64));
        let tps = match (exact_tps, stream_tps) {
            (Some(ex), Some(st)) if ex < st * 0.55 => Some(st),
            (Some(ex), _) => Some(ex),
            (None, Some(st)) => Some(st),
            (None, None) => fb_tps,
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
    use crate::tui::context::bar::compute_cache_hit_pct;

    #[test]
    fn computes_exact_tps_ttft_and_tracks_pf_sid_stream() {
        assert_eq!(compute_cache_hit_pct(1_000_000, 1_000_000), 99.9);
        assert_eq!(compute_cache_hit_pct(894_400, 1_000_000), 89.4);
        let mut acc = ModelPerfAccum::default();
        for (o, d, t) in [(200, 2_500, 500), (8, 60_060, 60_000)] {
            acc.record_exact(&RawPerfSample {
                model: "m".into(),
                out_tok: o,
                dur_ms: d,
                ttft_ms: t,
                first_content_ms: t,
            });
        }
        assert_eq!(acc.resolve(), (Some(100.0), Some(30_250)));

        let sid = format!("ut_{}", std::process::id());
        let p = ipc_path(&format!("pf_sid_{sid}.jsonl"));
        let _ = fs::create_dir_all(p.parent().unwrap());
        fs::write(
            &p,
            "{\"m\":\"gemini-4-carbon\",\"o\":320,\"d\":2400,\"t\":400,\"f\":400}\n",
        )
        .unwrap();
        let mut tr = PerfStreamTracker::default();
        tr.update(&sid, 0);
        let _ = fs::remove_file(&p);
        assert_eq!(tr.samples.len(), 1);
        assert_eq!(tr.samples[0].out_tok, 320);

        let mut fb = ModelPerfAccum::default();
        fb.record_fallback(&TurnRecord {
            out_tok: 400,
            stream_ms: 1_000,
            first_bytes: 5_000,
            total_bytes: 10_000,
            first_vis_bytes: 0,
            total_vis_bytes: 5_000,
            ..TurnRecord::default()
        });
        assert_eq!(fb.resolve().0, Some(200.0));
    }
}
