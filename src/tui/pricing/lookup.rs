//! `tokscale`-aligned model key normalization, prefix/suffix stripping, and JSONC rate loader.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::json::{extract_json_f64, extract_json_u64, iter_object_members};
use crate::tui::pricing::ModelRate;

pub fn resolve_pricing_path(home: &str) -> PathBuf {
    if let Ok(explicit) = env::var("CCIDE_PRICING_FILE") {
        let t = explicit.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        let t = xdg.trim();
        if !t.is_empty() {
            let p = Path::new(t).join("cc-ide/pricing.json");
            if p.exists() {
                return p;
            }
        }
    }
    Path::new(home).join(".config/cc-ide/pricing.json")
}

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

const EFFORT_SUFFIXES: &[&str] = &["-low", "-medium", "-med", "-high", "-xhigh", "-max"];
const MAX_SUFFIX_STRIP_SEGMENTS: usize = 4;

pub fn parse_section_into(section_obj: &str, dst: &mut HashMap<String, ModelRate>) {
    for (raw_k, val_slice) in iter_object_members(section_obj) {
        let key = raw_k.trim().to_ascii_lowercase();
        if key.starts_with("github_copilot/") || !val_slice.starts_with('{') {
            continue;
        }
        if let Some(rate) = parse_rate_obj(val_slice, &key) {
            let norm_k = normalize_model_key(&key);
            if !norm_k.is_empty() && norm_k != key {
                dst.insert(norm_k, rate);
            }
            dst.insert(key, rate);
        }
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
    let trimmed = s.trim();
    let bytes = trimmed.as_bytes();
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_dash = false;
    for (i, ch) in trimmed.char_indices() {
        if ch.is_ascii_whitespace() {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        } else if ch == '.'
            && i > 0
            && i + 1 < bytes.len()
            && bytes[i - 1].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
        {
            out.push('-');
            prev_dash = true;
        } else {
            out.push(ch);
            prev_dash = ch == '-';
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
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
    let mut base_norm = norm;
    for eff in EFFORT_SUFFIXES {
        if let Some(stripped) = norm.strip_suffix(eff) {
            if stripped.len() >= 5 && !FUZZY_BLOCKLIST.contains(&stripped) {
                for map in [overrides, rates] {
                    if let Some(&r) = map.get(stripped) {
                        return Some(r);
                    }
                }
                base_norm = stripped;
            }
            break;
        }
    }
    let mut cand = base_norm.to_string();
    for _ in 0..MAX_SUFFIX_STRIP_SEGMENTS {
        let Some((head, _)) = cand.rsplit_once('-') else {
            break;
        };
        if head.len() < 5 || FUZZY_BLOCKLIST.contains(&head) {
            break;
        }
        for map in [overrides, rates] {
            if let Some(&r) = map.get(head) {
                return Some(r);
            }
        }
        cand = head.to_string();
    }
    if base_norm.len() >= 5 && !FUZZY_BLOCKLIST.contains(&base_norm) {
        for map in [overrides, rates] {
            let mut best: Option<(usize, &str, ModelRate)> = None;
            for (k, &r) in map {
                if k.len() >= 5
                    && !FUZZY_BLOCKLIST.contains(&k.as_str())
                    && (base_norm.starts_with(k.as_str()) || k.starts_with(base_norm))
                {
                    let len = k.len().min(base_norm.len());
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

#[cfg(test)]
mod tests {
    use crate::tui::pricing::PricingCatalog;

    #[test]
    fn resolves_swarm_effort_suffixes_and_preserves_dot_version_models() {
        let cat = PricingCatalog::new("/nonexistent");
        for (model, exp_in) in [
            ("claude-opus-5-5-high", 4.0),
            ("gemini-4-argon", 2.0),
            ("gemini-4-argon-high", 2.0),
            ("gemini-4-hydrogen", 2.0),
            ("gemini-4-hydrogen-high", 2.0),
            ("gemini-4-neon", 2.0),
            ("hydrogen-preview", 2.0),
            ("gemini-3.8-flash-low", 1.5),
            ("gpt-oss-120b-medium", 0.15),
            ("gpt-6-astra", 2.5),
            ("gemini-2.5-pro", 1.25),
            ("gpt-4.1", 2.0),
        ] {
            let r = cat.resolve(model);
            assert!(
                (r.input - exp_in).abs() < 1e-6,
                "model={model} got={} expected={exp_in}",
                r.input
            );
        }
        let unk = cat.resolve("unknown-future-model-xhigh");
        assert!(unk.input > 0.0 && unk.output > 0.0);
        let (act, unc) = unk.compute_turn_cost(10_000, 0, 0, 0, 1_000);
        assert!(act > 0.0 && (act - unc).abs() < 1e-9);
    }
}
