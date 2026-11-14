//! Adaptive Multi-Backend `workspace open` Dispatcher (`src/commands/workspace/open.rs`).

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::commands::workspace::escape_json;
use crate::commands::workspace::iterm::{
    emit_osc1337, ensure_daemon_started, find_python_bin, send_daemon_ipc,
};
use crate::commands::workspace::tmux::run_tmux_open;
use crate::commands::workspace::wezterm::run_wezterm_open;
use crate::commands::workspace::wt::run_wt_open;
use crate::ffi::current_ppid;
use crate::platform::{current_username, find_executable, ipc_path};

pub fn handle_open(home: &str, cdir: &Path, args: &[String]) {
    let target_dir = args.get(3).map(|s| s.as_str()).unwrap_or(".");
    let yazi_id = args.get(4).map(|s| s.as_str()).unwrap_or("1000");
    let show_git = args.get(5).map(|s| s == "1").unwrap_or(true);
    let show_mon = args.get(6).map(|s| s == "1").unwrap_or(false);
    let left_pct = args
        .get(7)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(30);
    let git_root = args
        .get(8)
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty() && *s != "-")
        .unwrap_or(target_dir);
    let show_right = args.get(9).map(|s| s == "1").unwrap_or(true);
    let requested = args
        .get(10)
        .cloned()
        .or_else(|| env::var("CCIDE_BACKEND").ok())
        .unwrap_or_else(|| "auto".to_string());

    let shell_pid = env::var("CCIDE_SHELL_PID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(current_ppid);
    if shell_pid > 1 {
        let _ = fs::write(
            cdir.join(format!("tab_shell_{}.txt", yazi_id)),
            shell_pid.to_string(),
        );
    }

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
    let panes_file = ipc_path(&format!("panes_{}.txt", yazi_id));

    if requested == "iterm"
        || (requested == "auto" && !cfg!(windows) && !in_ssh && !in_tmux && is_iterm)
    {
        let caller_guid = env::var("ITERM_SESSION_ID").unwrap_or_default();
        let req = format!(
            "{{\"op\":\"open\",\"caller_guid\":\"{}\",\"target_dir\":\"{}\",\"yazi_id\":\"{}\",\"show_git\":{},\"show_mon\":{},\"left_pct\":{},\"git_root\":\"{}\",\"show_right\":{},\"shell_pid\":{}}}",
            escape_json(&caller_guid),
            escape_json(target_dir),
            escape_json(yazi_id),
            show_git,
            show_mon,
            left_pct,
            escape_json(git_root),
            show_right,
            shell_pid
        );
        if let Some(resp) = send_daemon_ipc(&req, 2000) {
            let spec = format!("iterm:{}", resp);
            let _ = fs::write(&panes_file, &spec);
            println!("{}", spec);
            return;
        }
        ensure_daemon_started(home);
        let ws = format!("{}/.claude/scripts/iterm2_workspace.py", home);
        let left_pct_s = left_pct.to_string();
        if let Ok(out) = Command::new(find_python_bin(home))
            .args([
                ws.as_str(),
                "open",
                target_dir,
                yazi_id,
                if show_git { "1" } else { "0" },
                if show_mon { "1" } else { "0" },
                left_pct_s.as_str(),
                git_root,
                if show_right { "1" } else { "0" },
            ])
            .output()
        {
            if out.status.success() {
                let spec = format!("iterm:{}", String::from_utf8_lossy(&out.stdout).trim());
                let _ = fs::write(&panes_file, &spec);
                println!("{}", spec);
                return;
            }
        }
    }

    if requested == "osc" || (requested == "auto" && in_ssh && !in_tmux && is_iterm) {
        let ack_path = ipc_path(&format!("osc_ack_{}", yazi_id));
        let _ = fs::remove_file(&ack_path);
        let payload = format!(
            "{{\"op\":\"open_ssh\",\"dir\":\"{}\",\"git_root\":\"{}\",\"yid\":\"{}\",\"git\":{},\"mon\":{},\"left_pct\":{},\"right\":{},\"shell_pid\":{},\"user\":\"{}\",\"ssh_conn\":\"{}\"}}",
            escape_json(target_dir),
            escape_json(git_root),
            escape_json(yazi_id),
            usize::from(show_git),
            usize::from(show_mon),
            left_pct,
            usize::from(show_right),
            shell_pid,
            escape_json(&current_username()),
            escape_json(&env::var("SSH_CONNECTION").unwrap_or_default())
        );
        if emit_osc1337(&payload) {
            let deadline = Instant::now() + Duration::from_millis(450);
            while Instant::now() < deadline {
                if ack_path.exists() {
                    let spec = format!("osc:{}", yazi_id);
                    let _ = fs::write(&panes_file, &spec);
                    println!("{}", spec);
                    return;
                }
                thread::sleep(Duration::from_millis(15));
            }
        }
    }

    if requested == "wezterm" || (requested == "auto" && !in_tmux && in_wezterm) {
        if let Some(spec) = run_wezterm_open(
            home, target_dir, yazi_id, show_git, left_pct, git_root, show_right, shell_pid,
        ) {
            println!("{}", spec);
            return;
        }
    }
    if requested == "wt" || (requested == "auto" && cfg!(windows) && in_wt) {
        if let Some(spec) = run_wt_open(
            home, target_dir, yazi_id, show_git, left_pct, git_root, show_right, shell_pid,
        ) {
            println!("{}", spec);
            return;
        }
    }
    if in_tmux || requested == "tmux" {
        if let Some(spec) = run_tmux_open(
            home, target_dir, yazi_id, show_git, show_mon, left_pct, git_root, show_right,
            shell_pid,
        ) {
            println!("{}", spec);
        }
    } else if requested == "auto" {
        if find_executable("tmux", home).is_some() {
            if let Some(spec) = run_tmux_open(
                home, target_dir, yazi_id, show_git, show_mon, left_pct, git_root, show_right,
                shell_pid,
            ) {
                println!("{}", spec);
                return;
            }
        }
        eprintln!(
            "ccide-core: no supported terminal (iTerm2/WezTerm/WT) or tmux session active; \
             run inside tmux or install tmux for 3-column tiling."
        );
    } else {
        eprintln!(
            "ccide-core: requested workspace backend '{}' is unavailable in this environment.",
            requested
        );
    }
}
