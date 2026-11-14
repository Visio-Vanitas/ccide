//! Tier 3 (`tmux` Universal 3-Column Tiling Engine) Backend.

use std::env;
use std::fs;
use std::process::{Command, Stdio};

use crate::platform::ipc_path;

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn pane_id_from(out: &std::process::Output) -> Option<String> {
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn tmux() -> Command {
    Command::new("tmux")
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
    // A stale sentinel from a previous tab would make freshly spawned panes exit immediately.
    let _ = fs::remove_file(ipc_path(&format!("close_{}", yazi_id)));
    if let Ok(prev) = fs::read_to_string(&panes_file) {
        if let Some(rest) = prev.trim().strip_prefix("tmux:") {
            if let Some((_center, sides)) = rest.split_once(':') {
                for sp in sides.split(',').filter(|s| !s.is_empty()) {
                    let _ = tmux()
                        .args(["kill-pane", "-t", sp])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
        }
    }

    let tmux_pane = env::var("TMUX_PANE").unwrap_or_default();
    let mut center_args: Vec<&str> = vec!["display-message"];
    if !tmux_pane.is_empty() {
        center_args.push("-t");
        center_args.push(tmux_pane.as_str());
    }
    center_args.extend(["-p", "#{pane_id}"]);
    let center_out = tmux().args(&center_args).output().ok()?;
    let center_pane = pane_id_from(&center_out)?;

    let win_w: usize = tmux()
        .args(["display-message", "-p", "#{window_width}"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|w| *w > 0)
        .unwrap_or(220);

    let left_pct = left_pct.clamp(18, 45);
    // On narrow windows keep the center Claude pane usable: cap both side columns so
    // `left_cols + right_cols` never consumes the whole window.
    let max_side = if win_w < 100 {
        (win_w / 3).max(16)
    } else {
        win_w.saturating_sub(60).max(20)
    };
    let left_cols = (win_w * left_pct / 100).max(46).min(max_side);
    let right_cols = if show_right {
        (win_w * 16 / 100).max(38).min(max_side)
    } else {
        0
    };

    let _ = tmux()
        .args([
            "set-option",
            "-w",
            "-t",
            center_pane.as_str(),
            "mouse",
            "on",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let runner = format!("{}/.claude/scripts/pane-runner.sh", home);
    let mut created_sides = Vec::new();

    // 1. Left column: `yazi` at `left_cols` cells.
    let yazi_cmd = format!(
        "{} yazi {} {}",
        sh_quote(&runner),
        sh_quote(target_dir),
        sh_quote(yazi_id)
    );
    let left_top = tmux()
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
        .ok()
        .and_then(|o| pane_id_from(&o));

    if let Some(left_top) = left_top {
        created_sides.push(left_top.clone());
        let mut left_bottom = left_top;

        if show_git {
            let git_cmd = format!("{} lazygit {}", sh_quote(&runner), sh_quote(git_root));
            if let Some(gp) = tmux()
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
                .ok()
                .and_then(|o| pane_id_from(&o))
            {
                created_sides.push(gp.clone());
                left_bottom = gp;
            }
        }

        if show_mon {
            let mon_cmd = format!("{} macmon {}", sh_quote(&runner), sh_quote(target_dir));
            if let Some(mp) = tmux()
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
                .ok()
                .and_then(|o| pane_id_from(&o))
            {
                created_sides.push(mp);
            }
        }
    }

    // 2. Right column: `sidebar` at `right_cols` cells.
    if show_right && right_cols > 0 {
        let sb_cmd = format!(
            "{} sidebar {} {} {}",
            sh_quote(&runner),
            sh_quote(target_dir),
            sh_quote(yazi_id),
            shell_pid
        );
        if let Some(rp) = tmux()
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
            .ok()
            .and_then(|o| pane_id_from(&o))
        {
            created_sides.push(rp);
        }
    }

    // 3. Re-focus the Center Claude Code pane.
    let _ = tmux()
        .args(["select-pane", "-t", &center_pane])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let spec = format!("tmux:{}:{}", center_pane, created_sides.join(","));
    let _ = fs::write(&panes_file, &spec);
    Some(spec)
}
