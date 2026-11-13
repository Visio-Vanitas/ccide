//! Subagent-Clear & Live JSONC Theme Switching Actions for `ccide-core workspace`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::ffi::{is_pid_alive, wake_pid};
use crate::json::{extract_json_str, extract_json_u64};
use crate::platform::ipc_dir;
use crate::session::transcript::{append_dismissed_subagents, get_session_subagents};
use crate::tui::palette::{render_palette_template, resolve_palette_path, Palette, PRESET_NAMES};
use crate::tui::RESET;

pub fn wake_all_sidebars() {
    if let Ok(rd) = fs::read_dir(ipc_dir()) {
        for entry in rd.flatten() {
            let fname = entry.file_name();
            let name = fname.to_string_lossy();
            if name.starts_with("sb_") && name.ends_with(".pid") {
                if let Ok(ptxt) = fs::read_to_string(entry.path()) {
                    if let Ok(pid) = ptxt.trim().parse::<u32>() {
                        wake_pid(pid);
                    }
                }
            }
        }
    }
}

fn replace_preset_in_jsonc(existing: &str, new_preset: &str) -> Option<String> {
    let needle = "\"preset\"";
    let pos = existing.find(needle)?;
    let after_key = &existing[pos + needle.len()..];
    let colon_off = after_key.find(':')?;
    let after_colon = &after_key[colon_off + 1..];
    let q1_off = after_colon.find('"')?;
    let rest_val = &after_colon[q1_off + 1..];
    let q2_off = rest_val.find('"')?;

    let val_start = pos + needle.len() + colon_off + 1 + q1_off + 1;
    let val_end = val_start + q2_off;
    let mut out = String::with_capacity(existing.len() + 16);
    out.push_str(&existing[..val_start]);
    out.push_str(new_preset);
    out.push_str(&existing[val_end..]);
    Some(out)
}

pub fn run_clear_subagents(home: &str, explicit_sid: String) {
    let mut target_sids: Vec<(String, String)> = Vec::new();
    if !explicit_sid.is_empty() && explicit_sid != "--all" {
        target_sids.push((explicit_sid, ".".to_string()));
    } else if let Ok(rd) = fs::read_dir(Path::new(home).join(".claude/sessions")) {
        for entry in rd.flatten() {
            if let Ok(txt) = fs::read_to_string(entry.path()) {
                let pid = extract_json_u64(&txt, "pid").unwrap_or(0) as u32;
                if is_pid_alive(pid) {
                    if let Some(sid) = extract_json_str(&txt, "sessionId") {
                        let cwd = extract_json_str(&txt, "cwd").unwrap_or_else(|| ".".to_string());
                        target_sids.push((sid, cwd));
                    }
                }
            }
        }
    }
    let mut total_cleared = 0usize;
    let mut trackers = HashMap::new();
    for (sid, cwd) in &target_sids {
        let items = get_session_subagents(home, sid, cwd, &mut trackers);
        let done_ids: Vec<String> = items
            .into_iter()
            .filter(|a| a.status != "running")
            .map(|a| a.full_id)
            .collect();
        if !done_ids.is_empty() {
            total_cleared += done_ids.len();
            append_dismissed_subagents(home, sid, &done_ids);
        }
    }
    wake_all_sidebars();
    println!("Cleared {} completed subagent(s).", total_cleared);
}

pub fn run_theme_command(home: &str, sub: &str) {
    let cfg_path = resolve_palette_path(home);
    match sub {
        "path" => println!("{}", cfg_path.display()),
        "init" => {
            if !cfg_path.exists() {
                if let Some(parent) = cfg_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(&cfg_path, render_palette_template("default"));
            }
            println!("{}", cfg_path.display());
        }
        "reset" => {
            if let Some(parent) = cfg_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&cfg_path, render_palette_template("default"));
            wake_all_sidebars();
            println!("Reset palette to default -> {}", cfg_path.display());
        }
        "list" | "" => {
            let active = Palette::load(home);
            println!("Palette file : {}", cfg_path.display());
            println!("Active preset: {}", active.preset);
            println!("Presets:");
            for name in PRESET_NAMES {
                let p = Palette::from_json_str(&format!("{{\"preset\":\"{}\"}}", name));
                let mark = if active.preset == name { "●" } else { " " };
                println!(
                    "  {} {:<18} {}██{} {}██{} {}██{} {}██{} {}██{}",
                    mark,
                    name,
                    p.cyan,
                    RESET,
                    p.mint,
                    RESET,
                    p.amber,
                    RESET,
                    p.coral,
                    RESET,
                    p.lavender,
                    RESET
                );
            }
        }
        preset_arg => {
            let probe = Palette::from_json_str(&format!("{{\"preset\":\"{}\"}}", preset_arg));
            let is_valid = preset_arg.eq_ignore_ascii_case("default") || probe.preset != "default";
            if !is_valid {
                eprintln!(
                    "Unknown preset '{}'. Available presets: {}",
                    preset_arg,
                    PRESET_NAMES.join(", ")
                );
                std::process::exit(1);
            }
            if let Some(parent) = cfg_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let updated = if let Ok(existing) = fs::read_to_string(&cfg_path) {
                replace_preset_in_jsonc(&existing, &probe.preset)
                    .unwrap_or_else(|| render_palette_template(&probe.preset))
            } else {
                render_palette_template(&probe.preset)
            };
            let _ = fs::write(&cfg_path, updated);
            wake_all_sidebars();
            println!(
                "Switched palette preset to '{}' ({}) and hot-reloaded open sidebars.",
                probe.preset,
                cfg_path.display()
            );
        }
    }
}
