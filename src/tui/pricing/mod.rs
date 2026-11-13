//! Official Multi-Vendor Model Pricing & `tokscale`-Aligned Prompt-Cache Cost Engine.
//!
//! References `junhoyeo/tokscale` (`crates/tokscale-core/src/pricing/`):
//! - Progressive tiered token cost (`tiered_cost`) for `>128k`/`>200k`/`>272k` thresholds,
//!   plus request-wide tier switching for Haiku 5.5 (`>100k`).
//! - Deterministic prefix/suffix model lookup with `FUZZY_BLOCKLIST` protection.
//! - User custom overrides via `~/.config/cc-ide/pricing.json` (`mtime`-cached).

pub mod lookup;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::json::extract_json_object;
use lookup::{lookup_in_maps, normalize_model_key, parse_section_into, strip_jsonc};

const BUILTIN_PRICING_JSONC: &str = include_str!("../../../assets/pricing.json");

#[derive(Clone, Copy, Debug)]
pub struct ModelRate {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
    pub above_tokens: Option<u64>,
    pub request_wide_tier: bool,
    pub long_input: f64,
    pub long_output: f64,
    pub long_cache_write: f64,
    pub long_cache_read: f64,
}

fn progressive_tiered_cost(tokens: u64, base_mtok: f64, above: Option<u64>, tier_mtok: f64) -> f64 {
    let tok_f = tokens as f64;
    if let Some(lim) = above {
        let lim_f = lim as f64;
        if tok_f > lim_f {
            return (lim_f / 1_000_000.0) * base_mtok + ((tok_f - lim_f) / 1_000_000.0) * tier_mtok;
        }
    }
    (tok_f / 1_000_000.0) * base_mtok
}

impl ModelRate {
    #[allow(clippy::too_many_arguments)]
    pub fn compute_turn_cost_full(
        &self,
        in_tok: u64,
        cc_total_tok: u64,
        cc_1h_tok: u64,
        cr_tok: u64,
        out_tok: u64,
        fast_mode: bool,
        us_geo: bool,
        web_search_reqs: u64,
    ) -> (f64, f64) {
        let (mut actual, mut uncached) =
            self.compute_turn_cost(in_tok, cc_total_tok, cc_1h_tok, cr_tok, out_tok);
        if fast_mode {
            // Fast Mode (`/fast`) applies Claude Code's 2x speed multiplier unless overridden
            actual *= 2.0;
            uncached *= 2.0;
        }
        if us_geo {
            // `inference_geo: "us"` applies upstream's 1.1x surcharge (`AD(n)`)
            actual *= 1.1;
            uncached *= 1.1;
        }
        if web_search_reqs > 0 {
            let ws_usd = (web_search_reqs as f64) * 0.01;
            actual += ws_usd;
            uncached += ws_usd;
        }
        (actual, uncached)
    }

    /// Computes `(actual_cost_usd, uncached_baseline_usd)` using `tokscale`'s tiered billing model.
    pub fn compute_turn_cost(
        &self,
        in_tok: u64,
        cc_total_tok: u64,
        cc_1h_tok: u64,
        cr_tok: u64,
        out_tok: u64,
    ) -> (f64, f64) {
        let total_prompt = in_tok.saturating_add(cc_total_tok).saturating_add(cr_tok);
        let c1h = cc_1h_tok.min(cc_total_tok);
        let c5m = cc_total_tok.saturating_sub(c1h);
        let scale_1h = if self.cache_write_5m > 0.0 {
            self.cache_write_1h / self.cache_write_5m
        } else {
            1.0
        };

        // Haiku 5.5's 100k threshold is request-wide; generic >200k/>272k tiers are progressive (`tokscale`)
        if self.request_wide_tier
            && self
                .above_tokens
                .map(|lim| total_prompt > lim)
                .unwrap_or(false)
        {
            let actual = (in_tok as f64 / 1_000_000.0) * self.long_input
                + (c5m as f64 / 1_000_000.0) * self.long_cache_write
                + (c1h as f64 / 1_000_000.0) * (self.long_cache_write * scale_1h)
                + (cr_tok as f64 / 1_000_000.0) * self.long_cache_read
                + (out_tok as f64 / 1_000_000.0) * self.long_output;
            let uncached = (total_prompt as f64 / 1_000_000.0) * self.long_input
                + (out_tok as f64 / 1_000_000.0) * self.long_output;
            return (actual, uncached);
        }

        let in_cost =
            progressive_tiered_cost(in_tok, self.input, self.above_tokens, self.long_input);
        let cw5_cost = progressive_tiered_cost(
            c5m,
            self.cache_write_5m,
            self.above_tokens,
            self.long_cache_write,
        );
        let cw1_cost = progressive_tiered_cost(
            c1h,
            self.cache_write_1h,
            self.above_tokens,
            self.long_cache_write * scale_1h,
        );
        let cr_cost = progressive_tiered_cost(
            cr_tok,
            self.cache_read,
            self.above_tokens,
            self.long_cache_read,
        );
        let out_cost =
            progressive_tiered_cost(out_tok, self.output, self.above_tokens, self.long_output);

        let actual = in_cost + cw5_cost + cw1_cost + cr_cost + out_cost;
        let uncached =
            progressive_tiered_cost(total_prompt, self.input, self.above_tokens, self.long_input)
                + out_cost;

        (actual, uncached)
    }
}

pub struct PricingCatalog {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    rates: HashMap<String, ModelRate>,
    overrides: HashMap<String, ModelRate>,
}

impl PricingCatalog {
    pub fn new(home: &str) -> Self {
        let path = resolve_pricing_path(home);
        let mut cat = Self {
            path,
            last_mtime: None,
            rates: HashMap::new(),
            overrides: HashMap::new(),
        };
        cat.load_from_str(BUILTIN_PRICING_JSONC);
        cat.reload_if_changed(home);
        cat
    }

    fn load_from_str(&mut self, raw: &str) {
        let clean = strip_jsonc(raw);
        if let Some(models_obj) = extract_json_object(&clean, "models") {
            parse_section_into(models_obj, &mut self.rates);
        }
        if let Some(ov_obj) = extract_json_object(&clean, "custom_overrides") {
            parse_section_into(ov_obj, &mut self.overrides);
        }
    }

    pub fn reload_if_changed(&mut self, home: &str) {
        let p = resolve_pricing_path(home);
        let mt = fs::metadata(&p).and_then(|m| m.modified()).ok();
        if p == self.path && mt == self.last_mtime {
            return;
        }
        self.path = p.clone();
        self.last_mtime = mt;
        self.rates.clear();
        self.overrides.clear();
        self.load_from_str(BUILTIN_PRICING_JSONC);
        if let Ok(user_txt) = fs::read_to_string(&p) {
            self.load_from_str(&user_txt);
        }
    }

    pub fn resolve(&self, raw_model: &str) -> ModelRate {
        let norm = normalize_model_key(raw_model);
        if let Some(r) = lookup_in_maps(&norm, &self.overrides, &self.rates) {
            return r;
        }
        let fallback_key = if norm.contains("fable") || norm.contains("mythos") {
            "claude-fable-5-1"
        } else if norm.contains("deepseek") {
            if norm.contains("r1") || norm.contains("reason") {
                "deepseek-reasoner"
            } else if norm.contains("flash") {
                "deepseek-v4.1-flash"
            } else {
                "deepseek-chat"
            }
        } else if norm.contains("gemini") {
            if norm.contains("flash") {
                "gemini-3.8-flash"
            } else {
                "gemini-4-pro"
            }
        } else if norm.contains("grok") {
            "grok-4.5"
        } else if norm.contains("gpt") || norm.starts_with("o3") || norm.starts_with("o4") {
            "gpt-5.4"
        } else if norm.contains("haiku") {
            "claude-haiku-5-5"
        } else if norm.contains("opus") {
            "claude-opus-5-5"
        } else {
            "claude-sonnet-5-5"
        };
        self.rates.get(fallback_key).copied().unwrap_or(ModelRate {
            input: 3.0,
            output: 15.0,
            cache_write_5m: 3.75,
            cache_write_1h: 6.0,
            cache_read: 0.30,
            above_tokens: None,
            request_wide_tier: false,
            long_input: 3.0,
            long_output: 15.0,
            long_cache_write: 3.75,
            long_cache_read: 0.30,
        })
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokscale_progressive_tiers_and_suffix_stripping_work() {
        let cat = PricingCatalog::new("/nonexistent");
        // Strips `[1M]` and `-tiered` via segment stripping to match `gemini-3.8-flash`
        let r_flash = cat.resolve("google/gemini-3.8-flash-tiered[1M]");
        assert!((r_flash.input - 1.5).abs() < 1e-6);

        // Progressive >200k tier on Gemini 4 Carbon: 400k cache_read = 200k @ $0.20 + 200k @ $0.40 = $0.12
        let r_gem = cat.resolve("gemini-4-carbon[1M]");
        let (actual, uncached) = r_gem.compute_turn_cost(10_000, 0, 0, 400_000, 1_000);
        // actual = 10k@$2 ($0.02) + (200k@$0.2 + 200k@$0.4 = $0.12) + 1k@$12 ($0.012) = $0.152
        // uncached = (200k@$2 + 210k@$4 = 0.4 + 0.84 = $1.24) + $0.012 = $1.252
        assert!((actual - 0.152).abs() < 1e-6, "actual={actual}");
        assert!((uncached - 1.252).abs() < 1e-6, "uncached={uncached}");

        // Request-wide >100k tier on Haiku 5.5
        let r_hk = cat.resolve("claude-haiku-5-5");
        let (act_hk, _) = r_hk.compute_turn_cost(150_000, 0, 0, 0, 0);
        assert!((act_hk - 0.075).abs() < 1e-6);
    }
}
