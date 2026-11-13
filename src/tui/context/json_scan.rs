//! Zero-Dependency JSON Array/Object Slice Scanners & ISO-8601 Timestamp Parser (`src/tui/context/json_scan.rs`).

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

pub fn find_balanced_span(bytes: &[u8], start: usize, open_b: u8, close_b: u8) -> Option<usize> {
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

pub fn extract_json_array_slice<'a>(src: &'a str, key: &str) -> Option<&'a str> {
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

pub fn iter_array_objects(arr_slice: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = arr_slice.as_bytes();
    let mut i = 1usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let Some(j) = find_balanced_span(bytes, i, b'{', b'}') else {
                break;
            };
            out.push(&arr_slice[i..=j]);
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out
}

pub fn extract_string_list_in_array(arr_slice: &str) -> Vec<String> {
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
        for blk in iter_array_objects(arr) {
            let blk_len = blk.len() as u64;
            let btype = extract_json_str(blk, "type").unwrap_or_default();
            if matches!(
                btype.as_str(),
                "tool_use"
                    | "tool_result"
                    | "tool_reference"
                    | "server_tool_use"
                    | "mcp_tool_use"
                    | "mcp_tool_result"
                    | "mcp_tool_listing"
                    | "web_search_tool_result"
                    | "web_fetch_tool_result"
                    | "advisor_tool_result"
                    | "tool_search_tool_result"
            ) || btype.ends_with("_tool_result")
            {
                w.tool_io += blk_len;
            } else if matches!(btype.as_str(), "thinking" | "redacted_thinking") {
                w.thinking += blk_len;
            } else if btype == "image" {
                w.chat += IMAGE_EQUIV_BYTES;
            } else if blk.contains("<system-reminder>") || btype == "mid_conv_system" {
                w.sys_md += blk_len;
            } else {
                w.chat += blk_len;
            }
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

pub fn parse_iso8601_secs(ts: &str) -> Option<u64> {
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

pub fn extract_cache_creation_pair(scope: &str) -> (u64, u64) {
    let mut cc_t = extract_json_u64(scope, "cache_creation_input_tokens").unwrap_or(0);
    let mut cc_1h = 0u64;
    let mut cc_5m = 0u64;
    if let Some(cc_obj) = extract_json_object(scope, "cache_creation") {
        cc_1h = extract_json_u64(cc_obj, "ephemeral_1h_input_tokens").unwrap_or(0);
        cc_5m = extract_json_u64(cc_obj, "ephemeral_5m_input_tokens").unwrap_or(0);
    }
    if cc_t == 0 && (cc_1h > 0 || cc_5m > 0) {
        cc_t = cc_5m.saturating_add(cc_1h);
    }
    (cc_t, cc_1h)
}

#[cfg(test)]
mod tests {
    use crate::commands::statusline::{compute_context_budget, CompactConfig};
    use crate::session::{compact_model_for_width, short_model_label};
    use crate::tui::context::{
        build_context_panel, build_cost_panel, ContextSnapshot, ModelCostEntry,
    };
    use crate::tui::grid::{format_boxed_row, format_top_border, BoxRow};
    use crate::tui::palette::Palette;
    use crate::tui::width::display_width;

    fn mk_mc(name: &str, fam: &'static str, cost_usd: f64) -> ModelCostEntry {
        ModelCostEntry {
            model_short: name.to_string(),
            family: fam,
            turns: 100,
            input_tokens: 500_000,
            cache_read_tokens: 12_000_000,
            output_tokens: 80_000,
            cost_usd,
            saved_usd: 0.0,
            share_pct: 0,
            tps: Some(68.4),
            ttft_ms: Some(420),
        }
    }

    #[test]
    fn renders_full_bleed_rectangles_border_focus_and_compacts_long_model_names() {
        let pal = Palette::default_palette();
        for (inp, exp) in [
            ("deepseek-v4.1-flash[1M]", "deepseek-v4.1-flash"),
            ("gemini-3.8-flash-tiered[1M]", "gemini-3.8-flash-tiered"),
            (
                "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
                "claude-sonnet-4-5",
            ),
        ] {
            assert_eq!(short_model_label(inp), exp);
        }
        assert_eq!(
            compact_model_for_width("deepseek-v4.1-flash", 15),
            "ds-v4.1-flash"
        );
        assert_eq!(
            compact_model_for_width("gemini-3.8-flash-tiered", 19),
            "gm-3.8-flash-tiered"
        );

        let row = BoxRow {
            text: "   \x1b[2mgemini-4-carbon · cc-ide\x1b[0m".to_string(),
            bg: "\x1b[48;2;20;48;78m".to_string(),
            tag: Some((1, 0)),
        };
        let boxed = format_boxed_row(40, &row, &pal.border);
        assert!(boxed.contains("\x1b[0m\x1b[48;2;20;48;78m           \x1b[0m"));
        let top_foc = format_top_border(40, "SUBAGENTS", "main+2", &pal.cyan, true, &pal);
        assert!(top_foc.contains(" SUBAGENTS ") && !top_foc.contains('◆'));

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
            cost_usd: 122.48,
            saved_usd: 360.12,
            saved_pct: 75,
            burn_usd_per_hr: Some(11.14),
            rl_5h_pct: Some(14),
            rl_7d_pct: Some(31),
            lines_added: 120,
            lines_removed: 35,
            model_costs: vec![
                mk_mc("gemini-4-carbon", "gemini", 122.386),
                mk_mc("deepseek-v4.1-flash", "deepseek", 0.0942),
            ],
        };
        let ctx_p = build_context_panel(&snap, 42, 11, false, true, &pal);
        let (cost_p, _) = build_cost_panel(&snap, 42, 14, 0, 3, true, &pal);
        assert!(ctx_p.focused && cost_p.focused);
        assert!(cost_p
            .rows
            .iter()
            .all(|r| display_width(&r.text) <= 40 && r.bg.is_empty()));
        assert!(cost_p
            .rows
            .iter()
            .any(|r| r.text.contains("deepseek-v4.1-flash")));
    }
}
