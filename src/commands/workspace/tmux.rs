//! Tier 3 (`tmux` Universal 3-Column Tiling Engine) Backend.

use std::env;
use std::fs;
use std::process::{Command, Stdio};

use crate::commands::workspace::layout::{LayoutPlan, RebalanceOutcome};
use crate::commands::workspace::spec::{read_pane_spec, Backend, OpenRequest, PaneSpec};
use crate::platform::ipc_path;

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn pane_id_from(out: &std::process::Output) -> Option<String> {
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn tmux() -> Command {
    Command::new("tmux")
}

fn parse_window_width(raw: &[u8]) -> Option<usize> {
    String::from_utf8_lossy(raw)
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|w| *w > 0)
}

fn query_window_width(center_pane: &str) -> Option<usize> {
    let out = tmux()
        .args([
            "display-message",
            "-t",
            center_pane,
            "-p",
            "#{window_width}",
        ])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| parse_window_width(&out.stdout))
        .flatten()
}

fn split_pane(
    dir_flag: &str,
    target: &str,
    size_val: &str,
    cwd: &str,
    cmd: &str,
) -> Option<String> {
    let out = tmux()
        .args(["split-window", dir_flag, "-t", target, "-l", size_val])
        .args(["-c", cwd, "-P", "-F", "#{pane_id}", cmd])
        .output()
        .ok()?;
    pane_id_from(&out)
}

fn set_tmux_mouse(center_pane: &str) -> bool {
    tmux()
        .args(["set-option", "-w", "-t", center_pane, "mouse", "on"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn resize_tmux_pane_x(pane_id: &str, cols: usize) -> bool {
    let cols_s = cols.to_string();
    tmux()
        .args(["resize-pane", "-t", pane_id, "-x", &cols_s])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn run_tmux_open(req: &OpenRequest) -> Option<PaneSpec> {
    // A stale sentinel from a previous tab would make freshly spawned panes exit immediately.
    let _ = fs::remove_file(ipc_path(&format!("close_{}", req.yazi_id)));
    if let Some(prev) = read_pane_spec(req.yazi_id).filter(|p| p.backend == Backend::Tmux) {
        close_tmux(&prev);
    }

    let tmux_pane = env::var("TMUX_PANE").unwrap_or_default();
    let mut center_args: Vec<&str> = vec!["display-message"];
    if !tmux_pane.is_empty() {
        center_args.extend(["-t", tmux_pane.as_str()]);
    }
    center_args.extend(["-p", "#{pane_id}"]);
    let center_pane = pane_id_from(&tmux().args(&center_args).output().ok()?)?;

    let win_w = query_window_width(&center_pane).unwrap_or(220);
    let plan = LayoutPlan::compute(win_w, req.left_pct, req.show_right);
    let (left_cols, right_cols) = plan.tmux_lengths();
    let _ = set_tmux_mouse(&center_pane);

    let runner = format!("{}/.claude/scripts/pane-runner.sh", req.home);
    let mut yazi_pane = None;
    let mut lazygit_pane = None;
    let mut mon_pane = None;
    let mut sidebar_pane = None;

    // 1. Left column: `yazi` at `left_cols` cells (plus optional `lazygit` & `macmon`).
    if !plan.collapse_left && left_cols > 0 {
        let left_s = left_cols.to_string();
        let ycmd = format!(
            "{} yazi {} {}",
            sh_quote(&runner),
            sh_quote(req.target_dir),
            sh_quote(req.yazi_id)
        );
        if let Some(left_top) = split_pane("-hb", &center_pane, &left_s, req.target_dir, &ycmd) {
            yazi_pane = Some(left_top.clone());
            let mut left_bottom = left_top;
            if req.show_git {
                let gcmd = format!("{} lazygit {}", sh_quote(&runner), sh_quote(req.git_root));
                if let Some(gp) = split_pane("-v", &left_bottom, "50%", req.git_root, &gcmd) {
                    left_bottom = gp.clone();
                    lazygit_pane = Some(gp);
                }
            }
            if req.show_mon {
                let mcmd = format!("{} macmon {}", sh_quote(&runner), sh_quote(req.target_dir));
                mon_pane = split_pane("-v", &left_bottom, "35%", req.target_dir, &mcmd);
            }
        }
    }

    // 2. Right column: `sidebar` at `right_cols` cells.
    if req.show_right && !plan.collapse_right && right_cols > 0 {
        let right_s = right_cols.to_string();
        let sb_cmd = format!(
            "{} sidebar {} {} {}",
            sh_quote(&runner),
            sh_quote(req.target_dir),
            sh_quote(req.yazi_id),
            req.shell_pid
        );
        sidebar_pane = split_pane("-h", &center_pane, &right_s, req.target_dir, &sb_cmd);
    }

    // 3. Re-focus the Center Claude Code pane.
    let _ = tmux()
        .args(["select-pane", "-t", &center_pane])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    Some(PaneSpec {
        backend: Backend::Tmux,
        yid: req.yazi_id.to_string(),
        center: center_pane,
        yazi: yazi_pane,
        lazygit: lazygit_pane,
        mon: mon_pane,
        sidebar: sidebar_pane,
        left_pct: req.left_pct,
        show_git: req.show_git,
        show_mon: req.show_mon,
        show_right: req.show_right,
        legacy_sides: Vec::new(),
        legacy: false,
    })
}

pub fn close_tmux(spec: &PaneSpec) {
    for sp in spec.all_companion_ids() {
        let _ = tmux()
            .args(["kill-pane", "-t", sp])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

pub fn focus_tmux_center(spec: &PaneSpec) {
    if spec.center.is_empty() {
        return;
    }
    if let Ok(mut child) = tmux()
        .args(["select-pane", "-t", &spec.center])
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

pub fn rebalance_tmux(spec: &PaneSpec) -> RebalanceOutcome {
    if spec.center.is_empty() {
        return RebalanceOutcome::Unsupported {
            backend: "tmux",
            reason: "missing center pane id",
        };
    }
    let Some(win_w) = query_window_width(&spec.center) else {
        return RebalanceOutcome::Unsupported {
            backend: "tmux",
            reason: "tmux window unavailable",
        };
    };
    let _ = set_tmux_mouse(&spec.center);
    let plan = LayoutPlan::compute(win_w, spec.left_pct, spec.show_right);
    let (left_cols, right_cols) = plan.tmux_lengths();

    let mut any_ok = false;
    if left_cols > 0 {
        for cand in [
            spec.yazi.as_deref(),
            spec.lazygit.as_deref(),
            spec.mon.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if resize_tmux_pane_x(cand, left_cols) {
                any_ok = true;
                break;
            }
        }
    }
    if right_cols > 0 {
        let right_cand = spec.sidebar.as_deref().or_else(|| {
            spec.legacy
                .then(|| spec.legacy_sides.last().map(String::as_str))
                .flatten()
        });
        if let Some(rp) = right_cand {
            if resize_tmux_pane_x(rp, right_cols) {
                any_ok = true;
            }
        }
    }
    if (left_cols == 0 && right_cols == 0) || any_ok {
        RebalanceOutcome::Applied
    } else {
        RebalanceOutcome::Unsupported {
            backend: "tmux",
            reason: "panes not found",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_handles_missing_center_rebalance() {
        assert_eq!(sh_quote("/tmp/a'b"), "'/tmp/a'\\''b'");
        assert_eq!(parse_window_width(b"240\n"), Some(240));
        assert_eq!(parse_window_width(b"0\n"), None);
        let spec = PaneSpec::parse("v2;b=tmux;k=1;lp=30;f=gr", "1").unwrap();
        assert_eq!(
            rebalance_tmux(&spec),
            RebalanceOutcome::Unsupported {
                backend: "tmux",
                reason: "missing center pane id",
            }
        );
    }
}
