//! Model Family Classification, Label Shortening & Width-Aware Compaction (`src/session/model_compact.rs`).

use crate::session::agent::AgentKind;
use crate::tui::width::{char_display_width, display_width, floor_char_boundary};

fn has_openai_o_token(low: &str) -> bool {
    for tok in low.split(|c: char| !c.is_ascii_alphanumeric()) {
        if matches!(tok, "o1" | "o3" | "o4") {
            return true;
        }
    }
    false
}

pub fn classify_model_family(model_str: &str) -> &'static str {
    let low = model_str.to_ascii_lowercase();
    if low.contains("gemini")
        || low.contains("argon")
        || low.contains("hydrogen")
        || low.contains("neon")
        || low.contains("antigravity")
    {
        "gemini"
    } else if low.contains("deepseek") {
        "deepseek"
    } else if low.contains("grok") {
        "grok"
    } else if low.contains("codex")
        || low.contains("gpt")
        || low.contains("openai")
        || has_openai_o_token(&low)
    {
        "openai"
    } else {
        "claude"
    }
}

pub fn classify_model_family_for(agent: AgentKind, model: &str) -> &'static str {
    let t = model.trim();
    if t.is_empty()
        || t.eq_ignore_ascii_case("auto")
        || t.eq_ignore_ascii_case("default")
        || t.eq_ignore_ascii_case("inherit")
    {
        agent.default_family()
    } else {
        classify_model_family(t)
    }
}

/// Strips noisy routing prefixes (`anthropic/`, `us.anthropic.`), context-window tags
/// (`[1M]`), Bedrock revision suffixes (`-v1:0`), and trailing 8-digit snapshot dates
/// (`-20250929`, `@20250929`) WITHOUT chopping semantic model characters like `-flash`.
pub fn short_model_label(raw: &str) -> String {
    let mut s = raw.split('[').next().unwrap_or(raw).trim();
    if let Some((_, rhs)) = s.rsplit_once('/') {
        if !rhs.is_empty() {
            s = rhs;
        }
    }
    for pfx in [
        "us.anthropic.",
        "eu.anthropic.",
        "apac.anthropic.",
        "global.anthropic.",
    ] {
        if let Some(rest) = s.strip_prefix(pfx) {
            s = rest;
            break;
        }
    }
    if let Some((lhs, _)) = s.split_once("-v1:") {
        s = lhs;
    }
    if let Some((lhs, rhs)) = s.rsplit_once(['-', '@']) {
        if rhs.len() == 8 && rhs.bytes().all(|b| b.is_ascii_digit()) && lhs.len() >= 4 {
            s = lhs;
        }
    }
    s.to_string()
}

/// Progressive semantic compactor when a pane is narrower than `max_cols`:
/// 1. Full canonical name (`deepseek-v4.1-flash`) when `width <= max_cols`.
/// 2. Vendor-prefix compaction (`deepseek-` -> `ds-`, `gemini-` -> `gm-`, `claude-` -> ``)
///    since the colored `●` / panel title already encodes the vendor family.
/// 3. Middle-ellipsis (`head…tail`) so distinguishing suffixes (`-flash`, `-tiered`, `-mini`)
///    are never cut off.
pub fn compact_model_for_width(name: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if display_width(name) <= max_cols {
        return name.to_string();
    }
    let low = name.to_ascii_lowercase();
    let compact_cand = if let Some(rest) = name.get(7..).filter(|_| low.starts_with("claude-")) {
        rest.to_string()
    } else if let Some(rest) = name.get(9..).filter(|_| low.starts_with("deepseek-")) {
        format!("ds-{rest}")
    } else if let Some(rest) = name.get(7..).filter(|_| low.starts_with("gemini-")) {
        format!("gm-{rest}")
    } else if let Some(rest) = name.get(7..).filter(|_| low.starts_with("openai-")) {
        rest.to_string()
    } else {
        name.to_string()
    };
    if display_width(&compact_cand) <= max_cols {
        return compact_cand;
    }
    if max_cols <= 4 {
        let end = floor_char_boundary(&compact_cand, max_cols);
        return compact_cand[..end].to_string();
    }
    let avail = max_cols - 1;
    let tail_cols = (avail / 2).max(3);
    let head_cols = avail - tail_cols;
    let mut head_end = 0usize;
    let mut head_acc = 0usize;
    for (idx, ch) in compact_cand.char_indices() {
        let cw = char_display_width(ch);
        if head_acc + cw > head_cols {
            break;
        }
        head_acc += cw;
        head_end = idx + ch.len_utf8();
    }
    let mut tail_start = compact_cand.len();
    let mut acc = 0usize;
    for (idx, ch) in compact_cand.char_indices().rev() {
        let cw = char_display_width(ch);
        if acc + cw > tail_cols {
            break;
        }
        acc += cw;
        tail_start = idx;
    }
    format!(
        "{}…{}",
        &compact_cand[..head_end],
        &compact_cand[tail_start..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_vendor_prefixes_and_measures_non_ascii_tail_width() {
        assert_eq!(
            compact_model_for_width("deepseek-v4.1-flash", 14),
            "ds-v4.1-flash"
        );
        assert_eq!(
            compact_model_for_width("gemini-4-carbon", 11),
            "gm-4-carbon"
        );
        // Width-1 non-ASCII (`é`, `α`) in tail uses `char_display_width` (=1), not 2
        let compacted = compact_model_for_width("model-αβγδεζηθ", 9);
        assert!(display_width(&compacted) <= 9);
        assert!(compacted.contains('…'));
        assert_eq!(classify_model_family("o3-mini"), "openai");
        assert_eq!(classify_model_family("nemo1-turbo3"), "claude");
        assert_eq!(
            classify_model_family("Claude Opus 4.6 (Thinking)"),
            "claude"
        );
        assert_eq!(classify_model_family("gpt-6-astra"), "openai");
        assert_eq!(classify_model_family("gpt-6-codex"), "openai");
        assert_eq!(classify_model_family("hydrogen-preview"), "gemini");
        assert_eq!(classify_model_family_for(AgentKind::Agy, "auto"), "gemini");
        assert_eq!(
            classify_model_family_for(AgentKind::Agy, "Claude Opus 4.6 (Thinking)"),
            "claude"
        );
        assert_eq!(
            classify_model_family_for(AgentKind::Codex, "default"),
            "openai"
        );
        for w in [6usize, 10, 16] {
            let s = compact_model_for_width(&short_model_label("Claude Opus 4.6 (Thinking)"), w);
            assert!(display_width(&s) <= w);
        }
    }
}
