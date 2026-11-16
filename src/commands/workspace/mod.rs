//! `ccide-core workspace`: Adaptive Cross-Platform Tiling Workspace & Theme Controller (`src/commands/workspace/`).

pub mod actions;
pub mod detect;
pub mod ghostty;
pub mod ghostty_linux;
pub mod ghostty_script;
pub mod iterm;
pub mod layout;
pub mod open;
pub mod rebalance;
pub mod reload;
pub mod spec;
pub mod spec_io;
pub mod tmux;
pub mod wezterm;
pub mod wezterm_rebalance;
pub mod wt;

use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::platform::{cache_dir, ipc_dir, user_home_dir};

use actions::{run_clear_subagents, run_theme_command};
use ghostty::{activate_ghostty_center, close_ghostty_panes, probe_ghostty_native_capable};
use iterm::{
    close_iterm_panes, emit_osc1337, ensure_daemon_started, find_python_bin, send_daemon_ipc,
};
use open::handle_open;
use spec::{read_pane_spec, remove_tab_state, Backend, PaneSpec};
use tmux::{close_tmux, focus_tmux_center};
use wezterm::{activate_wezterm_center, close_wezterm_panes};
use wt::{close_wt_panes, focus_wt_center_from_sidebar};

pub(crate) use crate::json::json_escape as escape_json;

/// Fast, non-blocking Center-Pane Focus & Subagent Switcher used by `ccide-core sidebar`.
pub fn focus_center_after_view_switch(
    home: &str,
    tab_yazi_id: &str,
    session_id: &str,
    view_id: &str,
    footer_rank: usize,
) {
    if let Some(spec) = read_pane_spec(tab_yazi_id) {
        match spec.backend {
            Backend::Tmux => {
                focus_tmux_center(&spec);
                return;
            }
            Backend::WezTerm => {
                activate_wezterm_center(home, &spec);
                return;
            }
            Backend::Ghostty => {
                activate_ghostty_center(&spec);
                return;
            }
            Backend::Wt => {
                focus_wt_center_from_sidebar(home);
                return;
            }
            Backend::Osc => {
                let payload = format!(
                    "{{\"op\":\"focus\",\"yid\":\"{}\",\"sid\":\"{}\",\"view\":\"{}\"}}",
                    escape_json(tab_yazi_id),
                    escape_json(session_id),
                    escape_json(view_id)
                );
                let _ = emit_osc1337(&payload);
                return;
            }
            Backend::Iterm => {}
        }
    } else {
        let in_ssh = env::var("SSH_CONNECTION").is_ok() || env::var("SSH_TTY").is_ok();
        if in_ssh && env::var("TMUX").is_err() {
            let payload = format!(
                "{{\"op\":\"focus\",\"yid\":\"{}\",\"sid\":\"{}\",\"view\":\"{}\"}}",
                escape_json(tab_yazi_id),
                escape_json(session_id),
                escape_json(view_id)
            );
            let _ = emit_osc1337(&payload);
            return;
        }
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
        if let Ok(mut child) = Command::new(&py)
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
            .spawn()
        {
            let _ = std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
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
            let arg_spec = args.get(3).map(|s| s.trim()).unwrap_or("");
            let yazi_id = args.get(4).map(|s| s.trim()).unwrap_or("");
            let parsed_spec = if !arg_spec.is_empty() {
                PaneSpec::parse(arg_spec, yazi_id)
            } else if !yazi_id.is_empty() {
                read_pane_spec(yazi_id)
            } else {
                None
            };
            close_wt_panes(yazi_id);
            if let Some(spec) = parsed_spec {
                let eff_yid = if !yazi_id.is_empty() {
                    yazi_id.to_string()
                } else {
                    spec.yid.clone()
                };
                match spec.backend {
                    Backend::Tmux => close_tmux(&spec),
                    Backend::WezTerm => close_wezterm_panes(&home, &spec),
                    Backend::Ghostty => close_ghostty_panes(&spec),
                    Backend::Wt => close_wt_panes(&eff_yid),
                    Backend::Osc => {
                        let _ = emit_osc1337(&format!(
                            "{{\"op\":\"close\",\"yid\":\"{}\"}}",
                            escape_json(&eff_yid)
                        ));
                    }
                    Backend::Iterm => {
                        let caller = env::var("ITERM_SESSION_ID").unwrap_or_default();
                        close_iterm_panes(&home, &caller, &spec);
                    }
                }
                if !eff_yid.is_empty() {
                    remove_tab_state(&eff_yid);
                }
            } else if !yazi_id.is_empty() {
                remove_tab_state(yazi_id);
            }
        }
        "rebalance" => {
            let code = rebalance::run_rebalance(&home, args.get(3).map(|s| s.as_str()));
            std::process::exit(code);
        }
        "detect" => detect::run_detect(&home, args),
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
        "probe-ghostty" => {
            std::process::exit(if probe_ghostty_native_capable(&home) {
                0
            } else {
                1
            });
        }
        _ => {}
    }
}
