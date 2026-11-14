//! Subagent & Native `Workflow` Model/Effort Orchestration Store (`src/orchestrator/mod.rs`).

pub mod presets;
pub mod router;
pub mod shims;
pub mod workflow_router;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::json::{extract_json_object, extract_json_str};
use crate::session::is_valid_model_name;
use crate::tui::context::json_scan::{extract_json_array_slice, extract_string_list_in_array};
use crate::tui::palette::strip_jsonc_comments;

pub use shims::{gp_shim_name, model_from_shim_agent_type, ro_shim_name, sync_agent_shims};

pub const EFFORT_TIERS: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "inherit"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleKind {
    Scout,
    Coder,
    Reviewer,
    Plan,
    Workflow,
}

pub const ALL_ROLES: [RoleKind; 5] = [
    RoleKind::Scout,
    RoleKind::Coder,
    RoleKind::Reviewer,
    RoleKind::Plan,
    RoleKind::Workflow,
];

impl RoleKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Scout => "scout",
            Self::Coder => "coder",
            Self::Reviewer => "reviewer",
            Self::Plan => "plan",
            Self::Workflow => "workflow",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Scout => "Scout",
            Self::Coder => "Coder",
            Self::Reviewer => "Review",
            Self::Plan => "Plan",
            Self::Workflow => "Workflow",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "scout" | "explore" | "search" => Some(Self::Scout),
            "coder" | "code" | "implement" | "general" => Some(Self::Coder),
            "reviewer" | "review" | "audit" | "verify" => Some(Self::Reviewer),
            "plan" | "architect" | "design" => Some(Self::Plan),
            "workflow" | "wf" => Some(Self::Workflow),
            _ => None,
        }
    }
}

pub fn short_effort_badge(eff: &str) -> &'static str {
    match eff.trim().to_ascii_lowercase().as_str() {
        "low" => "low",
        "medium" | "med" => "med",
        "high" => "high",
        "xhigh" => "xhig",
        "max" => "max",
        _ => "auto",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleSlot {
    pub models: Vec<String>,
    pub effort: String,
}

impl RoleSlot {
    pub fn single(model: &str, effort: &str) -> Self {
        Self {
            models: vec![model.to_string()],
            effort: effort.to_string(),
        }
    }

    pub fn primary_model(&self) -> &str {
        self.models.first().map(|s| s.as_str()).unwrap_or("inherit")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetProfile {
    pub name: String,
    pub scout: RoleSlot,
    pub coder: RoleSlot,
    pub reviewer: RoleSlot,
    pub plan: RoleSlot,
    pub workflow: RoleSlot,
}

impl PresetProfile {
    pub fn slot(&self, role: RoleKind) -> &RoleSlot {
        match role {
            RoleKind::Scout => &self.scout,
            RoleKind::Coder => &self.coder,
            RoleKind::Reviewer => &self.reviewer,
            RoleKind::Plan => &self.plan,
            RoleKind::Workflow => &self.workflow,
        }
    }

    pub fn slot_mut(&mut self, role: RoleKind) -> &mut RoleSlot {
        match role {
            RoleKind::Scout => &mut self.scout,
            RoleKind::Coder => &mut self.coder,
            RoleKind::Reviewer => &mut self.reviewer,
            RoleKind::Plan => &mut self.plan,
            RoleKind::Workflow => &mut self.workflow,
        }
    }
}

#[derive(Clone, Debug)]
pub struct OrchestratorConfig {
    pub active_preset: String,
    pub model_pool: Vec<String>,
    pub presets: Vec<PresetProfile>,
}

pub fn resolve_orchestrator_path(home: &str) -> PathBuf {
    env::var("CCIDE_ORCHESTRATOR_FILE")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .map(|p| PathBuf::from(p.trim()))
        .unwrap_or_else(|| Path::new(home).join(".config/cc-ide/orchestrator.json"))
}

use presets::push_unique;

impl OrchestratorConfig {
    pub fn load(home: &str) -> Self {
        let mut cfg = Self::default_with_settings(home);
        let Ok(raw) = fs::read_to_string(resolve_orchestrator_path(home)) else {
            return cfg;
        };
        let clean = strip_jsonc_comments(&raw);
        if let Some(ap) = extract_json_str(&clean, "active_preset").filter(|s| !s.is_empty()) {
            cfg.active_preset = ap;
        }
        if let Some(arr) = extract_json_array_slice(&clean, "model_pool") {
            let parsed: Vec<String> = extract_string_list_in_array(arr)
                .into_iter()
                .filter(|s| is_valid_model_name(s))
                .collect();
            if !parsed.is_empty() {
                cfg.model_pool = parsed;
            }
        }
        if let Some(p_obj) = extract_json_object(&clean, "presets") {
            for prof in &mut cfg.presets {
                if let Some(po) = extract_json_object(p_obj, &prof.name) {
                    for role in ALL_ROLES {
                        if let Some(ro) = extract_json_object(po, role.key()) {
                            let slot = prof.slot_mut(role);
                            if let Some(ma) = extract_json_array_slice(ro, "models") {
                                let ms = extract_string_list_in_array(ma);
                                if !ms.is_empty() {
                                    slot.models = ms;
                                }
                            } else if let Some(m1) =
                                extract_json_str(ro, "model").filter(|s| !s.is_empty())
                            {
                                slot.models = vec![m1];
                            }
                            if let Some(ef) =
                                extract_json_str(ro, "effort").filter(|s| !s.is_empty())
                            {
                                slot.effort = ef;
                            }
                        }
                    }
                }
            }
        }
        for prof in &cfg.presets {
            for role in ALL_ROLES {
                for m in &prof.slot(role).models {
                    push_unique(&mut cfg.model_pool, m);
                }
            }
        }
        cfg
    }

    pub fn active_profile(&self) -> &PresetProfile {
        self.presets
            .iter()
            .find(|p| p.name == self.active_preset)
            .unwrap_or(&self.presets[0])
    }

    pub fn active_profile_mut(&mut self) -> &mut PresetProfile {
        let idx = self
            .presets
            .iter()
            .position(|p| p.name == self.active_preset)
            .unwrap_or(0);
        &mut self.presets[idx]
    }
}

pub struct OrchestratorWatcher {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    pub cfg: OrchestratorConfig,
}

impl OrchestratorWatcher {
    pub fn new(home: &str) -> Self {
        let path = resolve_orchestrator_path(home);
        let last_mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        let cfg = OrchestratorConfig::load(home);
        sync_agent_shims(home, &cfg.model_pool);
        Self {
            path,
            last_mtime,
            cfg,
        }
    }

    pub fn reload_if_changed(&mut self, home: &str) -> bool {
        let cur = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if cur != self.last_mtime {
            self.last_mtime = cur;
            self.cfg = OrchestratorConfig::load(home);
            return true;
        }
        false
    }
}
