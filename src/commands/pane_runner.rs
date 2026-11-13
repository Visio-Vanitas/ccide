//! `ccide-core pane-runner`: Cross-Platform Companion TUI Pane Supervisor.
//!
//! Launches `yazi`, `lazygit`, `macmon`, or `ccide-core sidebar` without requiring `zsh`,
//! automatically restarting transient TUI exits and terminating cleanly when the center
//! Claude session closes (`<ipc_dir>/close_<yazi_id>` or `is_pid_alive(shell_pid) == false`).

use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::ffi::is_pid_alive;
use crate::platform::{current_engine_exe, find_executable, ipc_dir, ipc_path, user_home_dir};

fn should_exit(yazi_id: &str, shell_pid: u32) -> bool {
    if !yazi_id.is_empty() && ipc_path(&format!("close_{}", yazi_id)).exists() {
        return true;
    }
    shell_pid > 1 && !is_pid_alive(shell_pid)
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
    while !should_exit(yazi_id, shell_pid) {
        let status = match role {
            "yazi" => {
                if let Some(ybin) = find_executable("yazi", &home) {
                    let mut cmd = Command::new(ybin);
                    cmd.current_dir(target_dir);
                    if !yazi_id.is_empty() {
                        cmd.args(["--client-id", yazi_id]);
                    }
                    cmd.arg(target_dir);
                    cmd.status().ok()
                } else {
                    eprintln!("yazi not found in PATH; install sxyazi.yazi / brew install yazi.");
                    thread::sleep(Duration::from_secs(3));
                    None
                }
            }
            "lazygit" => {
                if let Some(lgbin) = find_executable("lazygit", &home) {
                    let py = find_executable("python3", &home)
                        .or_else(|| find_executable("python", &home));
                    let bridge = Path::new(&home).join(".claude/scripts/pty-hscroll-bridge.py");
                    let cfg_yml = Path::new(&home).join(".config/cc-ide/lazygit.yml");

                    let mut cmd = match py {
                        Some(py_exe) if !cfg!(windows) && bridge.is_file() => {
                            let mut c = Command::new(py_exe);
                            c.arg("-I").arg(&bridge).arg("--").arg(&lgbin);
                            c
                        }
                        _ => Command::new(&lgbin),
                    };
                    cmd.current_dir(target_dir).arg("-p").arg(target_dir);
                    if cfg_yml.is_file() {
                        cmd.arg("--use-config-file").arg(cfg_yml);
                    }
                    cmd.status().ok()
                } else {
                    eprintln!("lazygit not found in PATH; install lazygit.");
                    thread::sleep(Duration::from_secs(3));
                    None
                }
            }
            "macmon" => {
                if let Some(mbin) = find_executable("macmon", &home) {
                    Command::new(mbin).current_dir(target_dir).status().ok()
                } else {
                    Command::new(&engine_exe)
                        .args(["sidebar", target_dir, yazi_id, &shell_pid.to_string()])
                        .status()
                        .ok()
                }
            }
            _ => break,
        };

        if should_exit(yazi_id, shell_pid) {
            break;
        }
        // If the user explicitly pressed `q` inside Yazi/Lazygit (clean exit code 0), exit cleanly
        if status.map(|s| s.success()).unwrap_or(false) {
            break;
        }
        thread::sleep(Duration::from_millis(350));
    }
}
