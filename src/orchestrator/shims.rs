//! Custom Agent Pool Shim Generator & `orchestrator.json` Persistence (`src/orchestrator/shims.rs`).
//!
//! Materializes `~/.claude/agents/ccide-{gp,ro}--<slug>.md` so `PreToolUse` can route `Agent`
//! dispatches to ANY gateway model ID at runtime without restarting Claude Code.

use std::fs;
use std::path::Path;

use crate::orchestrator::{
    resolve_orchestrator_path, OrchestratorConfig, RoleKind, ALL_ROLES, EFFORT_TIERS,
};

pub fn model_slug(model_id: &str) -> String {
    let mut out = String::with_capacity(model_id.len());
    let mut prev_dash = false;
    for ch in model_id.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "inherit".to_string()
    } else {
        out
    }
}

pub fn gp_shim_name(model_id: &str) -> String {
    format!("ccide-gp--{}", model_slug(model_id))
}

pub fn ro_shim_name(model_id: &str) -> String {
    format!("ccide-ro--{}", model_slug(model_id))
}

/// Resolves `(clean_role_badge, model_id)` if `agent_type` is a managed `ccide-{gp,ro}--<slug>` shim.
pub fn model_from_shim_agent_type(home: &str, agent_type: &str) -> Option<(&'static str, String)> {
    let clean_name = agent_type.rsplit(':').next().unwrap_or(agent_type);
    let role_badge = if clean_name.starts_with("ccide-ro--") {
        "scout"
    } else if clean_name.starts_with("ccide-gp--") {
        "worker"
    } else {
        return None;
    };
    let md_path = Path::new(home)
        .join(".claude/agents")
        .join(format!("{}.md", clean_name));
    if let Ok(txt) = fs::read_to_string(md_path) {
        for line in txt.lines().take(12) {
            if let Some(rest) = line.trim().strip_prefix("model:") {
                let m = rest.trim().trim_matches('"').trim_matches('\'');
                if !m.is_empty() {
                    return Some((role_badge, m.to_string()));
                }
            }
        }
    }
    let slug = clean_name
        .strip_prefix("ccide-gp--")
        .or_else(|| clean_name.strip_prefix("ccide-ro--"))?;
    Some((role_badge, slug.to_string()))
}

fn write_if_changed(path: &Path, content: &str) {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == content {
            return;
        }
    }
    let tmp = path.with_extension(format!("md.tmp.{}", std::process::id()));
    if fs::write(&tmp, content).is_ok() && fs::rename(&tmp, path).is_err() {
        let _ = fs::write(path, content);
        let _ = fs::remove_file(&tmp);
    }
}

/// Synchronizes `~/.claude/agents/ccide-{gp,ro}--<slug>.md` shims for every model in `pool`.
pub fn sync_agent_shims(home: &str, pool: &[String]) {
    if home.is_empty() || home == "." {
        return;
    }
    let agents_dir = Path::new(home).join(".claude/agents");
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
        let safe_m = m.replace('"', "");
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

fn esc_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
            esc_json(&self.active_preset)
        ));
        for (i, m) in self.model_pool.iter().enumerate() {
            let comma = if i + 1 < self.model_pool.len() {
                ","
            } else {
                ""
            };
            out.push_str(&format!("    \"{}\"{}\n", esc_json(m), comma));
        }
        out.push_str("  ],\n  \"presets\": {\n");
        for (pi, prof) in self.presets.iter().enumerate() {
            out.push_str(&format!("    \"{}\": {{\n", esc_json(&prof.name)));
            for (ri, role) in ALL_ROLES.iter().enumerate() {
                let slot = prof.slot(*role);
                let ms: Vec<String> = slot
                    .models
                    .iter()
                    .map(|m| format!("\"{}\"", esc_json(m)))
                    .collect();
                let rcomma = if ri + 1 < ALL_ROLES.len() { "," } else { "" };
                out.push_str(&format!(
                    "      \"{}\": {{ \"models\": [{}], \"effort\": \"{}\" }}{}\n",
                    role.key(),
                    ms.join(", "),
                    esc_json(&slot.effort),
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
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let body = self.to_jsonc_string();
        let tmp = dst.with_extension(format!("json.tmp.{}", std::process::id()));
        if fs::write(&tmp, &body).is_ok() && fs::rename(&tmp, &dst).is_err() {
            let _ = fs::write(&dst, &body);
            let _ = fs::remove_file(&tmp);
        }
        sync_agent_shims(home, &self.model_pool);
    }
}
