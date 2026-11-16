//! WezTerm Pane Column Inspection & Adaptive `LayoutPlan` Rebalance (`src/commands/workspace/wezterm_rebalance.rs`).

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::commands::workspace::layout::{LayoutPlan, RebalanceOutcome};
use crate::commands::workspace::spec::PaneSpec;
use crate::json::{extract_json_object, extract_json_str, extract_json_u64, iter_array_objects};
use crate::platform::find_executable;

pub(super) fn wez_status(wez: &Path, args: &[&str]) -> bool {
    Command::new(wez)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(super) fn parse_wezterm_pane_cols(json: &str) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    let trimmed = json.trim();
    if !trimmed.starts_with('[') {
        return map;
    }
    for obj in iter_array_objects(trimmed) {
        let pid = extract_json_u64(obj, "pane_id")
            .map(|n| n.to_string())
            .or_else(|| extract_json_str(obj, "pane_id"));
        let cols = extract_json_object(obj, "size")
            .and_then(|sz| extract_json_u64(sz, "cols"))
            .map(|c| c as usize);
        if let (Some(id), Some(c)) = (pid, cols) {
            if c > 0 {
                map.insert(id, c);
            }
        }
    }
    map
}

pub(super) fn list_wezterm_cols(wez: &Path) -> Option<HashMap<String, usize>> {
    let out = Command::new(wez)
        .args(["cli", "list", "--format", "json"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| parse_wezterm_pane_cols(&String::from_utf8_lossy(&out.stdout)))
}

fn adjust_pane_size(wez: &Path, pane_id: &str, amount: usize, dir: &str) -> bool {
    if amount == 0 {
        return true;
    }
    let amt_s = amount.to_string();
    wez_status(
        wez,
        &[
            "cli",
            "adjust-pane-size",
            "--pane-id",
            pane_id,
            "--amount",
            &amt_s,
            dir,
        ],
    )
}

fn unsup(reason: &'static str) -> RebalanceOutcome {
    RebalanceOutcome::Unsupported {
        backend: "wezterm",
        reason,
    }
}

fn adjust_col(
    wez: &Path,
    id: Option<&str>,
    cur: usize,
    target: usize,
    grow: &str,
    shrink: &str,
) -> Option<bool> {
    let id = id?;
    if target > cur {
        Some(adjust_pane_size(wez, id, target - cur, grow))
    } else if cur > target && target > 0 {
        Some(adjust_pane_size(wez, id, cur - target, shrink))
    } else {
        None
    }
}

pub fn rebalance_wezterm(home: &str, spec: &PaneSpec) -> RebalanceOutcome {
    let Some(wez) = find_executable("wezterm", home) else {
        return unsup("wezterm cli not found");
    };
    if spec.center.is_empty() {
        return unsup("missing center pane id");
    }
    let Some(cols_map) = list_wezterm_cols(&wez) else {
        return unsup("wezterm cli list failed");
    };
    let Some(&cur_center) = cols_map.get(&spec.center) else {
        return unsup("center pane not found");
    };

    let left_id = [
        spec.yazi.as_deref(),
        spec.lazygit.as_deref(),
        spec.mon.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|id| cols_map.contains_key(*id));
    let cur_left = left_id
        .and_then(|id| cols_map.get(id).copied())
        .unwrap_or(0);
    let right_id = spec
        .sidebar
        .as_deref()
        .or_else(|| {
            spec.legacy
                .then(|| spec.legacy_sides.last().map(String::as_str))
                .flatten()
        })
        .filter(|id| cols_map.contains_key(*id));
    let cur_right = right_id
        .and_then(|id| cols_map.get(id).copied())
        .unwrap_or(0);

    if left_id.is_none() && right_id.is_none() {
        return unsup("panes not found");
    }

    let plan = LayoutPlan::compute(
        cur_left + cur_center + cur_right,
        spec.left_pct,
        spec.show_right,
    );
    let l_res = adjust_col(&wez, left_id, cur_left, plan.left_cols, "Right", "Left");
    let r_res = adjust_col(&wez, right_id, cur_right, plan.right_cols, "Left", "Right");
    let attempted = usize::from(l_res.is_some()) + usize::from(r_res.is_some());
    let succeeded = usize::from(l_res == Some(true)) + usize::from(r_res == Some(true));
    if attempted > 0 && succeeded == 0 {
        unsup("panes not found")
    } else {
        RebalanceOutcome::Applied
    }
}

#[cfg(test)]
mod tests {
    use super::parse_wezterm_pane_cols;

    #[test]
    fn parses_wezterm_cli_list_json_cols() {
        let raw = r#"[
            {"window_id":0,"tab_id":1,"pane_id":10,"size":{"rows":40,"cols":60}},
            {"window_id":0,"tab_id":1,"pane_id":11,"size":{"rows":40,"cols":108}},
            {"window_id":0,"tab_id":1,"pane_id":12,"size":{"rows":40,"cols":32}}
        ]"#;
        let map = parse_wezterm_pane_cols(raw);
        assert_eq!(map.get("10").copied(), Some(60));
        assert_eq!(map.get("11").copied(), Some(108));
        assert_eq!(map.get("12").copied(), Some(32));
    }
}
