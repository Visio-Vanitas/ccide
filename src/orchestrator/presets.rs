//! Default Model Pool Discovery & Built-In Orchestration Presets (`src/orchestrator/presets.rs`).

use std::fs;
use std::path::Path;

use crate::json::extract_json_str;
use crate::orchestrator::{OrchestratorConfig, PresetProfile, RoleSlot};
use crate::session::is_valid_model_name;

pub(crate) fn push_unique(pool: &mut Vec<String>, m: &str) {
    let t = m.trim();
    if is_valid_model_name(t) && !pool.iter().any(|x| x.eq_ignore_ascii_case(t)) {
        pool.push(t.to_string());
    }
}

impl OrchestratorConfig {
    pub fn default_with_settings(home: &str) -> Self {
        let st =
            fs::read_to_string(Path::new(home).join(".claude/settings.json")).unwrap_or_default();
        let mut pool = Vec::new();
        for k in [
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "model",
        ] {
            if let Some(v) = extract_json_str(&st, k)
                .filter(|v| !matches!(v.as_str(), "fable" | "opus" | "sonnet" | "haiku"))
            {
                push_unique(&mut pool, &v);
            }
        }
        for b in [
            "gemini-4-carbon[1M]",
            "gemini-4-hydrogen[1M]",
            "deepseek-v4.1-flash[1M]",
            "gemini-3.8-flash-tiered",
            "kimi-k3",
            "glm-5.3",
            "inherit",
        ] {
            push_unique(&mut pool, b);
        }
        let h = pool.first().cloned().unwrap_or_else(|| "inherit".into());
        let mid = pool
            .iter()
            .find(|m| m.contains("hydrogen") || m.contains("sonnet"))
            .or_else(|| pool.get(1))
            .cloned()
            .unwrap_or_else(|| h.clone());
        let fast = pool
            .iter()
            .find(|m| m.contains("flash") || m.contains("haiku"))
            .cloned()
            .unwrap_or_else(|| mid.clone());
        let mk = |n: &str, s: [&str; 2], c: [&str; 2], r: [&str; 2], p: [&str; 2], w: [&str; 2]| {
            PresetProfile {
                name: n.into(),
                scout: RoleSlot::single(s[0], s[1]),
                coder: RoleSlot::single(c[0], c[1]),
                reviewer: RoleSlot::single(r[0], r[1]),
                plan: RoleSlot::single(p[0], p[1]),
                workflow: RoleSlot::single(w[0], w[1]),
            }
        };
        Self {
            active_preset: "balanced".into(),
            model_pool: pool,
            presets: vec![
                mk(
                    "balanced",
                    [&fast, "low"],
                    [&mid, "medium"],
                    [&h, "high"],
                    [&h, "high"],
                    [&mid, "medium"],
                ),
                mk(
                    "fast",
                    [&fast, "low"],
                    [&fast, "low"],
                    [&mid, "medium"],
                    [&mid, "medium"],
                    [&fast, "low"],
                ),
                mk(
                    "deep-audit",
                    [&mid, "medium"],
                    [&h, "high"],
                    [&h, "xhigh"],
                    [&h, "xhigh"],
                    [&h, "high"],
                ),
                PresetProfile {
                    name: "hetero-rr".into(),
                    scout: RoleSlot {
                        models: vec![fast, "gemini-3.8-flash-tiered".into()],
                        effort: "low".into(),
                    },
                    coder: RoleSlot {
                        models: vec![mid.clone(), "kimi-k3".into()],
                        effort: "medium".into(),
                    },
                    reviewer: RoleSlot {
                        models: vec![h.clone(), "kimi-k3".into(), "glm-5.3".into()],
                        effort: "high".into(),
                    },
                    plan: RoleSlot::single(&h, "xhigh"),
                    workflow: RoleSlot::single(&mid, "medium"),
                },
            ],
        }
    }
}
