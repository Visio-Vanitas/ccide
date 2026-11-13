//! Per-Session Real-Time Model Family & Short-Label Resolution.

use std::fs;

use crate::json::extract_json_str;
use crate::tui::width::floor_char_boundary;

#[derive(Clone)]
pub struct ConfigModels {
    pub main_family: &'static str,
    pub main_model_short: String,
    pub sub_family: &'static str,
    pub sub_model_short: String,
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

pub fn short_model_label(raw: &str) -> String {
    let base = raw.split('[').next().unwrap_or(raw).trim();
    if base.len() <= 18 {
        base.to_string()
    } else {
        base[..floor_char_boundary(base, 18)].to_string()
    }
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
/// - **Main Model** for this specific session (`live_sess_<session_id>.json`, `transcript_model`
///   from `<session_id>.jsonl`, or `~/.claude/settings.json` `"model"`).
/// - **Subagent Model** strictly from `~/.claude/settings.json` (`CLAUDE_CODE_SUBAGENT_MODEL`).
pub fn load_config_models(
    home: &str,
    session_id: &str,
    tab_yazi_id: &str,
    transcript_model: &str,
) -> ConfigModels {
    let settings_path = format!("{}/.claude/settings.json", home);
    let content = fs::read_to_string(&settings_path).unwrap_or_default();
    let cache_dir = format!("{}/.cache/cc-ide", home);

    let mut session_raw_model = None;
    let mut live_mtime = std::time::UNIX_EPOCH;
    if !session_id.is_empty() {
        let spath = format!("{}/live_sess_{}.json", cache_dir, session_id);
        if let Ok(meta) = fs::metadata(&spath) {
            if let Ok(mt) = meta.modified() {
                live_mtime = mt;
            }
        }
        if let Ok(stxt) = fs::read_to_string(&spath) {
            session_raw_model = extract_json_str(&stxt, "model").filter(|m| !m.is_empty());
        }
    }

    if !transcript_model.is_empty() {
        let mut jsonl_newer = session_raw_model.is_none();
        if !jsonl_newer && !session_id.is_empty() {
            let projects_dir = format!("{}/.claude/projects", home);
            if let Ok(rd) = fs::read_dir(&projects_dir) {
                for entry in rd.flatten() {
                    let jp = entry.path().join(format!("{}.jsonl", session_id));
                    if let Ok(meta) = fs::metadata(&jp) {
                        if let Ok(j_mt) = meta.modified() {
                            if j_mt > live_mtime {
                                jsonl_newer = true;
                            }
                            break;
                        }
                    }
                }
            }
        }
        if jsonl_newer {
            session_raw_model = Some(transcript_model.to_string());
        }
    }

    if session_raw_model.is_none() && !tab_yazi_id.is_empty() {
        let tpath = format!("{}/live_tab_{}.json", cache_dir, tab_yazi_id);
        if let Ok(ttxt) = fs::read_to_string(&tpath) {
            let file_sid = extract_json_str(&ttxt, "session_id").unwrap_or_default();
            if session_id.is_empty() || file_sid == session_id {
                session_raw_model = extract_json_str(&ttxt, "model").filter(|m| !m.is_empty());
            }
        }
    }

    let raw_main = session_raw_model
        .or_else(|| extract_json_str(&content, "model"))
        .unwrap_or_else(|| "fable".to_string());
    let resolved_main = map_alias_with_settings(&raw_main, &content);

    let raw_sub = extract_json_str(&content, "CLAUDE_CODE_SUBAGENT_MODEL")
        .or_else(|| extract_json_str(&content, "subagentModel"))
        .unwrap_or_else(|| resolved_main.clone());
    let resolved_sub = map_alias_with_settings(&raw_sub, &content);

    ConfigModels {
        main_family: classify_model_family(&resolved_main),
        main_model_short: short_model_label(&resolved_main),
        sub_family: classify_model_family(&resolved_sub),
        sub_model_short: short_model_label(&resolved_sub),
    }
}
