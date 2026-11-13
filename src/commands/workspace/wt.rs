//! Native Windows Terminal (`wt.exe -w 0`) 3-Column Tiling Backend (`30% | 54% | 16%`).
//!
//! Uses `wt.exe -w 0` (`split-pane`, `swap-pane`, `move-focus`) to arrange `Yazi` + `Lazygit`
//! on the left (`30%`), `Claude Code` in the center (`54%`), and `ccide-core sidebar` on the
//! right (`16%`), paired with `<ipc_dir>/close_<yazi_id>` sentinel self-exit for clean teardown.

use std::fs;
use std::process::{Command, Stdio};

use crate::platform::{current_engine_exe, find_executable, ipc_path};

#[allow(clippy::too_many_arguments)]
pub fn run_wt_open(
    home: &str,
    target_dir: &str,
    yazi_id: &str,
    show_git: bool,
    left_pct: usize,
    git_root: &str,
    show_right: bool,
    shell_pid: u32,
) -> Option<String> {
    let wt = find_executable("wt", home)?;
    let _ = fs::remove_file(ipc_path(&format!("close_{}", yazi_id)));

    let engine = current_engine_exe(home);
    let eng_s = engine.to_str().unwrap_or("ccide-core.exe");
    let pid_s = shell_pid.to_string();
    let left_clamped = left_pct.clamp(20, 42);
    let center_plus_right_frac = format!("{:.2}", (100 - left_clamped) as f64 / 100.0);

    // Build a single atomic `wt.exe -w 0` command chain (`;` separates subcommands in `wt.exe`):
    // 1. Split vertical (`-V`) at `(1 - left_pct)` with `yazi` -> `swap-pane left` puts Yazi in the
    //    30% left slot and keeps the calling Claude Code pane in the 70% right slot!
    let mut wt_args: Vec<String> = vec![
        "-w".into(),
        "0".into(),
        "split-pane".into(),
        "-V".into(),
        "-s".into(),
        center_plus_right_frac,
        "-d".into(),
        target_dir.into(),
        eng_s.into(),
        "pane-runner".into(),
        "yazi".into(),
        target_dir.into(),
        yazi_id.into(),
        pid_s.clone(),
        ";".into(),
        "swap-pane".into(),
        "-d".into(),
        "left".into(),
    ];

    // 2. While focus is on the left Yazi pane, split bottom 50% for Lazygit, then move focus right to Center
    if show_git {
        wt_args.extend([
            ";".into(),
            "split-pane".into(),
            "-H".into(),
            "-s".into(),
            "0.50".into(),
            "-d".into(),
            git_root.into(),
            eng_s.into(),
            "pane-runner".into(),
            "lazygit".into(),
            git_root.into(),
            yazi_id.into(),
            pid_s.clone(),
        ]);
    }
    wt_args.extend([";".into(), "move-focus".into(), "right".into()]);

    // 3. From the 70% Center Claude pane, split right `0.23` (`~16%` of window) for Right Sidebar, then focus left
    if show_right {
        let rem = (100 - left_clamped).max(30) as f64;
        let right_frac = format!("{:.2}", (16.0 / rem).clamp(0.18, 0.35));
        wt_args.extend([
            ";".into(),
            "split-pane".into(),
            "-V".into(),
            "-s".into(),
            right_frac,
            "-d".into(),
            target_dir.into(),
            eng_s.into(),
            "pane-runner".into(),
            "sidebar".into(),
            target_dir.into(),
            yazi_id.into(),
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
        return None;
    }

    let spec = format!("wt:{}", yazi_id);
    let _ = fs::write(ipc_path(&format!("panes_{}.txt", yazi_id)), &spec);
    Some(spec)
}

pub fn focus_wt_center_from_sidebar(home: &str) {
    if let Some(wt) = find_executable("wt", home) {
        let _ = Command::new(wt)
            .args(["-w", "0", "move-focus", "left"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

pub fn close_wt_panes(yazi_id: &str) {
    if !yazi_id.is_empty() {
        let _ = fs::write(ipc_path(&format!("close_{}", yazi_id)), "1\n");
    }
}
