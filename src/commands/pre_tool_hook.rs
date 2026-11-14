//! `ccide-core pre-tool-hook` & `ccide-core orchestrator` CLI (`src/commands/pre_tool_hook.rs`).

use std::io::Read;

use crate::commands::workspace::actions::wake_all_sidebars;
use crate::orchestrator::router::{json_esc, route_pre_tool_use};
use crate::orchestrator::{
    sync_agent_shims, OrchestratorConfig, RoleKind, ALL_ROLES, EFFORT_TIERS,
};
use crate::platform::user_home_dir;

pub fn run_pre_tool_hook() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if input.trim().is_empty() {
        return;
    }
    let home = user_home_dir();
    if let Some(out) = route_pre_tool_use(&input, &home) {
        println!("{}", out);
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
    println!("Active preset: {}", cfg.active_preset);
    println!("Model pool: {}", cfg.model_pool.join(", "));
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
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("status");
    match sub {
        "status" => {
            let json_mode = args.iter().skip(3).any(|a| a == "--json" || a == "-j");
            let cfg = OrchestratorConfig::load(&home);
            print_status(&cfg, json_mode);
        }
        "preset" => {
            let Some(name) = args.get(3).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                eprintln!("Usage: ccide-core orchestrator preset <name>");
                return;
            };
            let mut cfg = OrchestratorConfig::load(&home);
            if !cfg.presets.iter().any(|p| p.name == name) {
                let avail: Vec<&str> = cfg.presets.iter().map(|p| p.name.as_str()).collect();
                eprintln!("Unknown preset '{}'. Available: {}", name, avail.join(", "));
                return;
            }
            cfg.active_preset = name.to_string();
            cfg.save(&home);
            wake_all_sidebars();
            println!("Active preset set to '{}'.", cfg.active_preset);
        }
        "set" => {
            let Some(role_str) = args.get(3).map(|s| s.as_str()) else {
                eprintln!("Usage: ccide-core orchestrator set <role> <model[,model2]> [effort]");
                return;
            };
            let Some(role) = RoleKind::from_key(role_str) else {
                eprintln!(
                    "Unknown role '{}'. Expected: scout, coder, reviewer, plan, workflow",
                    role_str
                );
                return;
            };
            let Some(models_raw) = args.get(4).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                eprintln!("Usage: ccide-core orchestrator set <role> <model[,model2]> [effort]");
                return;
            };
            let models: Vec<String> = models_raw
                .split(',')
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect();
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
            let effort_opt = args.get(5).map(|s| s.trim().to_ascii_lowercase());
            if let Some(ref ef) = effort_opt {
                let norm = if ef == "med" { "medium" } else { ef.as_str() };
                if !EFFORT_TIERS.contains(&norm) {
                    eprintln!(
                        "Invalid effort '{}'. Expected: {}",
                        ef,
                        EFFORT_TIERS.join(", ")
                    );
                    return;
                }
                cfg.active_profile_mut().slot_mut(role).effort = norm.to_string();
            }
            cfg.active_profile_mut().slot_mut(role).models = models;
            cfg.save(&home);
            wake_all_sidebars();
            println!(
                "Updated '{}' in preset '{}'.",
                role.key(),
                cfg.active_preset
            );
        }
        "sync" => {
            let cfg = OrchestratorConfig::load(&home);
            sync_agent_shims(&home, &cfg.model_pool);
            cfg.save(&home);
            println!("Synced {} model shims.", cfg.model_pool.len() * 2);
        }
        _ => {
            eprintln!("Usage: ccide-core orchestrator <status [--json] | preset <name> | set <role> <model[,model2]> [effort] | sync>");
        }
    }
}
