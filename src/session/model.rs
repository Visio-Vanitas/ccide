//! Per-Session Real-Time Model Family, Validation & Multi-Source Timestamp Resolution.

use std::fs;
use std::time::SystemTime;

use crate::json::extract_json_str;
use crate::tui::width::{display_width, floor_char_boundary};

#[derive(Clone)]
pub struct ConfigModels {
    pub main_family: &'static str,
    pub main_model_short: String,
    pub sub_family: &'static str,
    pub sub_model_short: String,
}

/// Rejects empty strings, angle-bracket placeholders (`<synthetic>`), and serialized
/// JS/JSON primitives (`false`, `true`, `null`, `undefined`, `none`, `nan`, `synthetic`).
pub fn is_valid_model_name(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() || s.starts_with('<') || s.ends_with('>') {
        return false;
    }
    !matches!(
        s.to_ascii_lowercase().as_str(),
        "false" | "true" | "null" | "undefined" | "none" | "nan" | "synthetic"
    )
}

pub fn classify_model_family(model_str: &str) -> &'static str {
    let low = model_str.to_ascii_lowercase();
    if low.contains("gemini") || low.contains("argon") {
        "gemini"
    } else if low.contains("deepseek") {
        "deepseek"
    } else if low.contains("gpt")
        || low.contains("o1")
        || low.contains("o3")
        || low.contains("o4")
        || low.contains("openai")
    {
        "openai"
    } else if low.contains("grok") {
        "grok"
    } else {
        "claude"
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
    let head_end = floor_char_boundary(&compact_cand, head_cols);
    let mut tail_start = compact_cand.len();
    let mut acc = 0usize;
    for (idx, ch) in compact_cand.char_indices().rev() {
        let cw = if ch.is_ascii() { 1 } else { 2 };
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

pub fn map_alias_with_settings(raw_model: &str, settings_content: &str) -> String {
    let m_low = raw_model.trim().to_ascii_lowercase();
    if m_low.contains("fable") {
        extract_json_str(settings_content, "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME")
            .or_else(|| extract_json_str(settings_content, "ANTHROPIC_DEFAULT_FABLE_MODEL"))
            .unwrap_or_else(|| raw_model.to_string())
    } else if m_low.starts_with("opus") || m_low == "claude-opus" {
        extract_json_str(settings_content, "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME")
            .or_else(|| extract_json_str(settings_content, "ANTHROPIC_DEFAULT_OPUS_MODEL"))
            .unwrap_or_else(|| raw_model.to_string())
    } else if m_low.starts_with("sonnet") || m_low == "claude-sonnet" {
        extract_json_str(settings_content, "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME")
            .or_else(|| extract_json_str(settings_content, "ANTHROPIC_DEFAULT_SONNET_MODEL"))
            .unwrap_or_else(|| raw_model.to_string())
    } else if m_low.starts_with("haiku") || m_low == "claude-haiku" {
        extract_json_str(settings_content, "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME")
            .or_else(|| extract_json_str(settings_content, "ANTHROPIC_DEFAULT_HAIKU_MODEL"))
            .unwrap_or_else(|| raw_model.to_string())
    } else {
        raw_model.to_string()
    }
}

/// Resolves:
/// - **Main Model** for this specific session by comparing genuine `SystemTime` (`mtime`) across
///   `live_sess_<session_id>.json`, `<session_id>.jsonl`, and `cur_pid_<pid>.txt` independently,
///   filtering every candidate through `is_valid_model_name()`.
/// - **Subagent Model** strictly from `~/.claude/settings.json` (`CLAUDE_CODE_SUBAGENT_MODEL`).
pub fn load_config_models(
    home: &str,
    session_id: &str,
    tab_yazi_id: &str,
    transcript_cand: Option<(SystemTime, &str)>,
    pushed_cand: Option<(SystemTime, &str)>,
) -> ConfigModels {
    let settings_path = format!("{}/.claude/settings.json", home);
    let content = fs::read_to_string(&settings_path).unwrap_or_default();
    let cache_dir = format!("{}/.cache/cc-ide", home);

    let mut candidates: Vec<(SystemTime, String)> = Vec::with_capacity(3);

    if !session_id.is_empty() {
        let spath = format!("{}/live_sess_{}.json", cache_dir, session_id);
        if let (Ok(meta), Ok(stxt)) = (fs::metadata(&spath), fs::read_to_string(&spath)) {
            if let Some(m) = extract_json_str(&stxt, "model").filter(|m| is_valid_model_name(m)) {
                let mt = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                candidates.push((mt, m));
            }
        }
        let mcache = format!("{}/status/model_{}.cache", cache_dir, session_id);
        if let (Ok(meta), Ok(mtxt)) = (fs::metadata(&mcache), fs::read_to_string(&mcache)) {
            let trimmed = mtxt.trim();
            if is_valid_model_name(trimmed) {
                let mt = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                candidates.push((mt, trimmed.to_string()));
            }
        }
    }

    if let Some((j_mt, tm)) = transcript_cand {
        if is_valid_model_name(tm) {
            let eff_mt = if j_mt > SystemTime::UNIX_EPOCH {
                j_mt
            } else {
                lookup_jsonl_mtime(home, session_id)
            };
            candidates.push((eff_mt, tm.trim().to_string()));
        }
    }

    if let Some((p_mt, pm)) = pushed_cand {
        if is_valid_model_name(pm) {
            candidates.push((p_mt, pm.trim().to_string()));
        }
    }

    let mut session_raw_model = candidates
        .into_iter()
        .max_by_key(|(mt, _)| *mt)
        .map(|(_, m)| m);

    if session_raw_model.is_none() && !tab_yazi_id.is_empty() {
        let tpath = format!("{}/live_tab_{}.json", cache_dir, tab_yazi_id);
        if let Ok(ttxt) = fs::read_to_string(&tpath) {
            let file_sid = extract_json_str(&ttxt, "session_id").unwrap_or_default();
            if session_id.is_empty() || file_sid == session_id {
                session_raw_model =
                    extract_json_str(&ttxt, "model").filter(|m| is_valid_model_name(m));
            }
        }
    }

    let raw_main = session_raw_model
        .or_else(|| extract_json_str(&content, "model").filter(|m| is_valid_model_name(m)))
        .unwrap_or_else(|| "fable".to_string());
    let resolved_main = map_alias_with_settings(&raw_main, &content);

    let raw_sub = extract_json_str(&content, "CLAUDE_CODE_SUBAGENT_MODEL")
        .filter(|m| is_valid_model_name(m))
        .or_else(|| extract_json_str(&content, "subagentModel").filter(|m| is_valid_model_name(m)))
        .unwrap_or_else(|| resolved_main.clone());
    let resolved_sub = map_alias_with_settings(&raw_sub, &content);

    ConfigModels {
        main_family: classify_model_family(&resolved_main),
        main_model_short: short_model_label(&resolved_main),
        sub_family: classify_model_family(&resolved_sub),
        sub_model_short: short_model_label(&resolved_sub),
    }
}

fn lookup_jsonl_mtime(home: &str, session_id: &str) -> SystemTime {
    if session_id.is_empty() {
        return SystemTime::UNIX_EPOCH;
    }
    if let Ok(rd) = fs::read_dir(format!("{}/.claude/projects", home)) {
        for entry in rd.flatten() {
            let jp = entry.path().join(format!("{}.jsonl", session_id));
            if let Ok(mt) = fs::metadata(&jp).and_then(|m| m.modified()) {
                return mt;
            }
        }
    }
    SystemTime::UNIX_EPOCH
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rejects_false_synthetic_and_picks_newest_valid_source() {
        assert!(!is_valid_model_name(""));
        assert!(!is_valid_model_name("false"));
        assert!(!is_valid_model_name("FALSE"));
        assert!(!is_valid_model_name("null"));
        assert!(!is_valid_model_name("undefined"));
        assert!(!is_valid_model_name("<synthetic>"));
        assert!(!is_valid_model_name("synthetic"));
        assert!(is_valid_model_name("gemini-4-carbon[1M]"));
        assert!(is_valid_model_name("deepseek-v4.1-flash[1M]"));

        let tmp_home = std::env::temp_dir().join(format!("ccide_mtest_{}", std::process::id()));
        let _ = fs::create_dir_all(tmp_home.join(".cache/cc-ide"));
        let _ = fs::create_dir_all(tmp_home.join(".claude"));
        fs::write(
            tmp_home.join(".claude/settings.json"),
            r#"{"model":"claude-fable-5-1","CLAUDE_CODE_SUBAGENT_MODEL":"deepseek-v4.1"}"#,
        )
        .unwrap();
        fs::write(
            tmp_home.join(".cache/cc-ide/live_sess_s1.json"),
            r#"{"model":"gemini-4-carbon[1M]"}"#,
        )
        .unwrap();
        let live_mt = fs::metadata(tmp_home.join(".cache/cc-ide/live_sess_s1.json"))
            .unwrap()
            .modified()
            .unwrap();

        // Even if `pushed_cand` has a newer timestamp, `"false"` is rejected so `gemini` stays stable!
        let cfg = load_config_models(
            tmp_home.to_str().unwrap(),
            "s1",
            "",
            Some((live_mt + Duration::from_secs(5), "<synthetic>")),
            Some((live_mt + Duration::from_secs(10), "false")),
        );
        assert_eq!(cfg.main_family, "gemini");
        assert_eq!(cfg.main_model_short, "gemini-4-carbon");
        assert_eq!(cfg.sub_family, "deepseek");

        // A genuinely newer valid `pushed_cand` wins cleanly over older `live_sess`
        let cfg2 = load_config_models(
            tmp_home.to_str().unwrap(),
            "s1",
            "",
            Some((live_mt - Duration::from_secs(5), "gpt-5")),
            Some((live_mt + Duration::from_secs(10), "grok-4[1M]")),
        );
        assert_eq!(cfg2.main_family, "grok");
        assert_eq!(cfg2.main_model_short, "grok-4");

        let _ = fs::remove_dir_all(&tmp_home);
    }
}
