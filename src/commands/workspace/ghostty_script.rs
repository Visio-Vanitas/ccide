//! Ghostty 1.3+ AppleScript (`Ghostty.sdef`) Generator & Pixel Split Geometry (`<= 300` lines).

use std::env;

pub fn compute_ghostty_split_deltas(total_px: usize, left_pct: usize) -> (isize, isize) {
    let lp = left_pct.clamp(18, 45) as isize;
    let w = total_px.max(1) as isize;
    (((50 - lp) * w) / 100, (((100 - lp) - 32) * w) / 200)
}

pub fn compute_ghostty_equalized_deltas(
    total_px: usize,
    left_pct: usize,
    has_right: bool,
) -> ((&'static str, usize), Option<(&'static str, usize)>) {
    let lp = left_pct.clamp(18, 45) as isize;
    let w = total_px.max(1) as isize;
    let cols = if has_right { 3 } else { 2 };
    let dx_l = w / cols - (lp * w) / 100;
    let dr = has_right.then(|| rsz(w / 3 - (16 * w) / 100, "right", "left"));
    (rsz(dx_l, "left", "right"), dr)
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
    yazi_id: &str,
    shell_pid: u32,
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
        shell_single_quote(yazi_id),
        shell_pid
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
    if role == "yazi" {
        s.push_str("set side_ids to (id of t_yazi)\n");
    } else {
        s.push_str(&format!(
            "set side_ids to side_ids & \",\" & (id of t_{role})\n"
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_ghostty_open_applescript(
    engine_exe: &str,
    home: &str,
    target_dir: &str,
    yazi_id: &str,
    show_git: bool,
    show_mon: bool,
    left_pct: usize,
    git_root: &str,
    show_right: bool,
    shell_pid: u32,
    total_px: usize,
) -> String {
    let (dx_l, dx_r) = compute_ghostty_split_deltas(total_px, left_pct);
    let env_l = format!(
        "\"HOME={}\", \"YAZI_ID={}\", \"PATH={}\"",
        escape_applescript_str(home),
        escape_applescript_str(yazi_id),
        escape_applescript_str(&env::var("PATH").unwrap_or_default())
    );
    let mut s = String::from(
        "tell application \"Ghostty\"\n\
         set t_center to focused terminal of selected tab of front window\n\
         set cid to id of t_center\n",
    );
    push_split_block(
        &mut s, engine_exe, yazi_id, shell_pid, &env_l, "yazi", target_dir, "t_center", "left",
    );
    if dx_l != 0 {
        let (d, px) = rsz(dx_l, "left", "right");
        s.push_str(&format!(
            "perform action \"resize_split:{d},{px}\" on t_yazi\n"
        ));
    }
    if show_git {
        push_split_block(
            &mut s, engine_exe, yazi_id, shell_pid, &env_l, "lazygit", git_root, "t_yazi", "down",
        );
        if show_mon {
            push_split_block(
                &mut s,
                engine_exe,
                yazi_id,
                shell_pid,
                &env_l,
                "macmon",
                git_root,
                "t_lazygit",
                "down",
            );
        }
    }
    if show_right {
        push_split_block(
            &mut s, engine_exe, yazi_id, shell_pid, &env_l, "sidebar", target_dir, "t_center",
            "right",
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::workspace::ghostty::parse_ghostty_version_ok;

    #[test]
    fn parses_version_deltas_and_applescript() {
        assert!(parse_ghostty_version_ok("1.3.0") && parse_ghostty_version_ok("Ghostty 1.3.2"));
        assert!(!parse_ghostty_version_ok("1.2.9") && !parse_ghostty_version_ok("none"));
        assert_eq!(compute_ghostty_split_deltas(2400, 30), (480, 456));
        let sc = build_ghostty_open_applescript(
            "/usr/local/bin/ccide-core",
            "/Users/dev",
            "/tmp/proj",
            "42",
            true,
            false,
            30,
            "/tmp/proj",
            true,
            999,
            2400,
        );
        assert!(sc.contains("resize_split:left,480") && sc.contains("resize_split:right,456"));
        assert!(sc.contains("pane-runner yazi") && sc.contains("pane-runner sidebar"));
    }

    #[test]
    fn computes_equalized_deltas_for_two_and_three_columns() {
        assert_eq!(
            compute_ghostty_equalized_deltas(2400, 30, true),
            (("left", 80), Some(("right", 416)))
        );
        assert_eq!(
            compute_ghostty_equalized_deltas(2400, 30, false),
            (("left", 480), None)
        );
        assert_eq!(
            compute_ghostty_equalized_deltas(2400, 40, true),
            (("right", 160), Some(("right", 416)))
        );
        assert_eq!(compute_ghostty_split_deltas(400, 30), (80, 76));
    }
}
