//! Adaptive Multi-Backend `workspace open` Dispatcher (`src/commands/workspace/open.rs`).

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::commands::workspace::escape_json;
use crate::commands::workspace::ghostty::{
    is_ghostty_env, probe_ghostty, run_ghostty_open, GhosttyCap,
};
use crate::commands::workspace::iterm::{
    build_iterm_pane_spec, emit_osc1337, ensure_daemon_started, find_python_bin, send_daemon_ipc,
};
use crate::commands::workspace::spec::{write_pane_spec, Backend, OpenRequest, PaneSpec};
use crate::commands::workspace::tmux::run_tmux_open;
use crate::commands::workspace::wezterm::run_wezterm_open;
use crate::commands::workspace::wt::run_wt_open;
use crate::ffi::{current_ppid, ProcToken};
use crate::platform::{atomic_write, current_username, find_executable, ipc_path};

fn finish_open(spec: PaneSpec) {
    let _ = write_pane_spec(&spec);
    println!("{}", spec.encode());
}

fn try_open_iterm(req: &OpenRequest) -> Option<PaneSpec> {
    let caller_guid = env::var("ITERM_SESSION_ID").unwrap_or_default();
    let ipc_req = format!(
        "{{\"op\":\"open\",\"caller_guid\":\"{}\",\"target_dir\":\"{}\",\"yazi_id\":\"{}\",\"show_git\":{},\"show_mon\":{},\"left_pct\":{},\"git_root\":\"{}\",\"show_right\":{},\"shell_pid\":{}}}",
        escape_json(&caller_guid),
        escape_json(req.target_dir),
        escape_json(req.yazi_id),
        req.show_git,
        req.show_mon,
        req.left_pct,
        escape_json(req.git_root),
        req.show_right,
        req.shell_pid
    );
    if let Some(resp) = send_daemon_ipc(&ipc_req, 2000) {
        if let Some(spec) = build_iterm_pane_spec(req, &caller_guid, &resp) {
            return Some(spec);
        }
    }
    ensure_daemon_started(req.home);
    let ws = format!("{}/.claude/scripts/iterm2_workspace.py", req.home);
    let left_pct_s = req.left_pct.to_string();
    let out = Command::new(find_python_bin(req.home))
        .args([
            ws.as_str(),
            "open",
            req.target_dir,
            req.yazi_id,
            if req.show_git { "1" } else { "0" },
            if req.show_mon { "1" } else { "0" },
            left_pct_s.as_str(),
            req.git_root,
            if req.show_right { "1" } else { "0" },
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    build_iterm_pane_spec(
        req,
        &caller_guid,
        String::from_utf8_lossy(&out.stdout).trim(),
    )
}

fn try_open_osc(req: &OpenRequest) -> Option<PaneSpec> {
    let ack_path = ipc_path(&format!("osc_ack_{}", req.yazi_id));
    let _ = fs::remove_file(&ack_path);
    let payload = format!(
        "{{\"op\":\"open_ssh\",\"dir\":\"{}\",\"git_root\":\"{}\",\"yid\":\"{}\",\"git\":{},\"mon\":{},\"left_pct\":{},\"right\":{},\"shell_pid\":{},\"user\":\"{}\",\"ssh_conn\":\"{}\"}}",
        escape_json(req.target_dir),
        escape_json(req.git_root),
        escape_json(req.yazi_id),
        usize::from(req.show_git),
        usize::from(req.show_mon),
        req.left_pct,
        usize::from(req.show_right),
        req.shell_pid,
        escape_json(&current_username()),
        escape_json(&env::var("SSH_CONNECTION").unwrap_or_default())
    );
    if !emit_osc1337(&payload) {
        return None;
    }
    let deadline = Instant::now() + Duration::from_millis(450);
    while Instant::now() < deadline {
        if ack_path.exists() {
            return Some(PaneSpec {
                backend: Backend::Osc,
                yid: req.yazi_id.to_string(),
                center: String::new(),
                yazi: None,
                lazygit: None,
                mon: None,
                sidebar: None,
                left_pct: req.left_pct,
                show_git: req.show_git,
                show_mon: req.show_mon,
                show_right: req.show_right,
                legacy_sides: Vec::new(),
                legacy: false,
            });
        }
        thread::sleep(Duration::from_millis(15));
    }
    None
}

pub fn handle_open(home: &str, cdir: &Path, args: &[String]) {
    let target_dir = args.get(3).map(|s| s.as_str()).unwrap_or(".");
    let yazi_id = args.get(4).map(|s| s.as_str()).unwrap_or("1000");
    let show_git = args.get(5).map(|s| s == "1").unwrap_or(true);
    let show_mon = args.get(6).map(|s| s == "1").unwrap_or(false);
    let left_pct = args.get(7).and_then(|s| s.parse::<u8>().ok()).unwrap_or(30);
    let git_root = args
        .get(8)
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty() && *s != "-")
        .unwrap_or(target_dir);
    let show_right = args.get(9).map(|s| s == "1").unwrap_or(true);
    let raw_req = args
        .get(10)
        .cloned()
        .or_else(|| env::var("CCIDE_BACKEND").ok())
        .unwrap_or_else(|| "auto".to_string());
    let requested = Backend::from_name(&raw_req)
        .map(|b| b.as_str().to_string())
        .unwrap_or(raw_req);

    let shell_pid = env::var("CCIDE_SHELL_PID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(current_ppid);
    if shell_pid > 1 {
        let tok_s = ProcToken::current(shell_pid)
            .map(|t| t.encode())
            .unwrap_or_else(|| shell_pid.to_string());
        let _ = atomic_write(
            &cdir.join(format!("tab_shell_{}.txt", yazi_id)),
            tok_s.as_bytes(),
        );
    }

    let req = OpenRequest {
        home,
        target_dir,
        yazi_id,
        git_root,
        show_git,
        show_mon,
        show_right,
        left_pct,
        shell_pid,
    };

    let in_tmux = env::var("TMUX").map(|s| !s.is_empty()).unwrap_or(false);
    let in_ssh = env::var("SSH_CONNECTION").is_ok() || env::var("SSH_TTY").is_ok();
    let in_wezterm = env::var("WEZTERM_PANE")
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let in_wt = env::var("WT_SESSION")
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let is_iterm = env::var("TERM_PROGRAM").as_deref() == Ok("iTerm.app")
        || env::var("LC_TERMINAL").as_deref() == Ok("iTerm2")
        || env::var("ITERM_SESSION_ID").is_ok();

    if requested == "iterm"
        || (requested == "auto" && !cfg!(windows) && !in_ssh && !in_tmux && is_iterm)
    {
        if let Some(spec) = try_open_iterm(&req) {
            finish_open(spec);
            return;
        }
    }
    if requested == "osc" || (requested == "auto" && in_ssh && !in_tmux && is_iterm) {
        if let Some(spec) = try_open_osc(&req) {
            finish_open(spec);
            return;
        }
    }
    if requested == "wezterm" || (requested == "auto" && !in_tmux && in_wezterm) {
        if let Some(spec) = run_wezterm_open(&req) {
            finish_open(spec);
            return;
        }
    }
    if requested == "wt" || (requested == "auto" && cfg!(windows) && in_wt) {
        if let Some(spec) = run_wt_open(&req) {
            finish_open(spec);
            return;
        }
    }
    if requested == "ghostty" || (requested == "auto" && !in_ssh && !in_tmux && is_ghostty_env()) {
        let cap = probe_ghostty(home);
        if requested == "ghostty" || cap != GhosttyCap::None {
            if let Some(spec) = run_ghostty_open(&req, cap) {
                finish_open(spec);
                return;
            }
        }
    }
    if in_tmux || requested == "tmux" {
        if let Some(spec) = run_tmux_open(&req) {
            finish_open(spec);
        }
    } else if requested == "auto" {
        if find_executable("tmux", home).is_none() {
            eprintln!(
                "ccide-core: no supported terminal (iTerm2/Ghostty/WezTerm/WT) or tmux session active; \
                 run inside tmux or install tmux for 3-column tiling."
            );
        }
    } else {
        eprintln!(
            "ccide-core: requested workspace backend '{}' is unavailable in this environment.",
            requested
        );
    }
}
