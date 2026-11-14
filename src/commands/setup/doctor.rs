//! Environment, Companion Tool & Binary Patch Health Diagnostics (`ccide-core doctor [--json]`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::commands::setup::doctor_render::{format_doctor_json, print_doctor_tui};
use crate::json::{extract_json_bool, extract_json_str};
use crate::platform::{clean_canonicalize, find_executable, ipc_path, user_home_dir};
use crate::tui::width::trunc_to_width;

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
    pub items: Vec<DoctorItem>,
    pub missing_hint: String,
}

fn cmd_first_line(bin: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    let txt = if !out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    txt.lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(String::from)
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
    let is_ssh = env::var("SSH_CONNECTION").is_ok() || env::var("SSH_TTY").is_ok();
    let is_tmux = env::var("TMUX").is_ok();
    let is_wezterm =
        env::var("WEZTERM_PANE").is_ok() || env::var("TERM_PROGRAM").as_deref() == Ok("WezTerm");
    let is_wt = env::var("WT_SESSION").is_ok();
    let is_iterm = env::var("TERM_PROGRAM").as_deref() == Ok("iTerm.app")
        || env::var("LC_TERMINAL").as_deref() == Ok("iTerm2")
        || env::var("ITERM_SESSION_ID").is_ok();

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
    } else {
        "Tier 3 · Universal tmux 3-Column Engine".to_string()
    };

    let mut items = Vec::new();
    let mut missing_pkgs: Vec<&str> = Vec::new();
    let is_macos = cfg!(target_os = "macos");

    let claude_bin = find_executable("claude", home);
    let patcher_py = [
        project_root.join("scripts/patch_claude_binary.py"),
        Path::new(home).join(".claude/scripts/patch_claude_binary.py"),
    ]
    .into_iter()
    .find(|p| p.is_file());
    let python_bin = find_executable("python3", home).or_else(|| find_executable("python", home));

    match (&claude_bin, &patcher_py, &python_bin) {
        (Some(cbin), Some(ppy), Some(py)) => {
            let ver = cmd_first_line(cbin, &["--version"]).unwrap_or_else(|| "found".to_string());
            let req_ok = Command::new(py)
                .args(["-I", ppy.to_str().unwrap_or(""), "--verify"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .ok()
                .and_then(|o| extract_json_bool(&String::from_utf8_lossy(&o.stdout), "required_ok"))
                .unwrap_or(false);
            items.push(DoctorItem {
                status: if req_ok {
                    CheckStatus::Ok
                } else {
                    CheckStatus::Warn
                },
                label: "Claude Code".to_string(),
                detail: if req_ok {
                    format!("{ver} · structural anchors verified")
                } else {
                    format!("{ver} · patch anchors need update")
                },
            });
        }
        (Some(cbin), _, _) => items.push(DoctorItem {
            status: CheckStatus::Ok,
            label: "Claude Code".to_string(),
            detail: cmd_first_line(cbin, &["--version"]).unwrap_or_else(|| "found".to_string()),
        }),
        (None, _, _) => items.push(DoctorItem {
            status: CheckStatus::Warn,
            label: "Claude Code".to_string(),
            detail: "not found (npm i -g @anthropic-ai/claude-code)".to_string(),
        }),
    }

    let settings_txt =
        fs::read_to_string(Path::new(home).join(".claude/settings.json")).unwrap_or_default();
    let locked_vars: Vec<&str> = [
        "CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL",
        "CLAUDE_CODE_SUBAGENT_MODEL_FORCE",
    ]
    .into_iter()
    .filter(|k| {
        env::var(k).map(|v| !v.trim().is_empty()).unwrap_or(false)
            || extract_json_str(&settings_txt, k)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
    })
    .collect();
    if !locked_vars.is_empty() {
        items.push(DoctorItem {
            status: CheckStatus::Warn,
            label: "Orchestrator".to_string(),
            detail: format!(
                "{} set (locks Workflow per-agent model overrides)",
                locked_vars.join(", ")
            ),
        });
    }

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
        items.push(DoctorItem {
            status: st,
            label: "Split Engine".to_string(),
            detail: det.to_string(),
        });
    } else if is_macos && !is_ssh && !is_tmux {
        let iterm_app = [
            PathBuf::from("/Applications/iTerm.app"),
            Path::new(home).join("Applications/iTerm.app"),
        ]
        .into_iter()
        .find(|p| p.is_dir());
        let sock_live = ipc_path("iterm_daemon.sock").exists();
        let pyenv_py = Path::new(home).join(".local/share/cc-ide/pyenv/bin/python3");
        let rec_py = fs::read_to_string(Path::new(home).join(".cache/cc-ide/python_bin"))
            .ok()
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| p.is_file());
        let py_cand = if pyenv_py.is_file() {
            Some(pyenv_py)
        } else {
            rec_py
        };
        let py_iterm_ok = py_cand
            .as_ref()
            .and_then(|py| {
                Command::new(py)
                    .args(["-I", "-c", "import iterm2"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .ok()
            })
            .map(|s| s.success())
            .unwrap_or(false);

        if sock_live {
            items.push(DoctorItem {
                status: CheckStatus::Ok,
                label: "iTerm2 Daemon".to_string(),
                detail: "warm socket active (/tmp/.cc-ide/iterm_daemon.sock)".to_string(),
            });
        } else if iterm_app.is_some() && py_iterm_ok {
            items.push(DoctorItem {
                status: CheckStatus::Info,
                label: "iTerm2 Daemon".to_string(),
                detail: "iterm2 venv ready · socket starts on install / launch".to_string(),
            });
        } else if iterm_app.is_some() {
            items.push(DoctorItem {
                status: CheckStatus::Warn,
                label: "iTerm2 API".to_string(),
                detail: "run ./install.sh to provision ~/.local/share/cc-ide/pyenv + API"
                    .to_string(),
            });
        } else if let Some(tbin) = find_executable("tmux", home) {
            let tv = cmd_first_line(&tbin, &["-V"]).unwrap_or_else(|| "tmux".to_string());
            items.push(DoctorItem {
                status: CheckStatus::Ok,
                label: "Split Engine".to_string(),
                detail: format!("{tv} (iTerm2 not found · Tier 3 active)"),
            });
        }
    } else if let Some(tbin) = find_executable("tmux", home) {
        let tv = cmd_first_line(&tbin, &["-V"]).unwrap_or_else(|| "tmux".to_string());
        items.push(DoctorItem {
            status: CheckStatus::Ok,
            label: "Split Engine".to_string(),
            detail: format!("{tv} (universal 3-column tiling)"),
        });
    } else {
        missing_pkgs.push("tmux");
        items.push(DoctorItem {
            status: CheckStatus::Warn,
            label: "Split Engine".to_string(),
            detail: "tmux not found (required outside local iTerm2/WezTerm)".to_string(),
        });
    }

    match (find_executable("yazi", home), find_executable("ya", home)) {
        (Some(yb), Some(_)) => items.push(DoctorItem {
            status: CheckStatus::Ok,
            label: "Yazi + ya".to_string(),
            detail: cmd_first_line(&yb, &["--version"]).unwrap_or_else(|| "installed".to_string()),
        }),
        _ => {
            missing_pkgs.push(if cfg!(windows) { "sxyazi.yazi" } else { "yazi" });
            items.push(DoctorItem {
                status: CheckStatus::Warn,
                label: "Yazi + ya".to_string(),
                detail: "missing (left-top file tree pane)".to_string(),
            });
        }
    }

    if let Some(lgb) = find_executable("lazygit", home) {
        let lv = cmd_first_line(&lgb, &["--version"]).unwrap_or_else(|| "installed".to_string());
        let short_lv = lv
            .split(',')
            .find(|s| s.trim_start().starts_with("version="))
            .map(|s| format!("lazygit {}", s.trim().trim_start_matches("version=")))
            .unwrap_or(lv);
        items.push(DoctorItem {
            status: CheckStatus::Ok,
            label: "Lazygit".to_string(),
            detail: trunc_to_width(&short_lv, 44),
        });
    } else {
        missing_pkgs.push(if cfg!(windows) {
            "JesseDuffield.lazygit"
        } else {
            "lazygit"
        });
        items.push(DoctorItem {
            status: CheckStatus::Warn,
            label: "Lazygit".to_string(),
            detail: "missing (left-bottom Git diff/graph pane)".to_string(),
        });
    }

    let has_delta = find_executable("delta", home).is_some();
    let has_glow = find_executable("glow", home).is_some();
    if !has_delta {
        missing_pkgs.push(if cfg!(windows) {
            "dandavison.delta"
        } else {
            "git-delta"
        });
    }
    if !has_glow {
        missing_pkgs.push(if cfg!(windows) {
            "charmbracelet.glow"
        } else {
            "glow"
        });
    }
    items.push(DoctorItem {
        status: if has_delta && has_glow {
            CheckStatus::Ok
        } else {
            CheckStatus::Info
        },
        label: "Delta & Glow".to_string(),
        detail: match (has_delta, has_glow) {
            (true, true) => "delta + glow active",
            (true, false) => "delta active · glow using Python fallback",
            (false, true) => "glow active · diff using Python fallback",
            (false, false) => "using built-in zero-dep Python ANSI renderers",
        }
        .to_string(),
    });

    let missing_hint = if missing_pkgs.is_empty() {
        "All companion tools & optional renderers are installed.".to_string()
    } else if cfg!(windows) {
        format!(
            "Optional/missing tools: winget install {}",
            missing_pkgs.join(" ")
        )
    } else if is_macos {
        format!(
            "Optional/missing tools: brew install {}",
            missing_pkgs.join(" ")
        )
    } else {
        format!("Missing packages: {}", missing_pkgs.join(", "))
    };

    DoctorReport {
        os_arch,
        backend_tier,
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
            items: vec![DoctorItem {
                status: CheckStatus::Ok,
                label: "Claude Code".to_string(),
                detail: "2.1.323 \"ok\"".to_string(),
            }],
            missing_hint: "All good.".to_string(),
        };
        let js = format_doctor_json(&rep);
        assert!(js.contains("\"schema_version\": 1"));
        assert!(js.contains("\"healthy\": true"));
        assert!(js.contains("2.1.323 \\\"ok\\\""));
    }
}
