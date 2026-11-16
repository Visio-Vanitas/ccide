//! Environment, Companion Tool & Binary Patch Health Diagnostics (`ccide-core doctor [--json]`).

#[path = "doctor_probes.rs"]
mod doctor_probes;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::setup::doctor_render::{format_doctor_json, print_doctor_tui};
use crate::commands::workspace::ghostty::{probe_ghostty, GhosttyCap};
use crate::json::extract_json_str;
use crate::platform::{clean_canonicalize, find_executable, ipc_dir, user_home_dir};

use doctor_probes::{
    cmd_first_line, probe_claude_and_orchestrator, probe_companion_tools, probe_legacy_patch_proto,
    probe_macos_iterm,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Warn,
    Info,
}

#[derive(Clone, Debug)]
pub struct DoctorItem {
    pub status: CheckStatus,
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct DoctorReport {
    pub os_arch: String,
    pub backend_tier: String,
    pub ipc_dir: String,
    pub ghostty_cap: &'static str,
    pub legacy_patch: bool,
    pub items: Vec<DoctorItem>,
    pub missing_hint: String,
}

/// Shorthand for a `DoctorItem`.
pub(super) fn item(status: CheckStatus, label: &str, detail: String) -> DoctorItem {
    DoctorItem {
        status,
        label: label.to_string(),
        detail,
    }
}

fn has_installer_marker(dir: &Path) -> bool {
    dir.join("install.sh").is_file() || dir.join("install.ps1").is_file()
}

pub fn resolve_project_root(explicit_arg: Option<&str>, home: &str) -> PathBuf {
    if let Some(s) = explicit_arg {
        let t = s.trim();
        if !t.is_empty() && !t.starts_with('-') {
            let p = PathBuf::from(t);
            if has_installer_marker(&p) {
                return p;
            }
        }
    }
    if let Ok(exe) = env::current_exe().and_then(clean_canonicalize) {
        if let Some(parent) = exe.parent().and_then(|p| p.parent()) {
            if has_installer_marker(parent) {
                return parent.to_path_buf();
            }
        }
    }
    if let Ok(cwd) = env::current_dir() {
        if has_installer_marker(&cwd) {
            return cwd;
        }
    }
    if let Ok(txt) = fs::read_to_string(Path::new(home).join(".claude/settings.json")) {
        if let Some(p) = extract_json_str(&txt, "path") {
            let pb = PathBuf::from(&p);
            if has_installer_marker(&pb) {
                return pb;
            }
        }
    }
    PathBuf::from(".")
}

pub fn collect_doctor_report(home: &str, project_root: &Path) -> DoctorReport {
    let os_arch = format!("{}/{}", env::consts::OS, env::consts::ARCH);
    let ipc_dir_s = ipc_dir().to_string_lossy().into_owned();
    let is_ssh = env::var("SSH_CONNECTION").is_ok() || env::var("SSH_TTY").is_ok();
    let is_tmux = env::var("TMUX").is_ok();
    let is_wezterm =
        env::var("WEZTERM_PANE").is_ok() || env::var("TERM_PROGRAM").as_deref() == Ok("WezTerm");
    let is_wt = env::var("WT_SESSION").is_ok();
    let is_iterm = env::var("TERM_PROGRAM").as_deref() == Ok("iTerm.app")
        || env::var("LC_TERMINAL").as_deref() == Ok("iTerm2")
        || env::var("ITERM_SESSION_ID").is_ok();
    let is_ghostty = env::var("TERM_PROGRAM")
        .map(|v| v.trim().eq_ignore_ascii_case("ghostty"))
        .unwrap_or(false)
        || env::var("GHOSTTY_RESOURCES_DIR")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
    let ghostty_cap_enum = if is_ghostty && !is_tmux && !is_ssh {
        probe_ghostty(home)
    } else {
        GhosttyCap::None
    };
    let ghostty_cap = ghostty_cap_enum.as_str();
    let ghostty_native = ghostty_cap_enum == GhosttyCap::MacOsAppleScript;

    let backend_tier = if cfg!(windows) {
        if is_wezterm {
            "Tier 1 (Win) · WezTerm Native Split CLI".to_string()
        } else if is_wt {
            "Tier 2 (Win) · Windows Terminal (wt.exe) 3-Column".to_string()
        } else {
            "Windows · Launch via WezTerm or Windows Terminal (wt.exe)".to_string()
        }
    } else if !is_ssh && !is_tmux && is_iterm {
        "Tier 1 · Local iTerm2 Metal Daemon (< 5ms split)".to_string()
    } else if is_ssh && !is_tmux && is_iterm {
        "Tier 2 · SSH + Client iTerm2 OSC 1337 Bridge".to_string()
    } else if is_wezterm && !is_tmux {
        "WezTerm · Native CLI Split Engine".to_string()
    } else if is_ghostty && !is_tmux && !is_ssh {
        if ghostty_native {
            "Ghostty · Native Metal Split Engine + Kitty APC Sprites".to_string()
        } else if cfg!(target_os = "macos") {
            "Ghostty · Tier 3 tmux Engine + Kitty Sprites (Ghostty 1.3+ enables native splits)"
                .to_string()
        } else {
            "Ghostty · Tier 3 tmux Engine + Kitty APC Sprites".to_string()
        }
    } else {
        "Tier 3 · Universal tmux 3-Column Engine".to_string()
    };

    let mut items = Vec::new();
    let mut missing_pkgs: Vec<&'static str> = Vec::new();
    let is_macos = cfg!(target_os = "macos");

    probe_claude_and_orchestrator(home, project_root, &mut items);
    let legacy_patch = probe_legacy_patch_proto(home, &mut items);

    if cfg!(windows) {
        let wez = find_executable("wezterm", home);
        let wt = find_executable("wt", home);
        let (st, det) = match (wez.is_some(), wt.is_some()) {
            (true, true) => (CheckStatus::Ok, "WezTerm + Windows Terminal (wt.exe) ready"),
            (true, false) => (CheckStatus::Ok, "WezTerm CLI split engine ready"),
            (false, true) => (
                CheckStatus::Ok,
                "Windows Terminal (wt.exe) split engine ready",
            ),
            (false, false) => {
                missing_pkgs.push("Microsoft.WindowsTerminal");
                (
                    CheckStatus::Warn,
                    "install Windows Terminal (wt.exe) or WezTerm",
                )
            }
        };
        items.push(item(st, "Split Engine", det.to_string()));
    } else if is_macos && is_iterm {
        probe_macos_iterm(home, &mut items);
    } else if is_wezterm && !is_tmux {
        let wv = find_executable("wezterm", home)
            .and_then(|b| cmd_first_line(&b, &["--version"]))
            .unwrap_or_else(|| "wezterm".to_string());
        items.push(item(
            CheckStatus::Ok,
            "Split Engine",
            format!("{wv} (native CLI split engine)"),
        ));
    } else if is_ghostty && !is_tmux && !is_ssh {
        let det = if ghostty_native {
            "Ghostty · Native Metal Split Engine + Kitty APC Sprites".to_string()
        } else if cfg!(target_os = "macos") {
            "Ghostty · Tier 3 tmux Engine + Kitty Sprites (Ghostty 1.3+ enables native splits)"
                .to_string()
        } else {
            format!("Ghostty ({ghostty_cap}) · Tier 3 tmux Engine + Kitty APC Sprites")
        };
        let status = if ghostty_native || find_executable("tmux", home).is_some() {
            CheckStatus::Ok
        } else {
            missing_pkgs.push("tmux");
            CheckStatus::Warn
        };
        items.push(item(status, "Split Engine", det));
    } else if let Some(tbin) = find_executable("tmux", home) {
        let tv = cmd_first_line(&tbin, &["-V"]).unwrap_or_else(|| "tmux".to_string());
        items.push(item(
            CheckStatus::Ok,
            "Split Engine",
            format!("{tv} (universal 3-column tiling)"),
        ));
    } else {
        missing_pkgs.push("tmux");
        items.push(item(
            CheckStatus::Warn,
            "Split Engine",
            "tmux not found (required outside local iTerm2/WezTerm)".to_string(),
        ));
    }

    probe_companion_tools(home, &mut items, &mut missing_pkgs);

    let joined = missing_pkgs.join(" ");
    let missing_hint = if missing_pkgs.is_empty() {
        "All companion tools & optional renderers are installed.".to_string()
    } else if cfg!(windows) {
        format!("Optional/missing tools: winget install {joined}")
    } else if is_macos {
        format!("Optional/missing tools: brew install {joined}")
    } else {
        format!("Missing packages: {}", missing_pkgs.join(", "))
    };

    DoctorReport {
        os_arch,
        backend_tier,
        ipc_dir: ipc_dir_s,
        ghostty_cap,
        legacy_patch,
        items,
        missing_hint,
    }
}

pub fn run_doctor(args: &[String]) {
    let home = user_home_dir();
    let json_mode = args.iter().skip(2).any(|a| a == "--json" || a == "-j");
    let root_arg = args
        .iter()
        .skip(2)
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str());
    let root = resolve_project_root(root_arg, &home);
    let report = collect_doctor_report(&home, &root);
    if json_mode {
        println!("{}", format_doctor_json(&report));
    } else {
        print_doctor_tui(&report, &home);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_valid_doctor_json_schema() {
        let rep = DoctorReport {
            os_arch: "windows/x86_64".to_string(),
            backend_tier: "Tier 1 (Win) · WezTerm".to_string(),
            ipc_dir: "C:/Temp/.cc-ide".to_string(),
            ghostty_cap: "none",
            legacy_patch: false,
            items: vec![item(
                CheckStatus::Ok,
                "Claude Code",
                "2.1.323 \"ok\"".to_string(),
            )],
            missing_hint: "All good.".to_string(),
        };
        let js = format_doctor_json(&rep);
        assert!(js.contains("\"schema_version\": 1"));
        assert!(js.contains("\"healthy\": true"));
        assert!(js.contains("2.1.323 \\\"ok\\\""));
        assert!(js.contains("\"ipc_dir\": \"C:/Temp/.cc-ide\""));
        assert!(js.contains("\"ipc_proto\": 1"));
        assert!(js.contains("\"patch_proto\": 1"));
        assert!(js.contains("\"ghostty_cap\": \"none\""));
        assert!(js.contains("\"legacy_patch\": false"));
    }
}
