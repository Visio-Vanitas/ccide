//! First-Class Cross-Platform WezTerm (`wezterm cli`) 3-Column Tiling Backend.

use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::commands::workspace::layout::LayoutPlan;
use crate::commands::workspace::spec::{read_pane_spec, Backend, OpenRequest, PaneSpec};
pub use crate::commands::workspace::wezterm_rebalance::rebalance_wezterm;
use crate::commands::workspace::wezterm_rebalance::{list_wezterm_cols, wez_status};
use crate::platform::{current_engine_exe, find_executable, ipc_path};

struct WezSplit<'a> {
    wez: &'a Path,
    yid: &'a str,
    pid_s: String,
    eng: &'a str,
}

impl<'a> WezSplit<'a> {
    fn pct(&self, target: &str, dir: &str, pct: u8, cwd: &str, role: &str) -> Option<String> {
        self.run(target, dir, "--percent", &pct.to_string(), cwd, role)
    }

    fn run(
        &self,
        target: &str,
        dir: &str,
        sz: &str,
        val: &str,
        cwd: &str,
        role: &str,
    ) -> Option<String> {
        let a = [
            "cli",
            "split-pane",
            "--pane-id",
            target,
            dir,
            sz,
            val,
            "--cwd",
            cwd,
        ];
        let b = [
            "--",
            self.eng,
            "pane-runner",
            role,
            cwd,
            self.yid,
            &self.pid_s,
        ];
        let out = Command::new(self.wez)
            .args(a)
            .args(b)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !id.is_empty()).then_some(id)
    }
}

pub fn close_wezterm_panes(home: &str, spec: &PaneSpec) {
    let Some(wez) = find_executable("wezterm", home) else {
        return;
    };
    for pid in spec.all_companion_ids() {
        wez_status(&wez, &["cli", "kill-pane", "--pane-id", pid]);
    }
}

pub fn run_wezterm_open(req: &OpenRequest) -> Option<PaneSpec> {
    let wez = find_executable("wezterm", req.home)?;
    let center_pane = env::var("WEZTERM_PANE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let _ = fs::remove_file(ipc_path(&format!("close_{}", req.yazi_id)));
    if let Some(prev) = read_pane_spec(req.yazi_id).filter(|p| p.backend == Backend::WezTerm) {
        close_wezterm_panes(req.home, &prev);
    }

    let total_cols = list_wezterm_cols(&wez)
        .and_then(|m| m.get(&center_pane).copied())
        .unwrap_or(200);
    let plan = LayoutPlan::compute(total_cols, req.left_pct, req.show_right);
    let (left_pct, right_rel_pct) = plan.wezterm_percents();
    let eng = current_engine_exe(req.home);
    let sp = WezSplit {
        wez: &wez,
        yid: req.yazi_id,
        pid_s: req.shell_pid.to_string(),
        eng: eng.to_str().unwrap_or("ccide-core"),
    };
    let (mut yazi_pane, mut lazygit_pane, mut mon_pane, mut sidebar_pane) =
        (None, None, None, None);

    if !plan.collapse_left && left_pct > 0 {
        if let Some(top) = sp.pct(&center_pane, "--left", left_pct, req.target_dir, "yazi") {
            yazi_pane = Some(top.clone());
            let mut bot = top;
            if req.show_git {
                if let Some(gp) = sp.pct(&bot, "--bottom", 50, req.git_root, "lazygit") {
                    bot = gp.clone();
                    lazygit_pane = Some(gp);
                }
            }
            if req.show_mon {
                mon_pane = sp.run(&bot, "--bottom", "--cells", "3", req.target_dir, "macmon");
            }
        }
    }
    if req.show_right && !plan.collapse_right && right_rel_pct > 0 {
        sidebar_pane = sp.pct(
            &center_pane,
            "--right",
            right_rel_pct,
            req.target_dir,
            "sidebar",
        );
    }
    let _ = Command::new(&wez)
        .args(["cli", "activate-pane", "--pane-id", &center_pane])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    Some(PaneSpec {
        backend: Backend::WezTerm,
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

pub fn activate_wezterm_center(home: &str, spec: &PaneSpec) {
    if spec.center.is_empty() {
        return;
    }
    if let Some(wez) = find_executable("wezterm", home) {
        let center = spec.center.clone();
        if let Ok(mut c) = Command::new(wez)
            .args(["cli", "activate-pane", "--pane-id", &center])
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
