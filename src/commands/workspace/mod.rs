//! `ccide-core workspace`: Adaptive Cross-Platform Tiling Workspace & Theme Controller (`src/commands/workspace/`).

pub mod actions;
pub mod iterm;
pub mod open;
pub mod tmux;
pub mod wezterm;
pub mod wt;

use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::platform::{cache_dir, ipc_dir, ipc_path, user_home_dir};

use actions::{run_clear_subagents, run_theme_command};
use iterm::{emit_osc1337, ensure_daemon_started, find_python_bin, send_daemon_ipc};
use open::handle_open;
use wezterm::{activate_wezterm_center, close_wezterm_panes};
use wt::{close_wt_panes, focus_wt_center_from_sidebar};

pub(crate) fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Fast, non-blocking Center-Pane Focus & Subagent Switcher used by `ccide-core sidebar`.
pub fn focus_center_after_view_switch(
    home: &str,
    tab_yazi_id: &str,
    session_id: &str,
    view_id: &str,
    footer_rank: usize,
) {
    let pane_spec =
        fs::read_to_string(ipc_path(&format!("panes_{}.txt", tab_yazi_id))).unwrap_or_default();
    let spec = pane_spec.trim();

    if let Some(rest) = spec.strip_prefix("tmux:") {
        let center_pane = rest.split(':').next().unwrap_or("");
        if !center_pane.is_empty() {
            let _ = Command::new("tmux")
                .args(["select-pane", "-t", center_pane])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            return;
        }
    }
    if let Some(rest) = spec.strip_prefix("wezterm:") {
        activate_wezterm_center(home, rest.split(':').next().unwrap_or(""));
        return;
    }
    if spec.starts_with("wt:") {
        focus_wt_center_from_sidebar(home);
        return;
    }

    let in_ssh = env::var("SSH_CONNECTION").is_ok() || env::var("SSH_TTY").is_ok();
    if spec.starts_with("osc:") || (in_ssh && env::var("TMUX").is_err()) {
        let payload = format!(
            "{{\"op\":\"focus\",\"yid\":\"{}\",\"sid\":\"{}\",\"view\":\"{}\"}}",
            escape_json(tab_yazi_id),
            escape_json(session_id),
            escape_json(view_id)
        );
        let _ = emit_osc1337(&payload);
        return;
    }

    let req = format!(
        "{{\"op\":\"switch-subagent\",\"yazi_id\":\"{}\",\"session_id\":\"{}\",\"view_id\":\"{}\",\"footer_rank\":{}}}",
        escape_json(tab_yazi_id),
        escape_json(session_id),
        escape_json(view_id),
        footer_rank
    );
    if send_daemon_ipc(&req, 150).is_some() {
        return;
    }

    ensure_daemon_started(home);
    let py = find_python_bin(home);
    let ws = format!("{}/.claude/scripts/iterm2_workspace.py", home);
    if Path::new(&py).exists() && Path::new(&ws).exists() {
        let rank_s = footer_rank.to_string();
        let _ = Command::new(&py)
            .args([
                ws.as_str(),
                "switch-subagent",
                tab_yazi_id,
                session_id,
                view_id,
                rank_s.as_str(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

pub fn run(args: &[String]) {
    let home = user_home_dir();
    let cdir = cache_dir(&home);
    let _ = fs::create_dir_all(&cdir);
    let _ = fs::create_dir_all(ipc_dir());
    let ws = format!("{}/.claude/scripts/iterm2_workspace.py", home);

    match args.get(2).map(|s| s.as_str()).unwrap_or("open") {
        "open" => handle_open(&home, &cdir, args),
        "close" => {
            let arg_spec = args.get(3).map(|s| s.as_str()).unwrap_or("");
            let yazi_id = args.get(4).map(|s| s.as_str()).unwrap_or("");
            let spec = if !arg_spec.is_empty() {
                arg_spec.to_string()
            } else if !yazi_id.is_empty() {
                fs::read_to_string(ipc_path(&format!("panes_{}.txt", yazi_id)))
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            } else {
                String::new()
            };
            close_wt_panes(yazi_id);
            if let Some(rest) = spec.strip_prefix("tmux:") {
                for sp in rest
                    .split(':')
                    .nth(1)
                    .unwrap_or(rest)
                    .split(',')
                    .filter(|s| !s.is_empty())
                {
                    let _ = Command::new("tmux")
                        .args(["kill-pane", "-t", sp])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            } else if spec.starts_with("wezterm:") {
                close_wezterm_panes(&home, &spec);
            } else if let Some(yid) = spec.strip_prefix("wt:") {
                close_wt_panes(yid);
            } else if let Some(yid) = spec.strip_prefix("osc:") {
                let _ = emit_osc1337(&format!(
                    "{{\"op\":\"close\",\"yid\":\"{}\"}}",
                    escape_json(yid)
                ));
            } else if !cfg!(windows) && !spec.is_empty() {
                let raw_ids = spec.strip_prefix("iterm:").unwrap_or(&spec);
                let req = format!(
                    "{{\"op\":\"close\",\"pane_ids\":\"{}\",\"caller_guid\":\"{}\"}}",
                    escape_json(raw_ids),
                    escape_json(&env::var("ITERM_SESSION_ID").unwrap_or_default())
                );
                if send_daemon_ipc(&req, 1000).is_none() {
                    let _ = Command::new(find_python_bin(&home))
                        .args([ws.as_str(), "close", raw_ids])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
        }
        "rebalance" => {
            if env::var("TMUX").map(|s| !s.is_empty()).unwrap_or(false) {
                let _ = Command::new("tmux")
                    .args(["set-option", "-w", "mouse", "on"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                return;
            }
            if !cfg!(windows) && send_daemon_ipc("{\"op\":\"rebalance\"}", 1500).is_none() {
                ensure_daemon_started(&home);
                let _ = Command::new(find_python_bin(&home))
                    .args([ws.as_str(), "rebalance"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        "focus" => {
            let query = args.get(3).map(|s| s.as_str()).unwrap_or("");
            if !cfg!(windows) && !query.is_empty() {
                let req = format!("{{\"op\":\"focus\",\"query\":\"{}\"}}", escape_json(query));
                if send_daemon_ipc(&req, 500).is_none() {
                    let _ = Command::new(find_python_bin(&home))
                        .args([ws.as_str(), "focus", query])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
        }
        "switch-subagent" => focus_center_after_view_switch(
            &home,
            args.get(3).map(|s| s.as_str()).unwrap_or(""),
            args.get(4).map(|s| s.as_str()).unwrap_or(""),
            args.get(5).map(|s| s.as_str()).unwrap_or("main"),
            args.get(6)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0),
        ),
        "clear-subagents" | "clear-done" => {
            run_clear_subagents(&home, args.get(3).cloned().unwrap_or_default())
        }
        "theme" | "palette" => {
            run_theme_command(&home, args.get(3).map(|s| s.as_str()).unwrap_or("list"))
        }
        _ => {}
    }
}
