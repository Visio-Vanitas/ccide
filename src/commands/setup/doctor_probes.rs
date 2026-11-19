//! Probe Helpers for `ccide-core doctor` (Claude Code, Orchestrator, iTerm2 Daemon & Companion Tools).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::commands::setup::doctor::{item, CheckStatus, DoctorItem};
use crate::ffi::is_pid_alive;
use crate::json::{extract_json_bool, extract_json_str};
use crate::platform::{find_executable, ipc_path, legacy_ipc_dir};
use crate::proto::{IPC_PROTO, PATCH_PROTO};
use crate::tui::width::trunc_to_width;

pub(super) fn cmd_first_line(bin: &Path, args: &[&str]) -> Option<String> {
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

pub(super) fn probe_claude_and_orchestrator(
    home: &str,
    project_root: &Path,
    items: &mut Vec<DoctorItem>,
) {
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
            let st = if req_ok {
                CheckStatus::Ok
            } else {
                CheckStatus::Warn
            };
            let d = if req_ok {
                format!("{ver} · structural anchors verified")
            } else {
                format!("{ver} · patch anchors need update")
            };
            items.push(item(st, "Claude Code", d));
        }
        (Some(cbin), _, _) => {
            let d = cmd_first_line(cbin, &["--version"]).unwrap_or_else(|| "found".to_string());
            items.push(item(CheckStatus::Ok, "Claude Code", d));
        }
        (None, _, _) => {
            let d = "not found (npm i -g @anthropic-ai/claude-code)".to_string();
            items.push(item(CheckStatus::Warn, "Claude Code", d));
        }
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
        let d = format!(
            "{} set (locks Workflow per-agent model overrides)",
            locked_vars.join(", ")
        );
        items.push(item(CheckStatus::Warn, "Orchestrator", d));
    }
    crate::commands::setup::doctor::doctor_agents::probe_extra_agents(home, items);
}

pub(super) fn probe_legacy_patch_proto(home: &str, items: &mut Vec<DoctorItem>) -> bool {
    let mut legacy_patch = false;
    if let Some(ldir) = legacy_ipc_dir() {
        let is_legacy_only = |pid: u32| -> bool {
            pid > 1
                && is_pid_alive(pid)
                && ldir.join(format!("cur_pid_{pid}.txt")).is_file()
                && !ipc_path(&format!("cur_pid_{pid}.txt")).is_file()
                && !ipc_path(&format!("ready_pid_{pid}")).is_file()
        };
        for se in crate::session::scan::scan_alive_sessions(home) {
            if is_legacy_only(se.pid) {
                legacy_patch = true;
                break;
            }
        }
        if !legacy_patch {
            if let Ok(rd) = fs::read_dir(&ldir) {
                for entry in rd.flatten() {
                    let fname = entry.file_name();
                    let name = fname.to_string_lossy();
                    if let Some(pid_s) = name
                        .strip_prefix("cur_pid_")
                        .and_then(|s| s.strip_suffix(".txt"))
                    {
                        if let Ok(pid) = pid_s.parse::<u32>() {
                            if is_legacy_only(pid) {
                                legacy_patch = true;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    if legacy_patch {
        items.push(item(
            CheckStatus::Warn,
            "IPC Patch Proto",
            "legacy /tmp/.cc-ide patch active · run `ccide` or `./install.sh --patch-only`"
                .to_string(),
        ));
    } else {
        items.push(item(
            CheckStatus::Ok,
            "IPC Patch Proto",
            format!("ipc_proto={IPC_PROTO} · patch_proto={PATCH_PROTO} · per-UID ready_pid"),
        ));
    }
    legacy_patch
}

pub(super) fn probe_macos_iterm(home: &str, items: &mut Vec<DoctorItem>) {
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
        let d = format!(
            "warm socket active ({})",
            ipc_path("iterm_daemon.sock").display()
        );
        items.push(item(CheckStatus::Ok, "iTerm2 Daemon", d));
    } else if iterm_app.is_some() && py_iterm_ok {
        items.push(item(
            CheckStatus::Info,
            "iTerm2 Daemon",
            "iterm2 venv ready · socket starts on install / launch".to_string(),
        ));
    } else if iterm_app.is_some() {
        items.push(item(
            CheckStatus::Warn,
            "iTerm2 API",
            "run ./install.sh to provision ~/.local/share/cc-ide/pyenv + API".to_string(),
        ));
    } else if let Some(tbin) = find_executable("tmux", home) {
        let tv = cmd_first_line(&tbin, &["-V"]).unwrap_or_else(|| "tmux".to_string());
        items.push(item(
            CheckStatus::Ok,
            "Split Engine",
            format!("{tv} (iTerm2 not found · Tier 3 active)"),
        ));
    }
}

pub(super) fn probe_companion_tools(
    home: &str,
    items: &mut Vec<DoctorItem>,
    missing_pkgs: &mut Vec<&'static str>,
) {
    match (find_executable("yazi", home), find_executable("ya", home)) {
        (Some(yb), Some(_)) => items.push(item(
            CheckStatus::Ok,
            "Yazi + ya",
            cmd_first_line(&yb, &["--version"]).unwrap_or_else(|| "installed".to_string()),
        )),
        _ => {
            missing_pkgs.push(if cfg!(windows) { "sxyazi.yazi" } else { "yazi" });
            items.push(item(
                CheckStatus::Warn,
                "Yazi + ya",
                "missing (left-top file tree pane)".to_string(),
            ));
        }
    }

    if let Some(lgb) = find_executable("lazygit", home) {
        let lv = cmd_first_line(&lgb, &["--version"]).unwrap_or_else(|| "installed".to_string());
        let short_lv = lv
            .split(',')
            .find(|s| s.trim_start().starts_with("version="))
            .map(|s| format!("lazygit {}", s.trim().trim_start_matches("version=")))
            .unwrap_or(lv);
        items.push(item(
            CheckStatus::Ok,
            "Lazygit",
            trunc_to_width(&short_lv, 44),
        ));
    } else {
        missing_pkgs.push(if cfg!(windows) {
            "JesseDuffield.lazygit"
        } else {
            "lazygit"
        });
        items.push(item(
            CheckStatus::Warn,
            "Lazygit",
            "missing (left-bottom Git diff/graph pane)".to_string(),
        ));
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
    let dg_status = if has_delta && has_glow {
        CheckStatus::Ok
    } else {
        CheckStatus::Info
    };
    let dg_detail = match (has_delta, has_glow) {
        (true, true) => "delta + glow active",
        (true, false) => "delta active · glow using Python fallback",
        (false, true) => "glow active · diff using Python fallback",
        (false, false) => "using built-in zero-dep Python ANSI renderers",
    };
    items.push(item(dg_status, "Delta & Glow", dg_detail.to_string()));
}
