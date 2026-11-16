//! Ghostty 1.3+ AppleScript (`Ghostty.sdef`) Generator & Pixel Split Geometry (`<= 300` lines).

use std::env;

use crate::commands::workspace::layout::{LayoutPlan, LEFT_MIN, RIGHT_MIN};
use crate::commands::workspace::spec::OpenRequest;

pub fn compute_ghostty_equalized_deltas(
    total_px: usize,
    total_cols: usize,
    left_pct: u8,
    show_right: bool,
    has_left: bool,
    has_right: bool,
) -> ((&'static str, usize), Option<(&'static str, usize)>) {
    let plan = LayoutPlan::compute(total_cols, left_pct, show_right);
    let cols_denom = total_cols.max(1);
    let left_cols = if has_left {
        plan.left_cols.max(LEFT_MIN)
    } else {
        0
    };
    let right_cols = if has_right {
        plan.right_cols.max(RIGHT_MIN)
    } else {
        0
    };
    let left_px = left_cols.saturating_mul(total_px) / cols_denom;
    let right_px = right_cols.saturating_mul(total_px) / cols_denom;
    let eq_px = (total_px / (1 + usize::from(has_left) + usize::from(has_right))) as isize;
    let dl = if has_left {
        rsz(eq_px - left_px as isize, "left", "right")
    } else {
        ("left", 0)
    };
    let dr = has_right.then(|| {
        rsz(
            ((total_px.saturating_sub(left_px)) / 2) as isize - right_px as isize,
            "right",
            "left",
        )
    });
    (dl, dr)
}

pub fn escape_applescript_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub(crate) fn rsz(d: isize, pos: &'static str, neg: &'static str) -> (&'static str, usize) {
    if d >= 0 {
        (pos, d as usize)
    } else {
        (neg, d.unsigned_abs())
    }
}

#[allow(clippy::too_many_arguments)]
fn push_split_block(
    s: &mut String,
    engine_exe: &str,
    req: &OpenRequest,
    env_l: &str,
    role: &str,
    dir: &str,
    parent: &str,
    sp: &str,
) {
    let sh = format!(
        "exec {} pane-runner {} {} {} {}",
        shell_single_quote(engine_exe),
        role,
        shell_single_quote(dir),
        shell_single_quote(req.yazi_id),
        req.shell_pid
    );
    let cmd = format!("/bin/sh -lc {}", shell_single_quote(&sh));
    s.push_str(&format!(
        "set cfg_{role} to new surface configuration\n\
         set initial working directory of cfg_{role} to \"{}\"\n\
         set command of cfg_{role} to \"{}\"\n\
         set wait after command of cfg_{role} to false\n\
         set environment variables of cfg_{role} to {{{env_l}}}\n\
         set t_{role} to split {parent} direction {sp} with configuration cfg_{role}\n",
        escape_applescript_str(dir),
        escape_applescript_str(&cmd),
    ));
    s.push_str(&format!(
        "if side_ids is \"\" then\nset side_ids to (id of t_{role})\nelse\nset side_ids to side_ids & \",\" & (id of t_{role})\nend if\n"
    ));
}

pub fn build_ghostty_open_applescript(
    engine_exe: &str,
    req: &OpenRequest,
    total_px: usize,
    total_cols: usize,
    surface_id: &str,
    front_fallback: bool,
) -> String {
    let plan = LayoutPlan::compute(total_cols, req.left_pct, req.show_right);
    let (dx_l, dx_r) = plan.ghostty_open_deltas(total_px);
    let env_l = format!(
        "\"HOME={}\", \"YAZI_ID={}\", \"PATH={}\"",
        escape_applescript_str(req.home),
        escape_applescript_str(req.yazi_id),
        escape_applescript_str(&env::var("PATH").unwrap_or_default())
    );
    let target_block = if !surface_id.is_empty() {
        format!(
            "try\nset t_center to first terminal whose id is \"{}\"\non error\nreturn \"\"\nend try\n",
            escape_applescript_str(surface_id)
        )
    } else if front_fallback {
        format!(
            "try\nset t_center to first terminal whose name is \"ccide-{}\"\non error\nset t_center to focused terminal of selected tab of front window\nend try\n",
            escape_applescript_str(req.yazi_id)
        )
    } else {
        format!(
            "try\nset t_center to first terminal whose name is \"ccide-{}\"\non error\nreturn \"\"\nend try\n",
            escape_applescript_str(req.yazi_id)
        )
    };
    let mut s = format!(
        "tell application \"Ghostty\"\n{target_block}set cid to id of t_center\nset side_ids to \"\"\n"
    );
    let push = |out: &mut String, role: &str, dir: &str, par: &str, sp: &str| {
        push_split_block(out, engine_exe, req, &env_l, role, dir, par, sp);
    };
    if !plan.collapse_left {
        push(&mut s, "yazi", req.target_dir, "t_center", "left");
        if dx_l != 0 {
            let (d, px) = rsz(dx_l, "left", "right");
            s.push_str(&format!(
                "perform action \"resize_split:{d},{px}\" on t_yazi\n"
            ));
        }
        if req.show_git {
            push(&mut s, "lazygit", req.git_root, "t_yazi", "down");
            if req.show_mon {
                push(&mut s, "macmon", req.git_root, "t_lazygit", "down");
            }
        }
    }
    if !plan.collapse_right && req.show_right {
        push(&mut s, "sidebar", req.target_dir, "t_center", "right");
        if dx_r != 0 {
            let (d, px) = rsz(dx_r, "right", "left");
            s.push_str(&format!(
                "perform action \"resize_split:{d},{px}\" on t_center\n"
            ));
        }
    }
    s.push_str("focus t_center\nreturn \"ghostty:\" & cid & \":\" & side_ids\nend tell");
    s
}

pub fn build_ghostty_close_applescript(ids: &[&str]) -> String {
    let mut sc = String::from("tell application \"Ghostty\"\n");
    for id in ids {
        sc.push_str(&format!(
            "try\nclose (first terminal whose id is \"{}\")\nend try\n",
            escape_applescript_str(id)
        ));
    }
    sc.push_str("end tell");
    sc
}

pub fn build_ghostty_rebalance_applescript(
    cid: &str,
    lid: &str,
    dl: (&str, usize),
    dr: Option<(&str, usize)>,
) -> String {
    let (esc_cid, esc_lid) = (escape_applescript_str(cid), escape_applescript_str(lid));
    let mut sc = format!(
        "tell application \"Ghostty\"\ntry\nperform action \"equalize_splits\" on (first terminal whose id is \"{esc_cid}\")\n"
    );
    if dl.1 > 0 {
        sc.push_str(&format!(
            "perform action \"resize_split:{},{}\" on (first terminal whose id is \"{esc_lid}\")\n",
            dl.0, dl.1
        ));
    }
    if let Some((dir, px)) = dr.filter(|d| d.1 > 0) {
        sc.push_str(&format!(
            "perform action \"resize_split:{dir},{px}\" on (first terminal whose id is \"{esc_cid}\")\n"
        ));
    }
    sc.push_str("return \"ok\"\non error\nreturn \"err\"\nend try\nend tell");
    sc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::workspace::ghostty::parse_ghostty_version_ok;

    fn sample_req() -> OpenRequest<'static> {
        OpenRequest {
            home: "/Users/dev",
            target_dir: "/tmp/proj",
            yazi_id: "42",
            git_root: "/tmp/proj",
            show_git: true,
            show_mon: false,
            show_right: true,
            left_pct: 30,
            shell_pid: 999,
        }
    }

    #[test]
    fn parses_version_deltas_and_applescript_f1_targeting() {
        assert!(parse_ghostty_version_ok("1.3.0") && parse_ghostty_version_ok("Ghostty 1.3.2"));
        assert!(!parse_ghostty_version_ok("1.2.9") && !parse_ghostty_version_ok("none"));
        assert_eq!(
            LayoutPlan::compute(240, 30, true).ghostty_open_deltas(2400),
            (480, 460)
        );
        let req = sample_req();
        let sc = build_ghostty_open_applescript("/bin/cc", &req, 2400, 240, "", false);
        assert!(
            sc.contains("first terminal whose name is \"ccide-42\"")
                && !sc.contains("front window")
        );
        assert!(sc.contains("resize_split:left,480") && sc.contains("resize_split:right,460"));
        assert!(sc.contains("pane-runner yazi") && sc.contains("pane-runner sidebar"));
        let sc_sid = build_ghostty_open_applescript("/bin/cc", &req, 2400, 240, "surf-99", false);
        assert!(
            sc_sid.contains("first terminal whose id is \"surf-99\"")
                && !sc_sid.contains("front window")
        );
        let sc_fb = build_ghostty_open_applescript("/bin/cc", &req, 2400, 240, "", true);
        assert!(sc_fb.contains("focused terminal of selected tab of front window"));
        let reb =
            build_ghostty_rebalance_applescript("c1", "l1", ("left", 80), Some(("right", 460)));
        assert!(reb.contains("return \"ok\"") && reb.contains("return \"err\""));
    }

    #[test]
    fn computes_equalized_deltas_for_two_and_three_columns() {
        assert_eq!(
            compute_ghostty_equalized_deltas(2400, 240, 30, true, true, true),
            (("left", 80), Some(("right", 460)))
        );
        assert_eq!(
            compute_ghostty_equalized_deltas(2400, 240, 30, false, true, false),
            (("left", 480), None)
        );
        assert_eq!(
            compute_ghostty_equalized_deltas(2400, 240, 40, true, true, true),
            (("right", 160), Some(("right", 340)))
        );
        assert_eq!(
            compute_ghostty_equalized_deltas(700, 70, 30, true, true, true),
            (("left", 13), Some(("left", 40)))
        );
    }
}
