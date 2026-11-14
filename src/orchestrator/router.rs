//! `PreToolUse` Router for `Agent` & Native `Workflow` (`src/orchestrator/router.rs`).

use std::fs;
use std::path::Path;

use crate::json::{extract_json_bool, extract_json_object, extract_json_str};
use crate::orchestrator::workflow_router::route_workflow;
use crate::orchestrator::{
    gp_shim_name, ro_shim_name, sync_agent_shims, OrchestratorConfig, RoleKind, RoleSlot,
    EFFORT_TIERS,
};
use crate::platform::{ipc_dir, ipc_path};
use crate::session::is_valid_model_name;

pub(crate) fn json_esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn normalize_effort(raw: &str) -> Option<String> {
    let low = raw.trim().to_ascii_lowercase();
    if low == "med" {
        return Some("medium".into());
    }
    EFFORT_TIERS.iter().find(|&&e| e == low).map(|&e| e.into())
}

fn fuzzy_match_model(raw: &str, pool: &[String]) -> Option<String> {
    let q = raw.trim();
    if q.is_empty() {
        return None;
    }
    if let Some(m) = pool.iter().find(|m| m.eq_ignore_ascii_case(q)) {
        return Some(m.clone());
    }
    let ql = q.to_ascii_lowercase();
    let exp = match ql.as_str() {
        "ds" => "deepseek",
        "gem" => "gemini",
        _ => ql.as_str(),
    };
    pool.iter()
        .find(|m| m.to_ascii_lowercase().contains(exp))
        .cloned()
        .or_else(|| is_valid_model_name(q).then(|| q.to_string()))
}

/// Parses leading `[@model:effort]`, `[@model]`, or `[@:effort]` from `desc`.
pub fn parse_inline_tag(desc: &str, pool: &[String]) -> (Option<String>, Option<String>, String) {
    let Some((body, tail)) = desc
        .trim_start()
        .strip_prefix("[@")
        .and_then(|r| r.split_once(']'))
    else {
        return (None, None, desc.to_string());
    };
    let (mr, er) = body
        .trim()
        .split_once(':')
        .map_or((body.trim(), ""), |(m, e)| (m.trim(), e.trim()));
    let model = (!mr.is_empty())
        .then(|| fuzzy_match_model(mr, pool))
        .flatten();
    let effort = (!er.is_empty()).then(|| normalize_effort(er)).flatten();
    (model, effort, tail.trim_start().to_string())
}

pub fn classify_agent_role(subagent_type: &str, desc: &str, prompt: &str) -> RoleKind {
    if subagent_type.eq_ignore_ascii_case("Plan") {
        return RoleKind::Plan;
    }
    if subagent_type.eq_ignore_ascii_case("Explore") {
        return RoleKind::Scout;
    }
    let combined = format!("{desc} {prompt}");
    let low = combined.to_ascii_lowercase();
    let has = |kws: &[&str]| kws.iter().any(|k| low.contains(k) || combined.contains(k));
    if has(&["plan", "architect", "design", "方案", "设计", "架构"]) {
        RoleKind::Plan
    } else if has(&[
        "explore", "search", "find", "scan", "locate", "docs", "查找", "检索", "清查",
    ]) {
        RoleKind::Scout
    } else if has(&[
        "review", "audit", "verify", "bug", "security", "审查", "自查", "验证",
    ]) {
        RoleKind::Reviewer
    } else {
        RoleKind::Coder
    }
}

pub fn pick_slot_model(slot: &RoleSlot, session_id: &str, role: RoleKind) -> String {
    if slot.models.len() <= 1 {
        return slot.primary_model().to_string();
    }
    let sid = if session_id.is_empty() {
        "default"
    } else {
        session_id
    };
    let path = ipc_path(&format!("orch_rr_{}_{}.txt", sid, role.key()));
    let idx = fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let _ = fs::create_dir_all(ipc_dir());
    let tmp = ipc_path(&format!(
        "orch_rr_{}_{}.tmp.{}",
        sid,
        role.key(),
        std::process::id()
    ));
    let next_val = ((idx + 1) % slot.models.len()).to_string();
    if fs::write(&tmp, &next_val).is_ok() && fs::rename(&tmp, &path).is_err() {
        let _ = fs::write(&path, &next_val);
        let _ = fs::remove_file(&tmp);
    }
    slot.models[idx % slot.models.len()].clone()
}

pub(crate) fn push_opt_str(fields: &mut Vec<String>, key: &str, val: Option<&str>) {
    if let Some(v) = val {
        fields.push(format!("\"{}\":\"{}\"", key, json_esc(v)));
    }
}

fn route_agent(tool_in: &str, session_id: &str, home: &str) -> Option<String> {
    let sub_type =
        extract_json_str(tool_in, "subagent_type").unwrap_or_else(|| "general-purpose".into());
    if sub_type == "fork" {
        return None;
    }
    let desc = extract_json_str(tool_in, "description").unwrap_or_default();
    let prompt = extract_json_str(tool_in, "prompt").unwrap_or_default();
    let cfg = OrchestratorConfig::load(home);

    let (mut tag_m, mut tag_e, clean_desc) = parse_inline_tag(&desc, &cfg.model_pool);
    if tag_m.is_none() && tag_e.is_none() {
        let (pm, pe, _) = parse_inline_tag(&prompt, &cfg.model_pool);
        tag_m = pm;
        tag_e = pe;
    }
    let role = classify_agent_role(&sub_type, &clean_desc, &prompt);
    let slot = cfg.active_profile().slot(role);
    let target_model = tag_m.unwrap_or_else(|| pick_slot_model(slot, session_id, role));
    let target_effort = tag_e.unwrap_or_else(|| slot.effort.clone());
    if target_model.eq_ignore_ascii_case("inherit")
        && target_effort.eq_ignore_ascii_case("inherit")
        && clean_desc == desc
    {
        return None;
    }

    let is_gp = sub_type.is_empty() || sub_type == "general-purpose";
    let is_ro = matches!(sub_type.as_str(), "Explore" | "Plan");
    let is_existing_shim = sub_type.starts_with("ccide-gp--") || sub_type.starts_with("ccide-ro--");
    let use_shim = !target_model.eq_ignore_ascii_case("inherit") && (is_gp || is_ro);
    let final_sub_type = if use_shim {
        let shim = if is_ro {
            ro_shim_name(&target_model)
        } else {
            gp_shim_name(&target_model)
        };
        if !Path::new(home)
            .join(".claude")
            .join("agents")
            .join(format!("{}.md", shim))
            .is_file()
        {
            let mut pool = cfg.model_pool.clone();
            if !pool.iter().any(|m| m.eq_ignore_ascii_case(&target_model)) {
                pool.push(target_model.clone());
            }
            sync_agent_shims(home, &pool);
        }
        shim
    } else {
        sub_type.clone()
    };

    let mut fields = vec![format!(
        "\"subagent_type\":\"{}\"",
        json_esc(&final_sub_type)
    )];
    if !use_shim && !is_existing_shim {
        let m_cand = if matches!(target_model.as_str(), "sonnet" | "opus" | "haiku" | "fable") {
            Some(target_model.clone())
        } else {
            extract_json_str(tool_in, "model")
        };
        push_opt_str(&mut fields, "model", m_cand.as_deref());
    }
    if matches!(
        target_effort.as_str(),
        "low" | "medium" | "high" | "xhigh" | "max"
    ) {
        fields.push(format!("\"effort\":\"{}\"", target_effort));
    }
    if !desc.is_empty() || clean_desc != desc {
        fields.push(format!("\"description\":\"{}\"", json_esc(&clean_desc)));
    }
    if !prompt.is_empty() {
        fields.push(format!("\"prompt\":\"{}\"", json_esc(&prompt)));
    }
    for k in ["isolation", "name", "cwd"] {
        push_opt_str(&mut fields, k, extract_json_str(tool_in, k).as_deref());
    }
    if let Some(bg) = extract_json_bool(tool_in, "run_in_background") {
        fields.push(format!("\"run_in_background\":{}", bg));
    }
    Some(format!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"allow\",\"updatedInput\":{{{}}}}}}}",
        fields.join(",")
    ))
}

pub fn route_pre_tool_use(stdin_json: &str, home: &str) -> Option<String> {
    let tool_name = extract_json_str(stdin_json, "tool_name")?;
    let session_id = extract_json_str(stdin_json, "session_id").unwrap_or_default();
    let perm_mode = extract_json_str(stdin_json, "permission_mode").unwrap_or_default();
    let tool_in = extract_json_object(stdin_json, "tool_input")?;
    match tool_name.as_str() {
        "Agent" => route_agent(tool_in, &session_id, home),
        "Workflow" => route_workflow(tool_in, &perm_mode, home),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn parses_inline_tags_and_fuzzy_pool() {
        let pool = vec![
            "gemini-4-hydrogen[1M]".into(),
            "deepseek-v4.1-flash[1M]".into(),
            "kimi-k3".into(),
        ];
        let chk = |inp, em: Option<&str>, ee: Option<&str>, ed: &str| {
            assert_eq!(
                parse_inline_tag(inp, &pool),
                (em.map(String::from), ee.map(String::from), ed.to_string())
            );
        };
        let (hy, ds) = ("gemini-4-hydrogen[1M]", "deepseek-v4.1-flash[1M]");
        chk(
            "[@kimi-k3:high] Audit",
            Some("kimi-k3"),
            Some("high"),
            "Audit",
        );
        chk("[@hydrogen:low] Scan", Some(hy), Some("low"), "Scan");
        chk("[@ds:med] Search", Some(ds), Some("medium"), "Search");
        chk("[@:xhigh] Deep", None, Some("xhigh"), "Deep");
    }

    #[test]
    fn rewrites_agent_skips_fork_rotates_and_expands_workflow() {
        let tmp = env::temp_dir().join(format!("ccide_rt_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        let h = tmp.to_string_lossy().into_owned();
        let fork_in = r#"{"tool_name":"Agent","session_id":"s1","tool_input":{"subagent_type":"fork","description":"x","prompt":"y"}}"#;
        assert!(route_pre_tool_use(fork_in, &h).is_none());

        let ex_in = r#"{"tool_name":"Agent","session_id":"s1","tool_input":{"subagent_type":"Explore","description":"[@kimi-k3:high] Find files","prompt":"scan"}}"#;
        let out = route_pre_tool_use(ex_in, &h).expect("routed");
        assert!(
            out.contains("\"subagent_type\":\"ccide-ro--kimi-k3\"")
                && out.contains("\"effort\":\"high\"")
                && !out.contains("\"model\":")
        );

        let slot = RoleSlot {
            models: vec!["m1".into(), "m2".into()],
            effort: "low".into(),
        };
        assert_eq!(pick_slot_model(&slot, "rr_t", RoleKind::Coder), "m1");
        assert_eq!(pick_slot_model(&slot, "rr_t", RoleKind::Coder), "m2");
        let _ = fs::remove_dir_all(&tmp);
    }
}
