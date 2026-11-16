//! Multi-Tab Workspace Rebalance Controller (`ccide-core workspace rebalance [yid]`).

use std::env;
use std::fs;

use crate::commands::workspace::ghostty::rebalance_ghostty;
use crate::commands::workspace::iterm::rebalance_iterm;
use crate::commands::workspace::layout::RebalanceOutcome;
use crate::commands::workspace::spec::{prune_dead_specs, read_pane_spec, Backend, PaneSpec};
use crate::commands::workspace::tmux::rebalance_tmux;
use crate::commands::workspace::wezterm::rebalance_wezterm;
use crate::commands::workspace::wt::rebalance_wt;
use crate::platform::{cache_dir, ipc_dir};
use crate::session::resolve_yazi_id_from_ppid;

fn collect_all_live_specs() -> Vec<PaneSpec> {
    let mut yids = Vec::new();
    if let Ok(rd) = fs::read_dir(ipc_dir()) {
        for entry in rd.flatten() {
            let fname = entry.file_name();
            let name = fname.to_string_lossy();
            if let Some(yid) = name
                .strip_prefix("panes_")
                .and_then(|s| s.strip_suffix(".txt"))
            {
                if !yid.is_empty() {
                    yids.push(yid.to_string());
                }
            }
        }
    }
    yids.sort();
    yids.dedup();
    yids.into_iter()
        .filter_map(|y| read_pane_spec(&y))
        .collect()
}

fn resolve_target_specs(home: &str, yid_arg: Option<&str>) -> Vec<PaneSpec> {
    let explicit = yid_arg.map(|s| s.trim()).filter(|s| !s.is_empty());
    if let Some(yid) = explicit {
        if let Some(spec) = read_pane_spec(yid) {
            return vec![spec];
        }
    }
    if let Ok(env_yid) = env::var("CC_YAZI_ID") {
        let trimmed = env_yid.trim();
        if !trimmed.is_empty() {
            if let Some(spec) = read_pane_spec(trimmed) {
                return vec![spec];
            }
        }
    }
    let cdir_s = cache_dir(home).to_string_lossy().into_owned();
    let ppid_yid = resolve_yazi_id_from_ppid(&cdir_s);
    if !ppid_yid.is_empty() {
        if let Some(spec) = read_pane_spec(&ppid_yid) {
            return vec![spec];
        }
    }
    collect_all_live_specs()
}

fn rebalance_single_tab(home: &str, spec: &PaneSpec) -> RebalanceOutcome {
    match spec.backend {
        Backend::Tmux => rebalance_tmux(spec),
        Backend::WezTerm => rebalance_wezterm(home, spec),
        Backend::Ghostty => rebalance_ghostty(home, spec),
        Backend::Iterm => rebalance_iterm(home),
        Backend::Wt => rebalance_wt(spec),
        Backend::Osc => RebalanceOutcome::Unsupported {
            backend: "osc",
            reason: "no ack channel over SSH OSC 1337",
        },
    }
}

/// Executes `ccide-core workspace rebalance [yid]`, printing the 1-line summary + per-tab
/// status lines to stdout and returning exit code `0` (at least one tab rebalanced),
/// `3` (all tabs unsupported or no workspace found), or `1` (internal error).
pub fn run_rebalance(home: &str, yid_arg: Option<&str>) -> i32 {
    let _ = prune_dead_specs(home);
    let specs = resolve_target_specs(home, yid_arg);
    if specs.is_empty() {
        println!("rebalanced 0/0 tabs");
        println!("unsupported none -: no cc-ide workspace found");
        return 3;
    }

    let mut ok_cnt = 0usize;
    let mut lines = Vec::with_capacity(specs.len());
    for spec in &specs {
        let yid = if spec.yid.is_empty() { "-" } else { &spec.yid };
        match rebalance_single_tab(home, spec) {
            RebalanceOutcome::Applied => {
                ok_cnt += 1;
                lines.push(format!("applied {} {}", spec.backend.as_str(), yid));
            }
            RebalanceOutcome::Reopened => {
                ok_cnt += 1;
                lines.push(format!("reopened {} {}", spec.backend.as_str(), yid));
            }
            RebalanceOutcome::Unsupported { backend, reason } => {
                lines.push(format!("unsupported {} {}: {}", backend, yid, reason));
            }
        }
    }

    println!("rebalanced {}/{} tabs", ok_cnt, specs.len());
    for line in &lines {
        println!("{}", line);
    }
    if ok_cnt > 0 {
        0
    } else {
        3
    }
}
