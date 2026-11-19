//! `ccide-core pre-tool-hook [--agent claude|agy|codex]` & `ccide-core orchestrator` CLI.

use std::env;
use std::io::{Read, Write};

use crate::commands::workspace::actions::wake_all_sidebars;
use crate::commands::yazi_hook::hook_input::{detect_agent, parse_hook_event};
use crate::json::{extract_json_str, json_escape as json_esc};
use crate::orchestrator::router::route_pre_tool_use;
use crate::orchestrator::{
    sync_agent_shims, OrchestratorConfig, RoleKind, ALL_ROLES, EFFORT_TIERS,
};
use crate::platform::user_home_dir;
use crate::session::AgentKind;

fn map_agy_subagent_role(hint: &str) -> Option<RoleKind> {
    let low = hint.trim().to_ascii_lowercase();
    if let Some(r) = RoleKind::from_key(&low) {
        return Some(r);
    }
    if low.contains("scout") || low.contains("explore") || low.contains("search") {
        Some(RoleKind::Scout)
    } else if low.contains("plan") || low.contains("arch") {
        Some(RoleKind::Plan)
    } else if low.contains("review") || low.contains("verify") || low.contains("audit") {
        Some(RoleKind::Reviewer)
    } else if low.contains("code") || low.contains("impl") || low.contains("general") {
        Some(RoleKind::Coder)
    } else {
        None
    }
}

pub fn run_pre_tool_hook_with_agent(forced: Option<AgentKind>) {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    match detect_agent(&input, forced) {
        AgentKind::Codex => {
            let _ = std::io::stdout().flush();
        }
        AgentKind::Agy => {
            let routed =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
                    if env::var("CCIDE_AGY_ORCH").as_deref() != Ok("1") || input.trim().is_empty() {
                        return None;
                    }
                    let ev = parse_hook_event(&input, Some(AgentKind::Agy));
                    if ev.tool_name != "invoke_subagent" {
                        return None;
                    }
                    let role_hint = extract_json_str(&ev.tool_input, "Role")
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| extract_json_str(&ev.tool_input, "TypeName"))
                        .unwrap_or_default();
                    let role = map_agy_subagent_role(&role_hint)?;
                    let cfg = OrchestratorConfig::load(&user_home_dir());
                    cfg.active_profile()
                        .slot(role)
                        .models
                        .first()
                        .filter(|m| !m.is_empty())
                        .cloned()
                }))
                .ok()
                .flatten();
            if let Some(m) = routed {
                println!(
                    "{{\"decision\":\"allow\",\"overwrite\":{{\"Model\":\"{}\"}}}}",
                    json_esc(&m)
                );
            } else {
                println!("{{\"decision\":\"ask\"}}");
            }
            let _ = std::io::stdout().flush();
        }
        AgentKind::Claude => {
            if !input.trim().is_empty() {
                if let Some(out) = route_pre_tool_use(&input, &user_home_dir()) {
                    println!("{out}");
                    let _ = std::io::stdout().flush();
                }
            }
        }
    }
}

fn print_status(cfg: &OrchestratorConfig, json_mode: bool) {
    let prof = cfg.active_profile();
    if json_mode {
        let pool: Vec<String> = cfg
            .model_pool
            .iter()
            .map(|m| format!("\"{}\"", json_esc(m)))
            .collect();
        let roles: Vec<String> = ALL_ROLES
            .iter()
            .map(|&r| {
                let s = prof.slot(r);
                let ms: Vec<String> = s
                    .models
                    .iter()
                    .map(|m| format!("\"{}\"", json_esc(m)))
                    .collect();
                format!(
                    "\"{}\":{{\"models\":[{}],\"effort\":\"{}\"}}",
                    r.key(),
                    ms.join(","),
                    json_esc(&s.effort)
                )
            })
            .collect();
        println!(
            "{{\"active_preset\":\"{}\",\"model_pool\":[{}],\"roles\":{{{}}}}}",
            json_esc(&cfg.active_preset),
            pool.join(","),
            roles.join(",")
        );
        return;
    }
    println!(
        "Active preset: {}\nModel pool: {}",
        cfg.active_preset,
        cfg.model_pool.join(", ")
    );
    for role in ALL_ROLES {
        let slot = prof.slot(role);
        println!(
            "  {:<8} -> [{}] (effort: {})",
            role.key(),
            slot.models.join(", "),
            slot.effort
        );
    }
}

pub fn run_orchestrator_cli(args: &[String]) {
    let home = user_home_dir();
    match args.get(2).map(|s| s.as_str()).unwrap_or("status") {
        "status" => {
            let json_mode = args.iter().skip(3).any(|a| a == "--json" || a == "-j");
            print_status(&OrchestratorConfig::load(&home), json_mode);
        }
        "preset" => {
            let Some(name) = args.get(3).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                eprintln!("Usage: ccide-core orchestrator preset <name>");
                return;
            };
            let mut cfg = OrchestratorConfig::load(&home);
            if !cfg.presets.iter().any(|p| p.name == name) {
                let avail: Vec<&str> = cfg.presets.iter().map(|p| p.name.as_str()).collect();
                eprintln!("Unknown preset '{name}'. Available: {}", avail.join(", "));
                return;
            }
            cfg.active_preset = name.to_string();
            cfg.save(&home);
            wake_all_sidebars();
            println!("Active preset set to '{}'.", cfg.active_preset);
        }
        "set" => {
            let (Some(role_str), Some(models_raw)) = (
                args.get(3).map(|s| s.as_str()),
                args.get(4).map(|s| s.trim()).filter(|s| !s.is_empty()),
            ) else {
                eprintln!("Usage: ccide-core orchestrator set <role> <model[,model2]> [effort]");
                return;
            };
            let Some(role) = RoleKind::from_key(role_str) else {
                eprintln!("Unknown role '{role_str}'. Expected: scout, coder, reviewer, plan, workflow");
                return;
            };
            let models: Vec<String> = models_raw.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
            if models.is_empty() {
                eprintln!("At least one model must be provided.");
                return;
            }
            let mut cfg = OrchestratorConfig::load(&home);
            for m in &models {
                if !cfg.model_pool.iter().any(|x| x.eq_ignore_ascii_case(m)) {
                    cfg.model_pool.push(m.clone());
                }
            }
            if let Some(ef) = args.get(5).map(|s| s.trim().to_ascii_lowercase()) {
                let norm = if ef == "med" { "medium" } else { ef.as_str() };
                if !EFFORT_TIERS.contains(&norm) {
                    eprintln!("Invalid effort '{ef}'. Expected: {}", EFFORT_TIERS.join(", "));
                    return;
                }
                cfg.active_profile_mut().slot_mut(role).effort = norm.to_string();
            }
            cfg.active_profile_mut().slot_mut(role).models = models;
            cfg.save(&home);
            wake_all_sidebars();
            println!("Updated '{}' in preset '{}'.", role.key(), cfg.active_preset);
        }
        "sync" => {
            let cfg = OrchestratorConfig::load(&home);
            sync_agent_shims(&home, &cfg.model_pool);
            cfg.save(&home);
            println!("Synced {} model shims.", cfg.model_pool.len() * 2);
        }
        _ => eprintln!("Usage: ccide-core orchestrator <status [--json] | preset <name> | set <role> <model[,model2]> [effort] | sync>"),
    }
}
