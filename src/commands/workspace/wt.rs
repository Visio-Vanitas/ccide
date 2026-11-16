//! Native Windows Terminal (`wt.exe -w 0`) 3-Column Tiling Backend (`30% | 54% | 16%`).
//!
//! Uses `wt.exe -w 0` (`split-pane`, `swap-pane`, `move-focus`) to arrange `Yazi` + `Lazygit`
//! (+ optional `macmon`) on the left, `Claude Code` in the center, and `ccide-core sidebar` on
//! the right, paired with `<ipc_dir>/close_<yazi_id>` sentinel self-exit for clean teardown.

use std::fs;
use std::process::{Command, Stdio};

use crate::commands::workspace::layout::{LayoutPlan, RebalanceOutcome};
use crate::commands::workspace::spec::{Backend, OpenRequest, PaneSpec};
use crate::platform::{atomic_write, current_engine_exe, find_executable, ipc_path};

/// `wt.exe` treats `;` as a subcommand separator, so directory arguments must escape it.
fn escape_wt_dir(dir: &str) -> String {
    dir.replace(';', "\\;")
}

pub fn run_wt_open(req: &OpenRequest) -> Option<PaneSpec> {
    let wt = find_executable("wt", req.home)?;
    let _ = fs::remove_file(ipc_path(&format!("close_{}", req.yazi_id)));

    let engine = current_engine_exe(req.home);
    let eng_s = engine.to_str().unwrap_or("ccide-core.exe");
    let pid_s = req.shell_pid.to_string();
    let plan = LayoutPlan::compute(200, req.left_pct, req.show_right);
    let (center_right_frac, right_frac) = plan.wt_fractions();
    let center_plus_right_s = format!("{:.2}", center_right_frac);
    let dir_arg = escape_wt_dir(req.target_dir);
    let git_arg = escape_wt_dir(req.git_root);

    // 1. Split vertical (`-V`) with `yazi` -> `swap-pane left` puts Yazi in the left slot
    //    and keeps the calling Claude Code pane in the remaining right slot.
    let mut wt_args: Vec<String> = vec![
        "-w".into(),
        "0".into(),
        "split-pane".into(),
        "-V".into(),
        "-s".into(),
        center_plus_right_s,
        "-d".into(),
        dir_arg.clone(),
        eng_s.into(),
        "pane-runner".into(),
        "yazi".into(),
        dir_arg.clone(),
        req.yazi_id.into(),
        pid_s.clone(),
        ";".into(),
        "swap-pane".into(),
        "-d".into(),
        "left".into(),
    ];

    // 2. While focus is on the left Yazi pane, add optional `lazygit` and `macmon` splits, then focus Center.
    if req.show_git {
        wt_args.extend([
            ";".into(),
            "split-pane".into(),
            "-H".into(),
            "-s".into(),
            "0.50".into(),
            "-d".into(),
            git_arg.clone(),
            eng_s.into(),
            "pane-runner".into(),
            "lazygit".into(),
            git_arg,
            req.yazi_id.into(),
            pid_s.clone(),
        ]);
    }
    if req.show_mon {
        wt_args.extend([
            ";".into(),
            "split-pane".into(),
            "-H".into(),
            "-s".into(),
            "0.15".into(),
            "-d".into(),
            dir_arg.clone(),
            eng_s.into(),
            "pane-runner".into(),
            "macmon".into(),
            dir_arg.clone(),
            req.yazi_id.into(),
            pid_s.clone(),
        ]);
    }
    wt_args.extend([";".into(), "move-focus".into(), "right".into()]);

    // 3. From Center Claude pane, split right for Sidebar, then move focus back left to Center.
    if req.show_right && !plan.collapse_right {
        let right_s = format!("{:.2}", right_frac);
        wt_args.extend([
            ";".into(),
            "split-pane".into(),
            "-V".into(),
            "-s".into(),
            right_s,
            "-d".into(),
            dir_arg.clone(),
            eng_s.into(),
            "pane-runner".into(),
            "sidebar".into(),
            dir_arg,
            req.yazi_id.into(),
            pid_s,
            ";".into(),
            "move-focus".into(),
            "left".into(),
        ]);
    }

    let ok = Command::new(wt)
        .args(&wt_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        close_wt_panes(req.yazi_id);
        return None;
    }

    Some(PaneSpec {
        backend: Backend::Wt,
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
    })
}

pub fn focus_wt_center_from_sidebar(home: &str) {
    if let Some(wt) = find_executable("wt", home) {
        if let Ok(mut c) = Command::new(wt)
            .args(["-w", "0", "move-focus", "left"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            let _ = std::thread::spawn(move || {
                let _ = c.wait();
            });
        }
    }
}

pub fn close_wt_panes(yazi_id: &str) {
    if !yazi_id.is_empty() {
        let _ = atomic_write(&ipc_path(&format!("close_{}", yazi_id)), b"1\n");
    }
}

pub fn rebalance_wt(_spec: &PaneSpec) -> RebalanceOutcome {
    RebalanceOutcome::Unsupported {
        backend: "wt",
        reason: "wt.exe has no pane resize CLI",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_semicolons_and_reports_unsupported_rebalance() {
        assert_eq!(
            escape_wt_dir(r"C:\proj;foo\bar;baz"),
            r"C:\proj\;foo\bar\;baz"
        );
        assert_eq!(escape_wt_dir(r"C:\proj\clean"), r"C:\proj\clean");
        let spec = PaneSpec {
            backend: Backend::Wt,
            yid: "7".into(),
            center: String::new(),
            yazi: None,
            lazygit: None,
            mon: None,
            sidebar: None,
            left_pct: 30,
            show_git: true,
            show_mon: false,
            show_right: true,
            legacy_sides: Vec::new(),
            legacy: false,
        };
        assert_eq!(
            rebalance_wt(&spec),
            RebalanceOutcome::Unsupported {
                backend: "wt",
                reason: "wt.exe has no pane resize CLI",
            }
        );
    }
}
