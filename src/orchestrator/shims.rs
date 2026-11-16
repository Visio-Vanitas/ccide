//! Custom Agent Pool Shim Generator & `orchestrator.json` Persistence (`src/orchestrator/shims.rs`).
//!
//! Materializes `~/.claude/agents/ccide-{gp,ro}--<slug>.md` so `PreToolUse` can route `Agent`
//! dispatches to ANY gateway model ID at runtime without restarting Claude Code.

use std::fs;
use std::path::Path;

use crate::json::json_escape;
pub use crate::model_id::{gp_shim_name, ro_shim_name};
#[allow(unused_imports)]
pub use crate::model_id::{model_from_shim_agent_type, model_slug};
use crate::orchestrator::{
    resolve_orchestrator_path, OrchestratorConfig, RoleKind, ALL_ROLES, EFFORT_TIERS,
};
use crate::platform::atomic_write;

fn write_if_changed(path: &Path, content: &str) {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == content {
            return;
        }
    }
    let _ = atomic_write(path, content.as_bytes());
}

/// Synchronizes `~/.claude/agents/ccide-{gp,ro}--<slug>.md` shims for every model in `pool`.
pub fn sync_agent_shims(home: &str, pool: &[String]) {
    if home.is_empty() || home == "." {
        return;
    }
    let agents_dir = Path::new(home).join(".claude").join("agents");
    if fs::create_dir_all(&agents_dir).is_err() {
        return;
    }
    let mut expected = Vec::with_capacity(pool.len() * 2);
    for raw_m in pool {
        let m = raw_m.trim();
        if m.is_empty() {
            continue;
        }
        let gp = gp_shim_name(m);
        let ro = ro_shim_name(m);
        let safe_m: String = m
            .chars()
            .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
            .collect();
        let gp_md = format!(
            "---\nname: {gp}\ndescription: \"[cc-ide] Routed general-purpose subagent on {safe_m}\"\nmodel: \"{safe_m}\"\n---\nYou are a focused engineering subagent orchestrated by cc-ide. Execute the assigned task accurately and concisely.\n"
        );
        let ro_md = format!(
            "---\nname: {ro}\ndescription: \"[cc-ide] Routed read-only scout/plan subagent on {safe_m}\"\nmodel: \"{safe_m}\"\ndisallowedTools:\n  - Write\n  - Edit\n  - MultiEdit\n  - NotebookEdit\n  - ExitPlanMode\n---\nYou are a read-only exploration and architecture subagent orchestrated by cc-ide. Inspect thoroughly and report concise, exact findings.\n"
        );
        let gp_file = format!("{gp}.md");
        let ro_file = format!("{ro}.md");
        write_if_changed(&agents_dir.join(&gp_file), &gp_md);
        write_if_changed(&agents_dir.join(&ro_file), &ro_md);
        expected.push(gp_file);
        expected.push(ro_file);
    }
    if let Ok(rd) = fs::read_dir(&agents_dir) {
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().into_owned();
            if (fname.starts_with("ccide-gp--") || fname.starts_with("ccide-ro--"))
                && fname.ends_with(".md")
                && !expected.iter().any(|x| x == &fname)
            {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

impl OrchestratorConfig {
    pub fn cycle_preset(&mut self, forward: bool) {
        if self.presets.is_empty() {
            return;
        }
        let cur = self
            .presets
            .iter()
            .position(|p| p.name == self.active_preset)
            .unwrap_or(0);
        let next = if forward {
            (cur + 1) % self.presets.len()
        } else {
            (cur + self.presets.len() - 1) % self.presets.len()
        };
        self.active_preset = self.presets[next].name.clone();
    }

    pub fn cycle_role_model(&mut self, role: RoleKind, forward: bool) {
        if self.model_pool.is_empty() {
            return;
        }
        let pool = self.model_pool.clone();
        let slot = self.active_profile_mut().slot_mut(role);
        let cur_m = slot.primary_model();
        let pos = pool
            .iter()
            .position(|m| m.eq_ignore_ascii_case(cur_m))
            .unwrap_or(0);
        let next = if forward {
            (pos + 1) % pool.len()
        } else {
            (pos + pool.len() - 1) % pool.len()
        };
        slot.models = vec![pool[next].clone()];
    }

    pub fn cycle_role_effort(&mut self, role: RoleKind, forward: bool) {
        let slot = self.active_profile_mut().slot_mut(role);
        let pos = EFFORT_TIERS
            .iter()
            .position(|&e| e.eq_ignore_ascii_case(&slot.effort))
            .unwrap_or(1);
        let next = if forward {
            (pos + 1) % EFFORT_TIERS.len()
        } else {
            (pos + EFFORT_TIERS.len() - 1) % EFFORT_TIERS.len()
        };
        slot.effort = EFFORT_TIERS[next].to_string();
    }

    pub fn to_jsonc_string(&self) -> String {
        let mut out = String::from(
            "{\n  // Subagent & Native Workflow Model/Effort Orchestration (`~/.config/cc-ide/orchestrator.json`).\n",
        );
        out.push_str(&format!(
            "  \"active_preset\": \"{}\",\n  \"model_pool\": [\n",
            json_escape(&self.active_preset)
        ));
        for (i, m) in self.model_pool.iter().enumerate() {
            let comma = if i + 1 < self.model_pool.len() {
                ","
            } else {
                ""
            };
            out.push_str(&format!("    \"{}\"{}\n", json_escape(m), comma));
        }
        out.push_str("  ],\n  \"presets\": {\n");
        for (pi, prof) in self.presets.iter().enumerate() {
            out.push_str(&format!("    \"{}\": {{\n", json_escape(&prof.name)));
            for (ri, role) in ALL_ROLES.iter().enumerate() {
                let slot = prof.slot(*role);
                let ms: Vec<String> = slot
                    .models
                    .iter()
                    .map(|m| format!("\"{}\"", json_escape(m)))
                    .collect();
                let rcomma = if ri + 1 < ALL_ROLES.len() { "," } else { "" };
                out.push_str(&format!(
                    "      \"{}\": {{ \"models\": [{}], \"effort\": \"{}\" }}{}\n",
                    role.key(),
                    ms.join(", "),
                    json_escape(&slot.effort),
                    rcomma
                ));
            }
            let pcomma = if pi + 1 < self.presets.len() { "," } else { "" };
            out.push_str(&format!("    }}{}\n", pcomma));
        }
        out.push_str("  }\n}\n");
        out
    }

    pub fn save(&self, home: &str) {
        let dst = resolve_orchestrator_path(home);
        let body = self.to_jsonc_string();
        let _ = atomic_write(&dst, body.as_bytes());
        sync_agent_shims(home, &self.model_pool);
    }
}
