//! Native `Workflow` `@role` Macro Expansion & `CCIDE_WORKFLOW_DISPATCH_JSON` Router.

use std::env;
use std::fs;

use crate::json::{extract_json_object, extract_json_str};
use crate::orchestrator::router::{json_esc, push_opt_str};
use crate::orchestrator::{OrchestratorConfig, ALL_ROLES};
use crate::tui::context::json_scan::extract_json_array_slice;

fn js_str_esc(s: &str, quote: char) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

pub fn expand_workflow_macros(script: &str, cfg: &OrchestratorConfig) -> String {
    let prof = cfg.active_profile();
    let mut out = script.to_string();
    for role in ALL_ROLES {
        let slot = prof.slot(role);
        let tag = format!("@{}", role.key());
        let model = slot.primary_model();
        let eff = if slot.effort == "inherit" {
            "medium"
        } else {
            &slot.effort
        };
        for q in ['"', '\''] {
            let pat = format!("{}{}{}", q, tag, q);
            if !out.contains(&pat) {
                continue;
            }
            let (mut next, mut rest) = (String::with_capacity(out.len()), out.as_str());
            while let Some(pos) = rest.find(&pat) {
                next.push_str(&rest[..pos]);
                let b = next.trim_end();
                let is_effort = b
                    .strip_suffix(':')
                    .map(|s| s.trim_end())
                    .map(|s| {
                        s.ends_with("effort")
                            || s.ends_with("\"effort\"")
                            || s.ends_with("'effort'")
                    })
                    .unwrap_or(false);
                let rep = if is_effort { eff } else { model };
                next.push(q);
                next.push_str(&js_str_esc(rep, q));
                next.push(q);
                rest = &rest[pos + pat.len()..];
            }
            next.push_str(rest);
            out = next;
        }
    }
    out
}

fn build_ccide_meta(cfg: &OrchestratorConfig) -> String {
    let prof = cfg.active_profile();
    let roles: Vec<String> = ALL_ROLES
        .iter()
        .map(|&r| {
            let s = prof.slot(r);
            let pool: Vec<String> = s
                .models
                .iter()
                .map(|m| format!("\"{}\"", json_esc(m)))
                .collect();
            format!(
                "\"{}\":{{\"model\":\"{}\",\"effort\":\"{}\",\"pool\":[{}]}}",
                r.key(),
                json_esc(s.primary_model()),
                json_esc(&s.effort),
                pool.join(",")
            )
        })
        .collect();
    format!(
        "{{\"preset\":\"{}\",\"roles\":{{{}}}}}",
        json_esc(&prof.name),
        roles.join(",")
    )
}

pub(crate) fn route_workflow(tool_in: &str, perm_mode: &str, home: &str) -> Option<String> {
    if let Some(trimmed) = env::var("CCIDE_WORKFLOW_DISPATCH_JSON")
        .ok()
        .and_then(|p| fs::read_to_string(p.trim()).ok())
    {
        let t = trimmed.trim();
        if t.starts_with('{') {
            return Some(format!(
                "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"allow\",\"updatedInput\":{}}}}}",
                t
            ));
        }
    }
    let cfg = OrchestratorConfig::load(home);
    let mut fields = Vec::new();
    if let Some(script) = extract_json_str(tool_in, "script") {
        fields.push(format!(
            "\"script\":\"{}\"",
            json_esc(&expand_workflow_macros(&script, &cfg))
        ));
    }
    for k in ["scriptPath", "resumeFromRunId"] {
        push_opt_str(&mut fields, k, extract_json_str(tool_in, k).as_deref());
    }
    let meta = build_ccide_meta(&cfg);
    if let Some(args_obj) = extract_json_object(tool_in, "args") {
        let inner = args_obj.trim();
        let is_empty_obj = inner
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .map(|s| s.trim().is_empty())
            .unwrap_or(false);
        let injected = if is_empty_obj {
            format!("{{\"__ccide\":{}}}", meta)
        } else if let Some(prefix) = inner.strip_suffix('}') {
            format!("{},\"__ccide\":{}}}", prefix, meta)
        } else {
            inner.to_string()
        };
        fields.push(format!("\"args\":{}", injected));
    } else if let Some(arr) = extract_json_array_slice(tool_in, "args") {
        fields.push(format!("\"args\":{}", arr));
    } else if extract_json_str(tool_in, "args").is_none() {
        fields.push(format!("\"args\":{{\"__ccide\":{}}}", meta));
    }
    let decision = if perm_mode.contains("bypass") || perm_mode.contains("auto") {
        "allow"
    } else {
        "ask"
    };
    Some(format!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"{}\",\"updatedInput\":{{{}}}}}}}",
        decision,
        fields.join(",")
    ))
}

#[cfg(test)]
mod tests {
    use crate::orchestrator::router::route_pre_tool_use;

    #[test]
    fn expands_workflow_role_macros_and_injects_ccide_args() {
        let wf_in = r#"{"tool_name":"Workflow","session_id":"s1","permission_mode":"bypassPermissions","tool_input":{"script":"agent({model: '@scout', effort: '@scout'})","args":{"x":1}}}"#;
        let wf_out = route_pre_tool_use(wf_in, "").expect("wf");
        assert!(
            wf_out.contains("\"permissionDecision\":\"allow\"")
                && wf_out.contains("__ccide")
                && !wf_out.contains("'@scout'")
        );
    }
}
