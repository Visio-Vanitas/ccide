//! Environment, Companion Tool & Binary Patch Health Diagnostics (`ccide-core doctor [--json]`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::commands::setup::doctor_render::{format_doctor_json, print_doctor_tui};
use crate::json::{extract_json_bool, extract_json_str};
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

fn find_in_path(bin_name: &str, home: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]
        .iter()
        .map(PathBuf::from)
        .collect();
    if !home.is_empty() {
        for rel in [".local/bin", ".cargo/bin", ".claude/bin"] {
            dirs.push(Path::new(home).join(rel));
        }
        if let Ok(rd) = fs::read_dir(Path::new(home).join(".nvm/versions/node")) {
            for e in rd.flatten() {
                dirs.push(e.path().join("bin"));
            }
        }
    }
    if let Ok(path_var) = env::var("PATH") {
        dirs.extend(
            path_var
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        );
    }
    dirs.into_iter()
        .map(|d| d.join(bin_name))
        .find(|cand| cand.is_file())
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

pub fn resolve_project_root(explicit_arg: Option<&str>, home: &str) -> PathBuf {
    if let Some(s) = explicit_arg {
        let t = s.trim();
        if !t.is_empty() && !t.starts_with('-') {
            let p = PathBuf::from(t);
            if p.join("install.sh").is_file() {
                return p;
            }
        }
    }
    if let Ok(exe) = env::current_exe().and_then(fs::canonicalize) {
        if let Some(parent) = exe.parent().and_then(|p| p.parent()) {
            if parent.join("install.sh").is_file() {
                return parent.to_path_buf();
            }
        }
    }
    if let Ok(cwd) = env::current_dir() {
        if cwd.join("install.sh").is_file() {
            return cwd;
        }
    }
    if let Ok(txt) = fs::read_to_string(Path::new(home).join(".claude/settings.json")) {
        if let Some(p) = extract_json_str(&txt, "path") {
            let pb = PathBuf::from(&p);
            if pb.join("install.sh").is_file() {
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
    let is_iterm = env::var("TERM_PROGRAM")
        .map(|v| v == "iTerm.app")
        .unwrap_or(false)
        || env::var("LC_TERMINAL")
            .map(|v| v == "iTerm2")
            .unwrap_or(false)
        || env::var("ITERM_SESSION_ID").is_ok();

    let backend_tier = if !is_ssh && !is_tmux && is_iterm {
        "Tier 1 · Local iTerm2 Metal Daemon (< 5ms split)".to_string()
    } else if is_ssh && !is_tmux && is_iterm {
        "Tier 2 · SSH + Client iTerm2 OSC 1337 Bridge".to_string()
    } else {
        "Tier 3 · Universal tmux 3-Column Engine".to_string()
    };

    let mut items = Vec::new();
    let mut missing_pkgs: Vec<&str> = Vec::new();
    let is_macos = cfg!(target_os = "macos");

    let claude_bin = find_in_path("claude", home);
    let patcher_py = [
        project_root.join("scripts/patch_claude_binary.py"),
        Path::new(home).join(".claude/scripts/patch_claude_binary.py"),
    ]
    .into_iter()
    .find(|p| p.is_file());
    let python_bin = find_in_path("python3", home);

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

    if is_macos && !is_ssh && !is_tmux {
        let iterm_app = [
            PathBuf::from("/Applications/iTerm.app"),
            Path::new(home).join("Applications/iTerm.app"),
        ]
        .into_iter()
        .find(|p| p.is_dir());
        let sock_live = Path::new("/tmp/.cc-ide/iterm_daemon.sock").exists();
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
        } else if let Some(tbin) = find_in_path("tmux", home) {
            let tv = cmd_first_line(&tbin, &["-V"]).unwrap_or_else(|| "tmux".to_string());
            items.push(DoctorItem {
                status: CheckStatus::Ok,
                label: "Split Engine".to_string(),
                detail: format!("{tv} (iTerm2 not found · Tier 3 active)"),
            });
        }
    } else if let Some(tbin) = find_in_path("tmux", home) {
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
            detail: "tmux not found (required outside local iTerm2)".to_string(),
        });
    }

    match (find_in_path("yazi", home), find_in_path("ya", home)) {
        (Some(yb), Some(_)) => items.push(DoctorItem {
            status: CheckStatus::Ok,
            label: "Yazi + ya".to_string(),
            detail: cmd_first_line(&yb, &["--version"]).unwrap_or_else(|| "installed".to_string()),
        }),
        _ => {
            missing_pkgs.push("yazi");
            items.push(DoctorItem {
                status: CheckStatus::Warn,
                label: "Yazi + ya".to_string(),
                detail: "missing (left-top file tree pane)".to_string(),
            });
        }
    }

    if let Some(lgb) = find_in_path("lazygit", home) {
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
        missing_pkgs.push("lazygit");
        items.push(DoctorItem {
            status: CheckStatus::Warn,
            label: "Lazygit".to_string(),
            detail: "missing (left-bottom Git diff/graph pane)".to_string(),
        });
    }

    let has_delta = find_in_path("delta", home).is_some();
    let has_glow = find_in_path("glow", home).is_some();
    if !has_delta {
        missing_pkgs.push("git-delta");
    }
    if !has_glow {
        missing_pkgs.push("glow");
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
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
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
            os_arch: "darwin/aarch64".to_string(),
            backend_tier: "Tier 1 · Local iTerm2".to_string(),
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
