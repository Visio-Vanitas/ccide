//! Tier 3 (`tmux` Universal 3-Column Tiling Engine) Backend.

use std::fs;
use std::process::{Command, Stdio};

use crate::platform::ipc_path;

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[allow(clippy::too_many_arguments)]
pub fn run_tmux_open(
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
    let panes_file = ipc_path(&format!("panes_{}.txt", yazi_id));
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

    let _ = Command::new("tmux")
        .args(["select-pane", "-t", &center_pane])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let spec = format!("tmux:{}:{}", center_pane, created_sides.join(","));
    let _ = fs::write(&panes_file, &spec);
    Some(spec)
}
