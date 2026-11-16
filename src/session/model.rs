//! Per-Session Real-Time Model Resolution & Settings Alias Mapping (`src/session/model.rs`).

use std::fs;
use std::time::SystemTime;

use crate::json::extract_json_str;
use crate::platform::ipc_path;

pub use crate::model_id::is_valid_model_name;
pub use crate::session::model_compact::{
    classify_model_family, compact_model_for_width, short_model_label,
};

#[derive(Clone)]
pub struct ConfigModels {
    pub main_family: &'static str,
    pub main_model_raw: String,
    pub main_model_short: String,
    pub sub_family: &'static str,
    pub sub_model_short: String,
}

fn lookup_setting_or_env(settings_content: &str, k1: &str, k2: &str) -> Option<String> {
    extract_json_str(settings_content, k1)
        .filter(|m| is_valid_model_name(m))
        .or_else(|| extract_json_str(settings_content, k2).filter(|m| is_valid_model_name(m)))
        .or_else(|| std::env::var(k1).ok().filter(|m| is_valid_model_name(m)))
        .or_else(|| std::env::var(k2).ok().filter(|m| is_valid_model_name(m)))
}

pub fn map_alias_with_settings(raw_model: &str, settings_content: &str) -> String {
    let m_low = raw_model.trim().to_ascii_lowercase();
    let suffix_1m = m_low.contains("[1m]");
    let base = if m_low.contains("fable") {
        lookup_setting_or_env(
            settings_content,
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
        )
    } else if m_low.contains("opus") {
        lookup_setting_or_env(
            settings_content,
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
        )
    } else if m_low.contains("sonnet") {
        lookup_setting_or_env(
            settings_content,
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
        )
    } else if m_low.contains("haiku") {
        lookup_setting_or_env(
            settings_content,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        )
    } else {
        None
    };
    let mut resolved = base.unwrap_or_else(|| raw_model.to_string());
    if suffix_1m && !resolved.contains('[') {
        resolved.push_str("[1M]");
    }
    resolved
}

fn push_trimmed_model_file(path: &std::path::Path, out: &mut Vec<(SystemTime, String)>) {
    if let (Ok(meta), Ok(txt)) = (fs::metadata(path), fs::read_to_string(path)) {
        let trimmed = txt.trim();
        if is_valid_model_name(trimmed) {
            out.push((
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                trimmed.to_string(),
            ));
        }
    }
}

/// Resolves:
/// - **Main Model** for this specific session by comparing genuine `SystemTime` (`mtime`) across
///   live runtime state (`mod_model_<sid>.txt`, `status/model_<sid>.cache`, `live_sess_<sid>.json`)
///   and explicit `.jsonl` model switches, filtering through `is_valid_model_name()`.
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

    let mut candidates: Vec<(SystemTime, String)> = Vec::with_capacity(5);

    if !session_id.is_empty() {
        let spath = format!("{}/live_sess_{}.json", cache_dir, session_id);
        if let (Ok(meta), Ok(stxt)) = (fs::metadata(&spath), fs::read_to_string(&spath)) {
            let mid = extract_json_str(&stxt, "model_id").filter(|m| is_valid_model_name(m));
            let mdisp = extract_json_str(&stxt, "model").filter(|m| is_valid_model_name(m));
            let chosen = match (mid, mdisp) {
                (Some(id), Some(disp)) => {
                    let mapped_id = map_alias_with_settings(&id, &content);
                    let id_still_claude_alias =
                        classify_model_family(&mapped_id) == "claude" && mapped_id == id;
                    let mut picked =
                        if id_still_claude_alias && classify_model_family(&disp) != "claude" {
                            disp.clone()
                        } else {
                            id
                        };
                    if disp.to_ascii_lowercase().contains("[1m]") && !picked.contains('[') {
                        picked.push_str("[1M]");
                    }
                    Some(picked)
                }
                (Some(id), None) => Some(id),
                (None, Some(disp)) => Some(disp),
                (None, None) => None,
            };
            if let Some(m) = chosen {
                candidates.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), m));
            }
        }
        push_trimmed_model_file(
            std::path::Path::new(&format!("{}/status/model_{}.cache", cache_dir, session_id)),
            &mut candidates,
        );
        push_trimmed_model_file(
            &ipc_path(&format!("mod_model_{session_id}.txt")),
            &mut candidates,
        );
    }

    if let Some((j_mt, tm)) = transcript_cand {
        if is_valid_model_name(tm) {
            // Only let `.jsonl` compete by timestamp when `j_mt` was explicitly stamped (e.g.
            // `"Set model to ..."`), or as a baseline fallback (`UNIX_EPOCH`) when no live state
            // file exists yet — preventing `PostModelSwitch` `hook_success` `.jsonl` writes from
            // reviving a previous turn's assistant model.
            candidates.push((j_mt, tm.trim().to_string()));
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

    let main_family = classify_model_family(&resolved_main);
    let main_model_short = short_model_label(&resolved_main);
    ConfigModels {
        main_family,
        main_model_raw: resolved_main,
        main_model_short,
        sub_family: classify_model_family(&resolved_sub),
        sub_model_short: short_model_label(&resolved_sub),
    }
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

        // Full `claude-opus-5-5` / `claude-sonnet-5-5` IDs map through `ANTHROPIC_DEFAULT_*`
        let st_aliases = r#"{"ANTHROPIC_DEFAULT_OPUS_MODEL_NAME":"gemini-4-hydrogen","ANTHROPIC_DEFAULT_SONNET_MODEL_NAME":"deepseek-v4.1-flash","ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME":"gemini-4-hydrogen"}"#;
        assert_eq!(
            map_alias_with_settings("claude-opus-5-5", st_aliases),
            "gemini-4-hydrogen"
        );
        assert_eq!(
            map_alias_with_settings("claude-sonnet-5-5", st_aliases),
            "deepseek-v4.1-flash"
        );
        assert_eq!(
            map_alias_with_settings("Claude Haiku 5.5", st_aliases),
            "gemini-4-hydrogen"
        );
        assert_eq!(classify_model_family("o3-mini"), "openai");
        assert_eq!(classify_model_family("nemo1-turbo3"), "claude");

        let _ = fs::remove_dir_all(&tmp_home);
    }
}
