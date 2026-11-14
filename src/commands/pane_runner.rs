//! `ccide-core pane-runner`: Cross-Platform Companion TUI Pane Supervisor.
//!
//! Launches `yazi`, `lazygit`, `macmon`, or `ccide-core sidebar` without requiring `zsh`.
//! Children are supervised with a non-blocking `try_wait()` poll so a `close_<yazi_id>`
//! sentinel (`wt.exe` teardown) or a dead center `shell_pid` terminates the companion
//! immediately instead of waiting for the TUI to exit on its own.

use std::fs;
use std::path::Path;
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use crate::ffi::is_pid_alive;
use crate::platform::{current_engine_exe, find_executable, ipc_dir, ipc_path, user_home_dir};

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const MIN_BACKOFF_MS: u64 = 350;
const MAX_BACKOFF_MS: u64 = 3000;
const CRASH_WINDOW: Duration = Duration::from_millis(500);

fn should_exit(yazi_id: &str, shell_pid: u32) -> bool {
    if !yazi_id.is_empty() && ipc_path(&format!("close_{}", yazi_id)).exists() {
        return true;
    }
    shell_pid > 1 && !is_pid_alive(shell_pid)
}

/// Supervise one child until it exits.
/// Returns `None` when the supervisor must stop, or `Some(crashed_fast)` when it should respawn.
fn supervise(mut child: Child, yazi_id: &str, shell_pid: u32) -> Option<bool> {
    let started = Instant::now();
    loop {
        if should_exit(yazi_id, shell_pid) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            // Clean exit (the user pressed `q`) ends the pane; a crash is restarted.
            Ok(Some(status)) => {
                if status.success() {
                    return None;
                }
                return Some(started.elapsed() < CRASH_WINDOW);
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(_) => return None,
        }
    }
}

pub fn run(args: &[String]) {
    let role = args.get(2).map(|s| s.as_str()).unwrap_or("sidebar");
    let target_dir = args.get(3).map(|s| s.as_str()).unwrap_or(".");
    let yazi_id = args.get(4).map(|s| s.as_str()).unwrap_or("");
    let shell_pid = args.get(5).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);

    let home = user_home_dir();
    let _ = fs::create_dir_all(ipc_dir());

    if !yazi_id.is_empty() {
        let _ = fs::write(ipc_path(&format!("osc_ack_{}", yazi_id)), "ok\n");
    }

    if role == "sidebar" {
        crate::commands::sidebar::run(target_dir, yazi_id, shell_pid);
        return;
    }

    let engine_exe = current_engine_exe(&home);
    // Crash-loop backoff persists across respawns so a permanently failing child
    // settles at a 3s retry cadence instead of spinning.
    let mut backoff = MIN_BACKOFF_MS;
    while !should_exit(yazi_id, shell_pid) {
        let mut cmd: Command = match role {
            "yazi" => {
                let Some(ybin) = find_executable("yazi", &home) else {
                    eprintln!("yazi not found in PATH; install sxyazi.yazi / brew install yazi.");
                    thread::sleep(Duration::from_secs(3));
                    continue;
                };
                let mut c = Command::new(ybin);
                c.current_dir(target_dir);
                if !yazi_id.is_empty() {
                    c.args(["--client-id", yazi_id]);
                }
                c.arg(target_dir);
                c
            }
            "lazygit" => {
                let Some(lgbin) = find_executable("lazygit", &home) else {
                    eprintln!("lazygit not found in PATH; install lazygit.");
                    thread::sleep(Duration::from_secs(3));
                    continue;
                };
                let py =
                    find_executable("python3", &home).or_else(|| find_executable("python", &home));
                let bridge = Path::new(&home).join(".claude/scripts/pty-hscroll-bridge.py");
                let cfg_yml = Path::new(&home).join(".config/cc-ide/lazygit.yml");

                let mut c = match py {
                    Some(py_exe) if !cfg!(windows) && bridge.is_file() => {
                        let mut c = Command::new(py_exe);
                        c.arg("-I").arg(&bridge).arg("--").arg(&lgbin);
                        c
                    }
                    _ => Command::new(&lgbin),
                };
                c.current_dir(target_dir).arg("-p").arg(target_dir);
                if cfg_yml.is_file() {
                    c.arg("--use-config-file").arg(cfg_yml);
                }
                c
            }
            "macmon" => {
                if let Some(mbin) = find_executable("macmon", &home) {
                    let mut c = Command::new(mbin);
                    c.current_dir(target_dir);
                    c
                } else {
                    let mut c = Command::new(&engine_exe);
                    c.args(["sidebar", target_dir, yazi_id, &shell_pid.to_string()]);
                    c
                }
            }
            _ => break,
        };

        match cmd.spawn() {
            Ok(child) => match supervise(child, yazi_id, shell_pid) {
                Some(crashed_fast) => {
                    backoff = if crashed_fast {
                        (backoff * 2).min(MAX_BACKOFF_MS)
                    } else {
                        MIN_BACKOFF_MS
                    };
                    thread::sleep(Duration::from_millis(backoff));
                }
                None => break,
            },
            Err(err) => {
                eprintln!("pane-runner: failed to launch {}: {}", role, err);
                thread::sleep(Duration::from_secs(3));
            }
        }
    }
}
