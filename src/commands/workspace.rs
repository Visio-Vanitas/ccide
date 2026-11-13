//! `ccide-core workspace`: 3-Tier Adaptive Tiling Workspace & Focus Controller.
//!
//! Seamlessly combines **Native Client Split-Pane Performance** with **100% SSH & Cross-Terminal Compatibility**:
//! - **Tier 1 (Local iTerm2 Daemon via Unix Socket `/tmp/.cc-ide/iterm_daemon.sock`)**:
//!   Zero-Python-startup `< 5ms` native Metal split panes and `< 2ms` focus switching, with
//!   automatic fallback to `iterm2_workspace.py`.
//! - **Tier 2 (SSH + Client iTerm2 Native Split via `OSC 1337` Passthrough + ACK Handshake)**:
//!   Emits `\x1b]1337;Custom=id=ccide:<json>\x07` over the SSH stream to the client Mac's
//!   `ccide_daemon.py`, which splits the client's local iTerm2 window natively (`30% | 54% | 16%`)
//!   and launches remote `yazi`, `lazygit`, and `ccide-core sidebar` over a multiplexed SSH
//!   `ControlMaster` socket. Verifies completion via `/tmp/.cc-ide/osc_ack_<yazi_id>` within `450ms`.
//! - **Tier 3 (`tmux` Universal 3-Column Tiling Engine)**:
//!   Zero-API-dependency `30% | 54% | 16%` 3-column layout inside `tmux` (`$TMUX` or `tmux -L ccide`),
//!   working identically across SSH jump hosts, Ghostty, WezTerm, Kitty, Alacritty, and VS Code.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::ffi::{getppid, is_pid_alive, kill, SIGUSR2};
use crate::json::{extract_json_str, extract_json_u64};
use crate::session::transcript::{append_dismissed_subagents, get_session_subagents};
use crate::tui::palette::{render_palette_template, resolve_palette_path, Palette, PRESET_NAMES};
use crate::tui::RESET;

fn wake_all_sidebars() {
    if let Ok(rd) = fs::read_dir("/tmp/.cc-ide") {
        for entry in rd.flatten() {
            let fname = entry.file_name();
            let name = fname.to_string_lossy();
            if name.starts_with("sb_") && name.ends_with(".pid") {
                if let Ok(ptxt) = fs::read_to_string(entry.path()) {
                    if let Ok(pid) = ptxt.trim().parse::<i32>() {
                        if pid > 1 {
                            unsafe {
                                let _ = kill(pid, SIGUSR2);
                            }
                        }
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

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub const DAEMON_SOCK_PATH: &str = "/tmp/.cc-ide/iterm_daemon.sock";

/// Sends a newline-delimited JSON request to the warm local iTerm2 daemon over Unix Domain Socket.
pub fn send_daemon_ipc(json_line: &str, timeout_ms: u64) -> Option<String> {
    let stream = UnixStream::connect(DAEMON_SOCK_PATH).ok()?;
    let dur = Some(Duration::from_millis(timeout_ms));
    let _ = stream.set_read_timeout(dur);
    let _ = stream.set_write_timeout(dur);
    let mut writer = stream.try_clone().ok()?;
    writer.write_all(json_line.as_bytes()).ok()?;
    if !json_line.ends_with('\n') {
        writer.write_all(b"\n").ok()?;
    }
    writer.flush().ok()?;

    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    reader.read_line(&mut resp).ok()?;
    let trimmed = resp.trim().to_string();
    if trimmed.is_empty() || trimmed.starts_with("ERR:") {
        None
    } else {
        Some(trimmed)
    }
}

/// Emits an `OSC 1337;Custom=id=ccide:<payload>` escape sequence directly to the controlling TTY
/// so the client iTerm2 `AutoLaunch` daemon receives it even across an SSH session.
pub fn emit_osc1337(json_payload: &str) -> bool {
    let seq = format!("\x1b]1337;Custom=id=ccide:{}\x07", json_payload);
    if let Ok(mut tty) = OpenOptions::new().write(true).open("/dev/tty") {
        if tty.write_all(seq.as_bytes()).is_ok() {
            let _ = tty.flush();
            return true;
        }
    }
    let mut out = std::io::stdout();
    if out.write_all(seq.as_bytes()).is_ok() {
        let _ = out.flush();
        return true;
    }
    false
}

fn ensure_daemon_started(home: &str) {
    if Path::new(DAEMON_SOCK_PATH).exists() && send_daemon_ipc("{\"op\":\"ping\"}", 80).is_some() {
        return;
    }
    let py = find_python_bin(home);
    let daemon_script = format!("{}/.claude/scripts/ccide_daemon.py", home);
    if Path::new(&py).exists() && Path::new(&daemon_script).exists() {
        let _ = Command::new(&py)
            .arg(&daemon_script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// Unified Python interpreter resolver shared with `install.sh` & `iterm-pyenv.py`:
/// 1. `~/.cache/cc-ide/python_bin` (recorded by `iterm-pyenv.py` / `install.sh`)
/// 2. `~/.local/share/cc-ide/pyenv/bin/python3` (dedicated PEP 668-safe venv)
/// 3. `~/miniconda3/bin/python3` (legacy fallback)
/// 4. `python3` in `PATH`, else `/usr/bin/python3`
fn find_python_bin(home: &str) -> String {
    let rec_file = format!("{}/.cache/cc-ide/python_bin", home);
    if let Ok(raw) = fs::read_to_string(&rec_file) {
        let cand = raw.trim();
        if !cand.is_empty() && Path::new(cand).is_file() {
            return cand.to_string();
        }
    }
    for cand in [
        format!("{}/.local/share/cc-ide/pyenv/bin/python3", home),
        format!("{}/miniconda3/bin/python3", home),
        "/opt/homebrew/bin/python3".to_string(),
        "/usr/local/bin/python3".to_string(),
    ] {
        if Path::new(&cand).is_file() {
            return cand;
        }
    }
    if let Ok(path_var) = env::var("PATH") {
        for dir in path_var.split(':').filter(|s| !s.is_empty()) {
            let p = Path::new(dir).join("python3");
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    "/usr/bin/python3".to_string()
}

/// Fast, non-blocking Center-Pane Focus & Subagent Switcher used by `ccide-core sidebar`.
pub fn focus_center_after_view_switch(
    home: &str,
    tab_yazi_id: &str,
    session_id: &str,
    view_id: &str,
    footer_rank: usize,
) {
    let panes_file = format!("/tmp/.cc-ide/panes_{}.txt", tab_yazi_id);
    let pane_spec = fs::read_to_string(&panes_file).unwrap_or_default();
    let spec = pane_spec.trim();

    // 1. Tier 3 (`tmux`): select the recorded center pane (`< 2ms`)
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

    // 2. Tier 2 (`osc` / SSH): emit OSC 1337 focus event back through the SSH stream (`< 1ms`)
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

    // 3. Tier 1 (Local iTerm2 Daemon via Unix Socket, `< 2ms`, zero Python cold-start)
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

    // Fallback to `iterm2_workspace.py` if daemon is not yet running
    ensure_daemon_started(home);
    let py = find_python_bin(home);
    let ws = format!("{}/.claude/scripts/iterm2_workspace.py", home);
    if Path::new(&py).exists() && Path::new(&ws).exists() {
        let _ = Command::new(&py)
            .args([
                &ws,
                "switch-subagent",
                tab_yazi_id,
                session_id,
                view_id,
                &footer_rank.to_string(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

#[allow(clippy::too_many_arguments)]
fn run_tmux_open(
    home: &str,
    target_dir: &str,
    yazi_id: &str,
    show_git: bool,
    show_mon: bool,
    left_pct: usize,
    git_root: &str,
    show_right: bool,
    shell_pid: u32,
) -> Option<String> {
    // Close any previous side panes for this `yazi_id`
    let panes_file = format!("/tmp/.cc-ide/panes_{}.txt", yazi_id);
    if let Ok(prev) = fs::read_to_string(&panes_file) {
        if let Some(rest) = prev.trim().strip_prefix("tmux:") {
            if let Some((_center, sides)) = rest.split_once(':') {
                for sp in sides.split(',').filter(|s| !s.is_empty()) {
                    let _ = Command::new("tmux")
                        .args(["kill-pane", "-t", sp])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
        }
    }

    let center_pane = Command::new("tmux")
        .args(["display-message", "-p", "#{pane_id}"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;

    let win_w: usize = Command::new("tmux")
        .args(["display-message", "-p", "#{window_width}"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(220);

    let left_cols = (win_w * left_pct / 100).max(46);
    let right_cols = if show_right {
        (win_w * 16 / 100).max(38)
    } else {
        0
    };

    let _ = Command::new("tmux")
        .args(["set-option", "-w", "mouse", "on"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let runner = format!("{}/.claude/scripts/pane-runner.sh", home);
    let mut created_sides = Vec::new();

    // 1. Split Left Column (`yazi`) before center pane
    let yazi_cmd = format!(
        "{} yazi {} {}",
        sh_quote(&runner),
        sh_quote(target_dir),
        sh_quote(yazi_id)
    );
    if let Ok(out) = Command::new("tmux")
        .args([
            "split-window",
            "-hb",
            "-t",
            &center_pane,
            "-l",
            &left_cols.to_string(),
            "-c",
            target_dir,
            "-P",
            "-F",
            "#{pane_id}",
            &yazi_cmd,
        ])
        .output()
    {
        if let Ok(s) = String::from_utf8(out.stdout) {
            let left_top = s.trim().to_string();
            if !left_top.is_empty() {
                created_sides.push(left_top.clone());
                let mut left_bottom = left_top;

                if show_git {
                    let git_cmd = format!("{} lazygit {}", sh_quote(&runner), sh_quote(git_root));
                    if let Ok(gout) = Command::new("tmux")
                        .args([
                            "split-window",
                            "-v",
                            "-t",
                            &left_bottom,
                            "-l",
                            "50%",
                            "-c",
                            git_root,
                            "-P",
                            "-F",
                            "#{pane_id}",
                            &git_cmd,
                        ])
                        .output()
                    {
                        if let Ok(gs) = String::from_utf8(gout.stdout) {
                            let gp = gs.trim().to_string();
                            if !gp.is_empty() {
                                created_sides.push(gp.clone());
                                left_bottom = gp;
                            }
                        }
                    }
                }

                if show_mon {
                    let mon_cmd = format!("{} macmon {}", sh_quote(&runner), sh_quote(target_dir));
                    if let Ok(mout) = Command::new("tmux")
                        .args([
                            "split-window",
                            "-v",
                            "-t",
                            &left_bottom,
                            "-l",
                            "35%",
                            "-c",
                            target_dir,
                            "-P",
                            "-F",
                            "#{pane_id}",
                            &mon_cmd,
                        ])
                        .output()
                    {
                        if let Ok(ms) = String::from_utf8(mout.stdout) {
                            let mp = ms.trim().to_string();
                            if !mp.is_empty() {
                                created_sides.push(mp);
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Split Right Column (`sidebar`) after center pane
    if show_right && right_cols > 0 {
        let sb_cmd = format!(
            "{} sidebar {} {} {}",
            sh_quote(&runner),
            sh_quote(target_dir),
            sh_quote(yazi_id),
            shell_pid
        );
        if let Ok(rout) = Command::new("tmux")
            .args([
                "split-window",
                "-h",
                "-t",
                &center_pane,
                "-l",
                &right_cols.to_string(),
                "-c",
                target_dir,
                "-P",
                "-F",
                "#{pane_id}",
                &sb_cmd,
            ])
            .output()
        {
            if let Ok(rs) = String::from_utf8(rout.stdout) {
                let rp = rs.trim().to_string();
                if !rp.is_empty() {
                    created_sides.push(rp);
                }
            }
        }
    }

    // 3. Re-focus the Center Claude Code pane
    let _ = Command::new("tmux")
        .args(["select-pane", "-t", &center_pane])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let spec = format!("tmux:{}:{}", center_pane, created_sides.join(","));
    let _ = fs::write(&panes_file, &spec);
    Some(spec)
}

pub fn run(args: &[String]) {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let cache_dir = format!("{}/.cache/cc-ide", home);
    let _ = fs::create_dir_all(&cache_dir);
    let _ = fs::create_dir_all("/tmp/.cc-ide");

    let action = args.get(2).map(|s| s.as_str()).unwrap_or("open");

    match action {
        "open" => {
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
                .filter(|s| !s.is_empty())
                .unwrap_or(target_dir);
            let show_right = args.get(9).map(|s| s == "1").unwrap_or(true);
            let requested_backend = args
                .get(10)
                .cloned()
                .or_else(|| env::var("CCIDE_BACKEND").ok())
                .unwrap_or_else(|| "auto".to_string());

            let shell_pid = env::var("CCIDE_SHELL_PID")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_else(|| unsafe { getppid() as u32 });
            if shell_pid > 1 {
                let _ = fs::write(
                    format!("{}/tab_shell_{}.txt", cache_dir, yazi_id),
                    shell_pid.to_string(),
                );
            }

            let in_tmux = env::var("TMUX").map(|s| !s.is_empty()).unwrap_or(false);
            let in_ssh = env::var("SSH_CONNECTION").is_ok() || env::var("SSH_TTY").is_ok();
            let is_iterm = env::var("TERM_PROGRAM").as_deref() == Ok("iTerm.app")
                || env::var("LC_TERMINAL").as_deref() == Ok("iTerm2")
                || env::var("ITERM_SESSION_ID").is_ok();

            let panes_file = format!("/tmp/.cc-ide/panes_{}.txt", yazi_id);

            // ── TIER 1: Local iTerm2 Native Split (Warm Daemon Socket -> Fallback Script) ──
            if requested_backend == "iterm"
                || (requested_backend == "auto" && !in_ssh && !in_tmux && is_iterm)
            {
                let caller_guid = env::var("ITERM_SESSION_ID").unwrap_or_default();
                let req = format!(
                    "{{\"op\":\"open\",\"caller_guid\":\"{}\",\"target_dir\":\"{}\",\"yazi_id\":\"{}\",\"show_git\":{},\"show_mon\":{},\"left_pct\":{},\"git_root\":\"{}\",\"show_right\":{},\"shell_pid\":{}}}",
                    escape_json(&caller_guid),
                    escape_json(target_dir),
                    escape_json(yazi_id),
                    if show_git { "true" } else { "false" },
                    if show_mon { "true" } else { "false" },
                    left_pct,
                    escape_json(git_root),
                    if show_right { "true" } else { "false" },
                    shell_pid
                );
                if let Some(resp) = send_daemon_ipc(&req, 2000) {
                    let spec = format!("iterm:{}", resp);
                    let _ = fs::write(&panes_file, &spec);
                    println!("{}", spec);
                    return;
                }

                // Cold fallback: start daemon for subsequent calls & run `iterm2_workspace.py`
                ensure_daemon_started(&home);
                let py = find_python_bin(&home);
                let ws = format!("{}/.claude/scripts/iterm2_workspace.py", home);
                let left_pct_s = left_pct.to_string();
                if let Ok(out) = Command::new(&py)
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
                        let ids = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        let spec = format!("iterm:{}", ids);
                        let _ = fs::write(&panes_file, &spec);
                        println!("{}", spec);
                        return;
                    }
                }
            }

            // ── TIER 2: SSH + Client iTerm2 Native Split via `OSC 1337` + ACK Handshake ──
            if requested_backend == "osc"
                || (requested_backend == "auto" && in_ssh && !in_tmux && is_iterm)
            {
                let ack_path = format!("/tmp/.cc-ide/osc_ack_{}", yazi_id);
                let _ = fs::remove_file(&ack_path);
                let user = env::var("USER").unwrap_or_default();
                let ssh_conn = env::var("SSH_CONNECTION").unwrap_or_default();
                let payload = format!(
                    "{{\"op\":\"open_ssh\",\"dir\":\"{}\",\"git_root\":\"{}\",\"yid\":\"{}\",\"git\":{},\"mon\":{},\"left_pct\":{},\"right\":{},\"shell_pid\":{},\"user\":\"{}\",\"ssh_conn\":\"{}\"}}",
                    escape_json(target_dir),
                    escape_json(git_root),
                    escape_json(yazi_id),
                    if show_git { 1 } else { 0 },
                    if show_mon { 1 } else { 0 },
                    left_pct,
                    if show_right { 1 } else { 0 },
                    shell_pid,
                    escape_json(&user),
                    escape_json(&ssh_conn)
                );
                if emit_osc1337(&payload) {
                    let deadline = Instant::now() + Duration::from_millis(450);
                    while Instant::now() < deadline {
                        if Path::new(&ack_path).exists() {
                            let spec = format!("osc:{}", yazi_id);
                            let _ = fs::write(&panes_file, &spec);
                            println!("{}", spec);
                            return;
                        }
                        thread::sleep(Duration::from_millis(15));
                    }
                }
            }

            // ── TIER 3: Universal `tmux` 3-Column Tiling Engine ──
            if in_tmux {
                if let Some(spec) = run_tmux_open(
                    &home, target_dir, yazi_id, show_git, show_mon, left_pct, git_root, show_right,
                    shell_pid,
                ) {
                    println!("{}", spec);
                }
            }
        }
        "close" => {
            let arg_spec = args.get(3).map(|s| s.as_str()).unwrap_or("");
            let yazi_id = args.get(4).map(|s| s.as_str()).unwrap_or("");
            let spec = if !arg_spec.is_empty() {
                arg_spec.to_string()
            } else if !yazi_id.is_empty() {
                fs::read_to_string(format!("/tmp/.cc-ide/panes_{}.txt", yazi_id))
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            } else {
                String::new()
            };

            if let Some(rest) = spec.strip_prefix("tmux:") {
                let sides = rest.split(':').nth(1).unwrap_or(rest);
                for sp in sides.split(',').filter(|s| !s.is_empty()) {
                    let _ = Command::new("tmux")
                        .args(["kill-pane", "-t", sp])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            } else if let Some(yid) = spec.strip_prefix("osc:") {
                let payload = format!("{{\"op\":\"close\",\"yid\":\"{}\"}}", escape_json(yid));
                let _ = emit_osc1337(&payload);
            } else {
                let raw_ids = spec.strip_prefix("iterm:").unwrap_or(&spec);
                let caller_guid = env::var("ITERM_SESSION_ID").unwrap_or_default();
                let req = format!(
                    "{{\"op\":\"close\",\"pane_ids\":\"{}\",\"caller_guid\":\"{}\"}}",
                    escape_json(raw_ids),
                    escape_json(&caller_guid)
                );
                if send_daemon_ipc(&req, 1000).is_none() {
                    let py = find_python_bin(&home);
                    let ws = format!("{}/.claude/scripts/iterm2_workspace.py", home);
                    let _ = Command::new(&py)
                        .args([&ws, "close", raw_ids])
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
            if send_daemon_ipc("{\"op\":\"rebalance\"}", 1500).is_none() {
                ensure_daemon_started(&home);
                let py = find_python_bin(&home);
                let ws = format!("{}/.claude/scripts/iterm2_workspace.py", home);
                let _ = Command::new(&py)
                    .args([&ws, "rebalance"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        "focus" => {
            let query = args.get(3).map(|s| s.as_str()).unwrap_or("");
            if !query.is_empty() {
                let req = format!("{{\"op\":\"focus\",\"query\":\"{}\"}}", escape_json(query));
                if send_daemon_ipc(&req, 500).is_none() {
                    let py = find_python_bin(&home);
                    let ws = format!("{}/.claude/scripts/iterm2_workspace.py", home);
                    let _ = Command::new(&py)
                        .args([&ws, "focus", query])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
        }
        "switch-subagent" => {
            let yazi_id = args.get(3).map(|s| s.as_str()).unwrap_or("");
            let session_id = args.get(4).map(|s| s.as_str()).unwrap_or("");
            let view_id = args.get(5).map(|s| s.as_str()).unwrap_or("main");
            let footer_rank = args
                .get(6)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            focus_center_after_view_switch(&home, yazi_id, session_id, view_id, footer_rank);
        }
        "clear-subagents" | "clear-done" => {
            let explicit_sid = args.get(3).cloned().unwrap_or_default();
            let mut target_sids: Vec<(String, String)> = Vec::new();
            if !explicit_sid.is_empty() && explicit_sid != "--all" {
                target_sids.push((explicit_sid, ".".to_string()));
            } else {
                let sess_dir = format!("{}/.claude/sessions", home);
                if let Ok(rd) = fs::read_dir(&sess_dir) {
                    for entry in rd.flatten() {
                        if let Ok(txt) = fs::read_to_string(entry.path()) {
                            let pid = extract_json_u64(&txt, "pid").unwrap_or(0) as u32;
                            if is_pid_alive(pid) {
                                if let Some(sid) = extract_json_str(&txt, "sessionId") {
                                    let cwd = extract_json_str(&txt, "cwd")
                                        .unwrap_or_else(|| ".".to_string());
                                    target_sids.push((sid, cwd));
                                }
                            }
                        }
                    }
                }
            }

            let mut total_cleared = 0usize;
            let mut trackers = std::collections::HashMap::new();
            for (sid, cwd) in &target_sids {
                let items = get_session_subagents(&home, sid, cwd, &mut trackers);
                let done_ids: Vec<String> = items
                    .into_iter()
                    .filter(|a| a.status != "running")
                    .map(|a| a.full_id)
                    .collect();
                if !done_ids.is_empty() {
                    total_cleared += done_ids.len();
                    append_dismissed_subagents(&home, sid, &done_ids);
                }
            }

            // Wake all live sidebars via SIGUSR2 so the list updates in < 1ms
            wake_all_sidebars();
            println!("Cleared {} completed subagent(s).", total_cleared);
        }
        "theme" | "palette" => {
            let sub = args.get(3).map(|s| s.as_str()).unwrap_or("list");
            let cfg_path = resolve_palette_path(&home);

            match sub {
                "path" => {
                    println!("{}", cfg_path.display());
                }
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
                    let active = Palette::load(&home);
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
                    let probe =
                        Palette::from_json_str(&format!("{{\"preset\":\"{}\"}}", preset_arg));
                    let is_valid =
                        preset_arg.eq_ignore_ascii_case("default") || probe.preset != "default";
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
        _ => {}
    }
}
