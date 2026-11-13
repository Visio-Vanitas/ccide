//! First-Class Cross-Platform WezTerm (`wezterm cli`) 3-Column Tiling Backend (`Windows` / `macOS` / `Linux`).

use std::env;
use std::fs;
use std::process::{Command, Stdio};

use crate::platform::{current_engine_exe, find_executable, ipc_path};

#[allow(clippy::too_many_arguments)]
pub fn run_wezterm_open(
    home: &str,
    target_dir: &str,
    yazi_id: &str,
    show_git: bool,
    left_pct: usize,
    git_root: &str,
    show_right: bool,
    shell_pid: u32,
) -> Option<String> {
    let wez = find_executable("wezterm", home)?;
    let center_pane = env::var("WEZTERM_PANE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;

    let panes_file = ipc_path(&format!("panes_{}.txt", yazi_id));
    let _ = fs::remove_file(ipc_path(&format!("close_{}", yazi_id)));
    if let Ok(prev) = fs::read_to_string(&panes_file) {
        close_wezterm_panes(home, &prev);
    }

    let engine = current_engine_exe(home);
    let eng_s = engine.to_str().unwrap_or("ccide-core");
    let pid_s = shell_pid.to_string();
    let left_pct_s = left_pct.clamp(18, 45).to_string();
    let mut created_sides = Vec::new();

    // 1. Left column (`yazi`, `left_pct%` of full window)
    if let Ok(out) = Command::new(&wez)
        .args([
            "cli",
            "split-pane",
            "--pane-id",
            &center_pane,
            "--left",
            "--percent",
            &left_pct_s,
            "--cwd",
            target_dir,
            "--",
            eng_s,
            "pane-runner",
            "yazi",
            target_dir,
            yazi_id,
            &pid_s,
        ])
        .output()
    {
        let left_top = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !left_top.is_empty() {
            created_sides.push(left_top.clone());
            if show_git {
                if let Ok(gout) = Command::new(&wez)
                    .args([
                        "cli",
                        "split-pane",
                        "--pane-id",
                        &left_top,
                        "--bottom",
                        "--percent",
                        "50",
                        "--cwd",
                        git_root,
                        "--",
                        eng_s,
                        "pane-runner",
                        "lazygit",
                        git_root,
                        yazi_id,
                        &pid_s,
                    ])
                    .output()
                {
                    let gp = String::from_utf8_lossy(&gout.stdout).trim().to_string();
                    if !gp.is_empty() {
                        created_sides.push(gp);
                    }
                }
            }
        }
    }

    // 2. Right column (`sidebar`, ~16% of full window -> `23%` of remaining `70%` center+right)
    if show_right {
        let rem_pct = 100usize.saturating_sub(left_pct).max(30);
        let right_rel_pct = ((1600 + rem_pct / 2) / rem_pct).clamp(18, 35).to_string();
        if let Ok(rout) = Command::new(&wez)
            .args([
                "cli",
                "split-pane",
                "--pane-id",
                &center_pane,
                "--right",
                "--percent",
                &right_rel_pct,
                "--cwd",
                target_dir,
                "--",
                eng_s,
                "pane-runner",
                "sidebar",
                target_dir,
                yazi_id,
                &pid_s,
            ])
            .output()
        {
            let rp = String::from_utf8_lossy(&rout.stdout).trim().to_string();
            if !rp.is_empty() {
                created_sides.push(rp);
            }
        }
    }

    // 3. Re-focus Center Claude Code pane
    let _ = Command::new(&wez)
        .args(["cli", "activate-pane", "--pane-id", &center_pane])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let spec = format!("wezterm:{}:{}", center_pane, created_sides.join(","));
    let _ = fs::write(&panes_file, &spec);
    Some(spec)
}

pub fn activate_wezterm_center(home: &str, center_pane: &str) {
    if center_pane.is_empty() {
        return;
    }
    if let Some(wez) = find_executable("wezterm", home) {
        let _ = Command::new(wez)
            .args(["cli", "activate-pane", "--pane-id", center_pane])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

pub fn close_wezterm_panes(home: &str, spec: &str) {
    let Some(rest) = spec.strip_prefix("wezterm:") else {
        return;
    };
    let Some(wez) = find_executable("wezterm", home) else {
        return;
    };
    let sides = rest.split(':').nth(1).unwrap_or("");
    for pid in sides.split(',').filter(|s| !s.is_empty()) {
        let _ = Command::new(&wez)
            .args(["cli", "kill-pane", "--pane-id", pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
