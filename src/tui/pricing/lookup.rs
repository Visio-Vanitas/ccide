//! `tokscale`-aligned model key normalization, prefix/suffix stripping, and JSONC rate loader.

use std::collections::HashMap;

use crate::json::{extract_json_f64, extract_json_u64};
use crate::tui::pricing::ModelRate;

const PROVIDER_PREFIXES: &[&str] = &[
    "us.anthropic.",
    "eu.anthropic.",
    "apac.anthropic.",
    "anthropic.",
    "anthropic/",
    "openai/",
    "google/",
    "gemini/",
    "vertex_ai/",
    "bedrock/",
    "deepseek/",
    "x-ai/",
    "xai/",
    "openrouter/",
    "azure/",
    "azure_ai/",
    "fireworks_ai/",
    "together_ai/",
    "groq/",
];

/// Generic brand/routing tokens blocked from standalone fuzzy matching (`tokscale` discipline).
const FUZZY_BLOCKLIST: &[&str] = &[
    "auto",
    "mini",
    "chat",
    "base",
    "claude",
    "anthropic",
    "gemini",
    "deepseek",
    "openai",
    "grok",
    "model",
    "router",
    "default",
    "pro",
    "flash",
];

const MAX_SUFFIX_STRIP_SEGMENTS: usize = 4;

pub fn strip_jsonc(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    let mut in_str = false;
    while i < n {
        if in_str {
            if bytes[i] == b'\\' && i + 1 < n {
                if let Some(bs) = raw[i..].chars().next() {
                    out.push(bs);
                    i += bs.len_utf8();
                }
                if let Some(ch) = raw[i..].chars().next() {
                    out.push(ch);
                    i += ch.len_utf8();
                }
                continue;
            }
            if bytes[i] == b'"' {
                in_str = false;
            }
            if let Some(ch) = raw[i..].chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
        } else if bytes[i] == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
        } else if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
        } else if let Some(ch) = raw[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            i += 1;
        }
    }
    out
}

pub fn parse_section_into(section_obj: &str, dst: &mut HashMap<String, ModelRate>) {
    let bytes = section_obj.as_bytes();
    let n = bytes.len();
    let mut i = 1usize;
    while i < n {
        if bytes[i] == b'"' {
            let k_start = i + 1;
            let mut k_end = k_start;
            while k_end < n {
                if bytes[k_end] == b'\\' {
                    k_end += 2;
                    continue;
                }
                if bytes[k_end] == b'"' {
                    break;
                }
                k_end += 1;
            }
            if k_end >= n {
                break;
            }
            let key = section_obj[k_start..k_end].trim().to_ascii_lowercase();
            if key.starts_with("github_copilot/") {
                i = k_end + 1;
                continue;
            }
            let mut j = k_end + 1;
            while j < n && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < n && bytes[j] == b':' {
                j += 1;
                while j < n && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < n && bytes[j] == b'{' {
                    let obj_start = j;
                    let mut depth = 0usize;
                    let mut in_str = false;
                    while j < n {
                        let b = bytes[j];
                        if in_str {
                            if b == b'\\' {
                                j += 2;
                                continue;
                            }
                            if b == b'"' {
                                in_str = false;
                            }
                        } else {
                            match b {
                                b'"' => in_str = true,
                                b'{' => depth += 1,
                                b'}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        if let Some(rate) =
                                            parse_rate_obj(&section_obj[obj_start..=j], &key)
                                        {
                                            dst.insert(key, rate);
                                        }
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        j += 1;
                    }
                }
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }
}

fn parse_rate_obj(obj: &str, key: &str) -> Option<ModelRate> {
    let input = extract_json_f64(obj, "input")?;
    let output = extract_json_f64(obj, "output")?;
    let cw5 = extract_json_f64(obj, "cache_write_5m")
        .or_else(|| extract_json_f64(obj, "cache_write"))
        .unwrap_or(input);
    let cw1h = extract_json_f64(obj, "cache_write_1h").unwrap_or(cw5);
    let cr = extract_json_f64(obj, "cache_read").unwrap_or(input * 0.1);
    let above_tokens = extract_json_u64(obj, "above_tokens").filter(|&v| v > 0);
    let request_wide_tier = key.contains("haiku-5-5")
        || crate::json::extract_json_bool(obj, "request_wide_tier") == Some(true);
    Some(ModelRate {
        input,
        output,
        cache_write_5m: cw5,
        cache_write_1h: cw1h,
        cache_read: cr,
        above_tokens,
        request_wide_tier,
        long_input: extract_json_f64(obj, "long_input").unwrap_or(input),
        long_output: extract_json_f64(obj, "long_output").unwrap_or(output),
        long_cache_write: extract_json_f64(obj, "long_cache_write").unwrap_or(cw5),
        long_cache_read: extract_json_f64(obj, "long_cache_read").unwrap_or(cr),
    })
}

pub fn normalize_model_key(raw: &str) -> String {
    let mut s = raw.trim().to_ascii_lowercase();
    for sep in ['[', '('] {
        if let Some(idx) = s.find(sep) {
            s.truncate(idx);
        }
    }
    for pfx in PROVIDER_PREFIXES {
        if let Some(rest) = s.strip_prefix(pfx) {
            s = rest.to_string();
        }
    }
    if let Some(idx) = s.find('@') {
        s.truncate(idx);
    }
    if let Some(last_slash) = s.rsplit('/').next() {
        s = last_slash.to_string();
    }
    s.trim().to_string()
}

pub fn lookup_in_maps(
    norm: &str,
    overrides: &HashMap<String, ModelRate>,
    rates: &HashMap<String, ModelRate>,
) -> Option<ModelRate> {
    if norm.is_empty() {
        return None;
    }
    for map in [overrides, rates] {
        if let Some(&r) = map.get(norm) {
            return Some(r);
        }
    }
    // Progressive suffix stripping (`-tiered`, `-xhigh`, `-preview`, `-20251001`, `-v1:0`)
    let mut cand = norm.to_string();
    for _ in 0..MAX_SUFFIX_STRIP_SEGMENTS {
        if let Some((head, _)) = cand.rsplit_once('-') {
            if head.len() < 5 || FUZZY_BLOCKLIST.contains(&head) {
                break;
            }
            for map in [overrides, rates] {
                if let Some(&r) = map.get(head) {
                    return Some(r);
                }
            }
            cand = head.to_string();
        } else {
            break;
        }
    }
    // Longest deterministic prefix match (guarding against `FUZZY_BLOCKLIST`)
    if norm.len() >= 5 && !FUZZY_BLOCKLIST.contains(&norm) {
        for map in [overrides, rates] {
            let mut best: Option<(usize, &str, ModelRate)> = None;
            for (k, &r) in map {
                if k.len() >= 5
                    && !FUZZY_BLOCKLIST.contains(&k.as_str())
                    && (norm.starts_with(k.as_str()) || k.starts_with(norm))
                {
                    let len = k.len().min(norm.len());
                    if best
                        .map(|(bl, bk, _)| len > bl || (len == bl && k.as_str() < bk))
                        .unwrap_or(true)
                    {
                        best = Some((len, k.as_str(), r));
                    }
                }
            }
            if let Some((_, _, r)) = best {
                return Some(r);
            }
        }
    }
    None
}
