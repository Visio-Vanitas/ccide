//! Subagent-Clear & Live JSONC Theme Switching Actions for `ccide-core workspace`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::ffi::{is_pid_alive, wake_token, ProcToken};
use crate::json::{extract_json_str, extract_json_u64};
use crate::platform::{find_executable, ipc_dir};
use crate::session::transcript::{append_dismissed_subagents, get_session_subagents};
use crate::tui::palette::{
    render_palette_template, resolve_palette_path, sync_syntax_artifacts, Palette, PRESET_NAMES,
};
use crate::tui::RESET;

pub fn wake_all_sidebars() {
    if let Ok(rd) = fs::read_dir(ipc_dir()) {
        for entry in rd.flatten() {
            let fname = entry.file_name();
            let name = fname.to_string_lossy();
            if name.starts_with("sb_") && name.ends_with(".pid") {
                if let Ok(ptxt) = fs::read_to_string(entry.path()) {
                    if let Some(tok) = ProcToken::parse(&ptxt) {
                        if tok.is_alive_match() {
                            let _ = wake_token(&tok);
                            continue;
                        }
                    }
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
}

pub fn refresh_open_yazi_previews(home: &str) {
    let _ = crate::commands::workspace::spec::prune_dead_specs(home);
    let Some(ya) = find_executable("ya", home) else {
        return;
    };
    let Ok(rd) = fs::read_dir(ipc_dir()) else {
        return;
    };
    for entry in rd.flatten() {
        let fname = entry.file_name();
        let name = fname.to_string_lossy();
        if let Some(yid) = name
            .strip_prefix("panes_")
            .and_then(|s| s.strip_suffix(".txt"))
        {
            if !yid.is_empty() && yid.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(mut ch) = Command::new(&ya)
                    .args(["emit-to", yid, "plugin", "hscroll", "refresh"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    let _ = std::thread::spawn(move || {
                        let _ = ch.wait();
                    });
                }
            }
        }
    }
}

fn replace_preset_in_jsonc(existing: &str, new_preset: &str) -> Option<String> {
    let bytes = existing.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        if bytes[i] == b'"' {
            if existing[i..].starts_with("\"preset\"") {
                let after_key = &existing[i + 8..];
                let trimmed = after_key.trim_start();
                if let Some(after_colon) = trimmed.strip_prefix(':') {
                    let val_trim = after_colon.trim_start();
                    if let Some(rest_val) = val_trim.strip_prefix('"') {
                        if let Some(q2_off) = rest_val.find('"') {
                            let val_start = existing.len() - rest_val.len();
                            let val_end = val_start + q2_off;
                            let mut out = String::with_capacity(existing.len() + 16);
                            out.push_str(&existing[..val_start]);
                            out.push_str(new_preset);
                            out.push_str(&existing[val_end..]);
                            return Some(out);
                        }
                    }
                }
            }
            i += 1;
            while i < n {
                if bytes[i] == b'\\' && i + 1 < n {
                    i += 1;
                    if let Some(ch) = existing[i..].chars().next() {
                        i += ch.len_utf8();
                    }
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                if let Some(ch) = existing[i..].chars().next() {
                    i += ch.len_utf8();
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if let Some(ch) = existing[i..].chars().next() {
            i += ch.len_utf8();
        } else {
            i += 1;
        }
    }
    None
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
            let _ = sync_syntax_artifacts(home, &Palette::load(home));
            println!("{}", cfg_path.display());
        }
        "reset" => {
            if let Some(parent) = cfg_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&cfg_path, render_palette_template("default"));
            let _ = sync_syntax_artifacts(home, &Palette::load(home));
            wake_all_sidebars();
            crate::commands::workspace::reload::reload_companion_panes(home);
            println!("Reset palette to default -> {}", cfg_path.display());
        }
        "sync" => {
            let _ = sync_syntax_artifacts(home, &Palette::load(home));
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
            let _ = sync_syntax_artifacts(home, &Palette::load(home));
            wake_all_sidebars();
            crate::commands::workspace::reload::reload_companion_panes(home);
            println!(
                "Switched palette preset to '{}' ({}) and hot-reloaded workspace + companion panes.",
                probe.preset,
                cfg_path.display()
            );
        }
    }
}
