//! Incremental `.jsonl` Transcript Context & `tokscale` + Claude Code 2.1.3xx Usage Parser.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::json::{extract_json_bool, extract_json_object, extract_json_str, extract_json_u64};
pub use crate::tui::context::json_scan::{
    classify_message_bytes, extract_cache_creation_pair, extract_json_array_slice,
    extract_string_list_in_array, iter_array_objects, parse_iso8601_secs, BlobWeights,
};
use crate::tui::context::perf::parse_iso8601_ms;

#[derive(Clone, Debug)]
pub struct TurnRecord {
    pub model: String,
    pub in_tok: u64,
    pub cc_tok: u64,
    pub cc_1h_tok: u64,
    pub cr_tok: u64,
    pub out_tok: u64,
    pub first_ms: u64,
    pub dur_ms: u64,
    pub first_bytes: u64,
    pub total_bytes: u64,
    pub fast_mode: bool,
    pub us_geo: bool,
    pub web_search_reqs: u64,
}

#[derive(Default)]
pub struct TranscriptCtxTracker {
    pub offset: u64,
    pub shapes: HashMap<String, (u64, u64)>,
    pub blobs: HashMap<String, BlobWeights>,
    pub live_hashes: Vec<String>,
    pub active_shape: (u64, u64),
    pub has_api_requests: bool,
    pub fb_weights: BlobWeights,
    pub turns_by_id: HashMap<String, TurnRecord>,
    pub anon_seq: usize,
    pub first_ts_s: Option<u64>,
    pub last_ts_s: Option<u64>,
    pub last_req_ts_ms: Option<u64>,
    pub in_tok: u64,
    pub cc_tok: u64,
    pub cr_tok: u64,
}

pub fn update_transcript_ctx_tracker(jpath: &Path, is_sub: bool, tr: &mut TranscriptCtxTracker) {
    let Ok(meta) = fs::metadata(jpath) else {
        return;
    };
    let len = meta.len();
    if len < tr.offset {
        *tr = TranscriptCtxTracker::default();
    }
    if len == tr.offset {
        return;
    }
    let Ok(mut f) = File::open(jpath) else {
        return;
    };
    if f.seek(SeekFrom::Start(tr.offset)).is_err() {
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
        if line.contains("\"api-request-shape\"") {
            if let (Some(sh_hash), Some(sh_obj)) = (
                extract_json_str(line, "shapeHash"),
                extract_json_object(line, "shape"),
            ) {
                let sys_len = extract_json_array_slice(sh_obj, "system")
                    .map(|s| s.len() as u64)
                    .unwrap_or(0);
                let tools_len = extract_json_array_slice(sh_obj, "tools")
                    .map(|s| s.len() as u64)
                    .unwrap_or(0);
                tr.shapes.insert(sh_hash, (sys_len, tools_len));
            }
        } else if line.contains("\"api-request-blob\"") {
            if let (Some(b_hash), Some(msg_obj)) = (
                extract_json_str(line, "hash"),
                extract_json_object(line, "message"),
            ) {
                tr.blobs.insert(b_hash, classify_message_bytes(msg_obj));
            }
        } else if line.contains("\"type\":\"api-request\"")
            || line.contains("\"type\": \"api-request\"")
        {
            let qsrc = extract_json_str(line, "querySource").unwrap_or_default();
            let main_ok = qsrc.is_empty()
                || qsrc.starts_with("repl_main_thread")
                || qsrc == "sdk"
                || qsrc == "compact";
            if is_sub || main_ok {
                tr.has_api_requests = true;
                if let Some(ms) =
                    extract_json_str(line, "timestamp").and_then(|s| parse_iso8601_ms(&s))
                {
                    tr.last_req_ts_ms = Some(ms);
                }
                let keep = (extract_json_u64(line, "keep").unwrap_or(0) as usize)
                    .min(tr.live_hashes.len());
                tr.live_hashes.truncate(keep);
                if let Some(tail_arr) = extract_json_array_slice(line, "tail") {
                    tr.live_hashes
                        .extend(extract_string_list_in_array(tail_arr));
                }
                if let Some(sh_hash) = extract_json_str(line, "shapeHash") {
                    if let Some(&sh_pair) = tr.shapes.get(&sh_hash) {
                        tr.active_shape = sh_pair;
                    }
                }
            }
        } else {
            if line.contains("\"compact_boundary\"") && !line.contains("\"preservedSegment\"") {
                tr.fb_weights = BlobWeights::default();
            }
            if extract_json_bool(line, "isUnmetered") == Some(true) {
                continue;
            }
            let Some(msg_obj) = extract_json_object(line, "message") else {
                continue;
            };
            if !tr.has_api_requests {
                tr.fb_weights.add_assign(classify_message_bytes(msg_obj));
            }
            if extract_json_str(msg_obj, "role").as_deref() == Some("user") {
                if let Some(ms) =
                    extract_json_str(line, "timestamp").and_then(|s| parse_iso8601_ms(&s))
                {
                    tr.last_req_ts_ms = Some(ms);
                }
                continue;
            }
            let msg_model = extract_json_str(msg_obj, "model").unwrap_or_default();
            if msg_model == "<synthetic>" {
                continue;
            }
            let Some(usage_obj) = extract_json_object(msg_obj, "usage") else {
                continue;
            };
            let mut in_t = extract_json_u64(usage_obj, "input_tokens").unwrap_or(0);
            let (mut cc_t, mut cc_1h) = extract_cache_creation_pair(usage_obj);
            let mut cr_t = extract_json_u64(usage_obj, "cache_read_input_tokens").unwrap_or(0);
            let mut out_t = extract_json_u64(usage_obj, "output_tokens").unwrap_or(0);
            let fast_mode = extract_json_str(usage_obj, "speed")
                .map(|s| s.eq_ignore_ascii_case("fast"))
                .unwrap_or(false);
            let us_geo = extract_json_str(usage_obj, "inference_geo")
                .map(|s| s.eq_ignore_ascii_case("us"))
                .unwrap_or(false);
            let web_reqs = extract_json_object(usage_obj, "server_tool_use")
                .and_then(|o| extract_json_u64(o, "web_search_requests"))
                .unwrap_or(0);

            let (mut ctx_in, mut ctx_cc, mut ctx_cr) = (in_t, cc_t, cr_t);
            let mid = extract_json_str(msg_obj, "id").unwrap_or_default();
            let rid = extract_json_str(line, "requestId").unwrap_or_default();
            let base_key = match (mid.is_empty(), rid.is_empty()) {
                (false, false) => format!("{mid}:{rid}"),
                (false, true) => mid,
                (true, false) => rid,
                (true, true) => {
                    tr.anon_seq += 1;
                    format!("anon_{}", tr.anon_seq)
                }
            };

            if let Some(iters_arr) = extract_json_array_slice(usage_obj, "iterations") {
                let mut sum_msg = (0u64, 0u64, 0u64, 0u64, 0u64);
                let mut has_msg_iters = false;
                for (idx_it, it_obj) in iter_array_objects(iters_arr).iter().enumerate() {
                    let it_type = extract_json_str(it_obj, "type").unwrap_or_default();
                    let i_in = extract_json_u64(it_obj, "input_tokens").unwrap_or(0);
                    let (i_cc, i_1h) = extract_cache_creation_pair(it_obj);
                    let i_cr = extract_json_u64(it_obj, "cache_read_input_tokens").unwrap_or(0);
                    let i_out = extract_json_u64(it_obj, "output_tokens").unwrap_or(0);
                    if it_type == "advisor_message" && (i_in + i_cc + i_cr + i_out > 0) {
                        let adv_m = extract_json_str(it_obj, "model").unwrap_or_default();
                        tr.turns_by_id.insert(
                            format!("{base_key}:adv:{idx_it}"),
                            TurnRecord {
                                model: adv_m,
                                in_tok: i_in,
                                cc_tok: i_cc,
                                cc_1h_tok: i_1h,
                                cr_tok: i_cr,
                                out_tok: i_out,
                                first_ms: 0,
                                dur_ms: 0,
                                first_bytes: 0,
                                total_bytes: 0,
                                fast_mode: false,
                                us_geo,
                                web_search_reqs: 0,
                            },
                        );
                    } else if matches!(it_type.as_str(), "message" | "fallback_message") {
                        has_msg_iters = true;
                        ctx_in = i_in;
                        ctx_cc = i_cc;
                        ctx_cr = i_cr;
                        sum_msg.0 = sum_msg.0.saturating_add(i_in);
                        sum_msg.1 = sum_msg.1.saturating_add(i_cc);
                        sum_msg.2 = sum_msg.2.saturating_add(i_1h);
                        sum_msg.3 = sum_msg.3.saturating_add(i_cr);
                        sum_msg.4 = sum_msg.4.saturating_add(i_out);
                    }
                }
                if has_msg_iters {
                    (in_t, cc_t, cc_1h, cr_t, out_t) = sum_msg;
                }
            }

            if ctx_in + ctx_cc + ctx_cr > 0 || in_t + cc_t + cr_t + out_t > 0 {
                if ctx_in + ctx_cc + ctx_cr > 0 {
                    (tr.in_tok, tr.cc_tok, tr.cr_tok) = (ctx_in, ctx_cc, ctx_cr);
                }
                let ts_raw = extract_json_str(line, "timestamp");
                if let Some(ts_s) = ts_raw.as_deref().and_then(parse_iso8601_secs) {
                    if tr.first_ts_s.is_none() {
                        tr.first_ts_s = Some(ts_s);
                    }
                    tr.last_ts_s = Some(ts_s);
                }
                let dur_ms = match (
                    tr.last_req_ts_ms,
                    ts_raw.as_deref().and_then(parse_iso8601_ms),
                ) {
                    (Some(t0), Some(t1)) if t1 > t0 => t1 - t0,
                    _ => 0,
                };
                let blk_b = extract_json_array_slice(msg_obj, "content")
                    .map(|s| s.len() as u64)
                    .unwrap_or(0);
                let prev = tr.turns_by_id.get(&base_key);
                let first_ms = prev
                    .map(|p| p.first_ms)
                    .filter(|&m| m > 0)
                    .unwrap_or(dur_ms);
                let first_bytes = prev
                    .map(|p| p.first_bytes)
                    .filter(|&b| b > 0)
                    .unwrap_or(blk_b);
                let total_bytes = prev
                    .map(|p| p.total_bytes.saturating_add(blk_b))
                    .unwrap_or(blk_b);
                tr.turns_by_id.insert(
                    base_key,
                    TurnRecord {
                        model: msg_model,
                        in_tok: in_t,
                        cc_tok: cc_t,
                        cc_1h_tok: cc_1h,
                        cr_tok: cr_t,
                        out_tok: out_t,
                        first_ms,
                        dur_ms,
                        first_bytes,
                        total_bytes,
                        fast_mode,
                        us_geo,
                        web_search_reqs: web_reqs,
                    },
                );
            }
        }
    }
    tr.offset += consumed;
}
