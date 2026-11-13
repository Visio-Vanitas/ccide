//! Incremental `.jsonl` Transcript Context & `tokscale`-Style Turn Deduplication Parser.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::json::{extract_json_object, extract_json_str, extract_json_u64};

/// Approximate text-equivalent byte weight (`~1.6k` vision tokens) for a base64 image block.
pub const IMAGE_EQUIV_BYTES: u64 = 6_400;

#[derive(Clone, Copy, Debug, Default)]
pub struct BlobWeights {
    pub sys_md: u64,
    pub tool_io: u64,
    pub thinking: u64,
    pub chat: u64,
}

impl BlobWeights {
    pub fn add_assign(&mut self, other: Self) {
        self.sys_md = self.sys_md.saturating_add(other.sys_md);
        self.tool_io = self.tool_io.saturating_add(other.tool_io);
        self.thinking = self.thinking.saturating_add(other.thinking);
        self.chat = self.chat.saturating_add(other.chat);
    }
}

#[derive(Clone, Debug)]
pub struct TurnRecord {
    pub model: String,
    pub in_tok: u64,
    pub cc_tok: u64,
    pub cc_1h_tok: u64,
    pub cr_tok: u64,
    pub out_tok: u64,
}

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
    pub in_tok: u64,
    pub cc_tok: u64,
    pub cr_tok: u64,
}

impl TranscriptCtxTracker {
    pub fn new() -> Self {
        Self {
            offset: 0,
            shapes: HashMap::new(),
            blobs: HashMap::new(),
            live_hashes: Vec::new(),
            active_shape: (0, 0),
            has_api_requests: false,
            fb_weights: BlobWeights::default(),
            turns_by_id: HashMap::new(),
            anon_seq: 0,
            first_ts_s: None,
            last_ts_s: None,
            in_tok: 0,
            cc_tok: 0,
            cr_tok: 0,
        }
    }
}

fn find_balanced_span(bytes: &[u8], start: usize, open_b: u8, close_b: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_str = false;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == open_b {
            depth += 1;
        } else if b == close_b {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn extract_json_array_slice<'a>(src: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{}\"", key);
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(&needle) {
        let pos = from + rel + needle.len();
        from = pos;
        let rest = src[pos..].trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let val = rest[1..].trim_start();
        if !val.starts_with('[') {
            continue;
        }
        let arr_start = src.len() - val.len();
        let end = find_balanced_span(src.as_bytes(), arr_start, b'[', b']')?;
        return Some(&src[arr_start..=end]);
    }
    None
}

fn extract_string_list_in_array(arr_slice: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = arr_slice.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'"' {
                    break;
                }
                j += 1;
            }
            if j < bytes.len() {
                out.push(arr_slice[start..j].to_string());
                i = j + 1;
                continue;
            }
            break;
        }
        i += 1;
    }
    out
}

pub fn classify_message_bytes(msg_slice: &str) -> BlobWeights {
    let mut w = BlobWeights::default();
    if let Some(arr) = extract_json_array_slice(msg_slice, "content") {
        let bytes = arr.as_bytes();
        let mut i = 1usize;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                let Some(j) = find_balanced_span(bytes, i, b'{', b'}') else {
                    break;
                };
                let blk = &arr[i..=j];
                let blk_len = blk.len() as u64;
                match extract_json_str(blk, "type").unwrap_or_default().as_str() {
                    "tool_use" | "tool_result" | "server_tool_use" => w.tool_io += blk_len,
                    "thinking" | "redacted_thinking" => w.thinking += blk_len,
                    "image" => w.chat += IMAGE_EQUIV_BYTES,
                    _ if blk.contains("<system-reminder>") => w.sys_md += blk_len,
                    _ => w.chat += blk_len,
                }
                i = j + 1;
                continue;
            }
            i += 1;
        }
    } else if let Some(s) = extract_json_str(msg_slice, "content") {
        let slen = s.len() as u64;
        if s.contains("<system-reminder>") {
            w.sys_md += slen;
        } else {
            w.chat += slen;
        }
    }
    w
}

fn parse_iso8601_secs(ts: &str) -> Option<u64> {
    if ts.len() < 19 {
        return None;
    }
    let y: i64 = ts.get(0..4)?.parse().ok()?;
    let m: i64 = ts.get(5..7)?.parse().ok()?;
    let d: i64 = ts.get(8..10)?.parse().ok()?;
    let hh: u64 = ts.get(11..13)?.parse().ok()?;
    let mm: u64 = ts.get(14..16)?.parse().ok()?;
    let ss: u64 = ts.get(17..19)?.parse().ok()?;
    let y_adj = y - if m <= 2 { 1 } else { 0 };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = (y_adj - era * 400) as u64;
    let mp = (if m > 2 { m - 3 } else { m + 9 }) as u64;
    let doy = (153 * mp + 2) / 5 + (d as u64).saturating_sub(1);
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + (doe as i64) - 719_468;
    (days >= 0).then(|| (days as u64) * 86_400 + hh * 3_600 + mm * 60 + ss)
}

pub fn update_transcript_ctx_tracker(jpath: &Path, is_sub: bool, tr: &mut TranscriptCtxTracker) {
    let Ok(meta) = fs::metadata(jpath) else {
        return;
    };
    let len = meta.len();
    if len < tr.offset {
        *tr = TranscriptCtxTracker::new();
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
    for line in String::from_utf8_lossy(&raw).lines() {
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
            if is_sub || qsrc.is_empty() || qsrc == "repl_main_thread" {
                tr.has_api_requests = true;
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
            if line.contains("\"compact_boundary\"") {
                tr.fb_weights = BlobWeights::default();
            }
            if let Some(msg_obj) = extract_json_object(line, "message") {
                if !tr.has_api_requests {
                    tr.fb_weights.add_assign(classify_message_bytes(msg_obj));
                }
                if let Some(usage_obj) = extract_json_object(msg_obj, "usage") {
                    let in_t = extract_json_u64(usage_obj, "input_tokens").unwrap_or(0);
                    let cc_t =
                        extract_json_u64(usage_obj, "cache_creation_input_tokens").unwrap_or(0);
                    let cc_1h = extract_json_object(usage_obj, "cache_creation")
                        .and_then(|o| extract_json_u64(o, "ephemeral_1h_input_tokens"))
                        .unwrap_or(0);
                    let cr_t = extract_json_u64(usage_obj, "cache_read_input_tokens").unwrap_or(0);
                    let out_t = extract_json_u64(usage_obj, "output_tokens").unwrap_or(0);
                    if in_t + cc_t + cr_t > 0 {
                        tr.in_tok = in_t;
                        tr.cc_tok = cc_t;
                        tr.cr_tok = cr_t;
                        if let Some(ts_s) =
                            extract_json_str(line, "timestamp").and_then(|s| parse_iso8601_secs(&s))
                        {
                            if tr.first_ts_s.is_none() {
                                tr.first_ts_s = Some(ts_s);
                            }
                            tr.last_ts_s = Some(ts_s);
                        }
                        let mid = extract_json_str(msg_obj, "id").unwrap_or_default();
                        let rid = extract_json_str(line, "requestId").unwrap_or_default();
                        let key = match (mid.is_empty(), rid.is_empty()) {
                            (false, false) => format!("{mid}:{rid}"),
                            (false, true) => mid,
                            (true, false) => rid,
                            (true, true) => {
                                tr.anon_seq += 1;
                                format!("anon_{}", tr.anon_seq)
                            }
                        };
                        tr.turns_by_id.insert(
                            key,
                            TurnRecord {
                                model: extract_json_str(msg_obj, "model").unwrap_or_default(),
                                in_tok: in_t,
                                cc_tok: cc_t,
                                cc_1h_tok: cc_1h,
                                cr_tok: cr_t,
                                out_tok: out_t,
                            },
                        );
                    }
                }
            }
        }
    }
    tr.offset = len;
}
