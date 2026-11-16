//! First-Class Ghostty (1.3+ AppleScript Split Engine + Linux Opt-In Probe) Backend (`<= 300` lines).

use std::env;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::commands::workspace::ghostty_linux::run_ghostty_linux_open;
#[allow(unused_imports)]
pub use crate::commands::workspace::ghostty_linux::{
    probe_ghostty, probe_ghostty_native_capable, probe_ghostty_with, GhosttyCap, GhosttyProbeEnv,
};
use crate::commands::workspace::ghostty_script::{
    build_ghostty_close_applescript, build_ghostty_open_applescript,
    build_ghostty_rebalance_applescript, compute_ghostty_equalized_deltas, escape_applescript_str,
};
use crate::commands::workspace::layout::{LayoutPlan, RebalanceOutcome};
use crate::commands::workspace::spec::{
    prune_dead_specs, read_pane_spec, Backend, OpenRequest, PaneSpec,
};
use crate::platform::{atomic_write, current_engine_exe, ipc_path};

pub fn is_ghostty_env() -> bool {
    let tp = env::var("TERM_PROGRAM").unwrap_or_default();
    let gr = env::var("GHOSTTY_RESOURCES_DIR").unwrap_or_default();
    tp.trim().eq_ignore_ascii_case("ghostty") || !gr.trim().is_empty()
}

pub(crate) fn run_osa(bin: &str, script: &str, capture: bool) -> Option<String> {
    let mut c = Command::new(bin);
    c.args(["-e", script])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    if !capture {
        return c
            .stdout(Stdio::null())
            .status()
            .ok()?
            .success()
            .then(String::new);
    }
    let o = c.output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

pub fn parse_ghostty_version_ok(raw: &str) -> bool {
    for (idx, _) in raw.match_indices('.') {
        let rev_d = raw[..idx].chars().rev().take_while(|c| c.is_ascii_digit());
        let maj_s: String = rev_d.collect::<Vec<_>>().into_iter().rev().collect();
        let rest = &raw[idx + 1..];
        let min_s: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let (Ok(maj), Ok(min)) = (maj_s.parse::<u32>(), min_s.parse::<u32>()) {
            return maj > 1 || (maj == 1 && min >= 3);
        }
    }
    false
}

fn query_tty_live_geom() -> Option<(usize, usize)> {
    if let Some(px) = env::var("CCIDE_GHOSTTY_WIDTH_PX")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&p| p > 0)
    {
        return Some((px, (px / 10).max(1)));
    }
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let tty = fs::File::open("/dev/tty").ok();
        let mut fds = tty
            .as_ref()
            .map(|f| vec![f.as_raw_fd()])
            .unwrap_or_default();
        fds.extend_from_slice(&[0, 1, 2]);
        for fd in fds {
            let mut ws: crate::ffi::Winsize = unsafe { std::mem::zeroed() };
            if unsafe { crate::ffi::ioctl(fd, crate::ffi::TIOCGWINSZ, &mut ws as *mut _) } == 0
                && ws.ws_col > 0
            {
                let cols = ws.ws_col as usize;
                let px = if ws.ws_xpixel > 0 {
                    ws.ws_xpixel as usize
                } else {
                    cols * 9
                };
                return Some((px, cols));
            }
        }
    }
    None
}

fn read_geom_file(yid: &str, role: &str) -> Option<(usize, usize)> {
    let raw = fs::read_to_string(ipc_path(&format!("geom_{yid}_{role}.txt"))).ok()?;
    let mut it = raw.split_whitespace();
    let cols = it.next()?.parse::<usize>().ok().filter(|&c| c > 0)?;
    let _rows = it.next()?.parse::<usize>().ok()?;
    let xpx = it.next()?.parse::<usize>().ok()?;
    Some((cols, xpx))
}

fn caller_yazi_id() -> Option<String> {
    ["CC_YAZI_ID", "YAZI_ID"]
        .iter()
        .find_map(|k| {
            env::var(k)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
        .or_else(|| {
            let cdir = crate::platform::cache_dir(&crate::platform::user_home_dir());
            let yid = crate::session::resolve_yazi_id_from_ppid(&cdir.to_string_lossy());
            (!yid.is_empty()).then_some(yid)
        })
}

pub fn run_ghostty_open(req: &OpenRequest, cap: GhosttyCap) -> Option<PaneSpec> {
    match cap {
        GhosttyCap::None => return None,
        GhosttyCap::LinuxCli | GhosttyCap::LinuxDbus => return run_ghostty_linux_open(req, cap),
        GhosttyCap::MacOsAppleScript => {}
    }
    if let Some(prev) = read_pane_spec(req.yazi_id).filter(|p| p.backend == Backend::Ghostty) {
        close_ghostty_panes(&prev);
    }
    let (total_px, total_cols) = query_tty_live_geom().unwrap_or((1920, 192));
    let surface_id = env::var("GHOSTTY_SURFACE_ID").unwrap_or_default();
    let surface_id = surface_id.trim();
    let front_fallback = env::var("CCIDE_GHOSTTY_FRONT_FALLBACK")
        .ok()
        .map(|s| s.trim() == "1")
        .unwrap_or(false);
    if surface_id.is_empty() {
        if let Ok(mut tty) = fs::OpenOptions::new().write(true).open("/dev/tty") {
            let _ = write!(tty, "\x1b]2;ccide-{}\x07", req.yazi_id);
            let _ = tty.flush();
        }
        thread::sleep(Duration::from_millis(30));
    }
    let eng = current_engine_exe(req.home);
    let sc = build_ghostty_open_applescript(
        eng.to_str().unwrap_or("ccide-core"),
        req,
        total_px,
        total_cols,
        surface_id,
        front_fallback,
    );
    let osa = GhosttyProbeEnv::from_env().osascript_bin;
    let Some(raw) = run_osa(&osa, &sc, true) else {
        let _ = fs::remove_file(ipc_path("ghostty_cap.cache"));
        return None;
    };
    let rest = raw.strip_prefix("ghostty:")?;
    let (cid, sides_raw) = rest.split_once(':').unwrap_or((rest, ""));
    if cid.is_empty() {
        return None;
    }
    let plan = LayoutPlan::compute(total_cols, req.left_pct, req.show_right);
    let mut sides = sides_raw.split(',').filter(|s| !s.is_empty());
    let mut next_side = |cond: bool| cond.then(|| sides.next().map(str::to_string)).flatten();
    let yazi = next_side(!plan.collapse_left);
    let lazygit = next_side(!plan.collapse_left && req.show_git);
    let mon = next_side(!plan.collapse_left && req.show_git && req.show_mon);
    let sidebar = next_side(!plan.collapse_right && req.show_right);
    let _ = atomic_write(
        &ipc_path(&format!("ghostty_w_{}.txt", req.yazi_id)),
        format!("{total_px} {} {total_cols}", usize::from(req.show_right)).as_bytes(),
    );
    Some(PaneSpec {
        backend: Backend::Ghostty,
        yid: req.yazi_id.to_string(),
        center: cid.to_string(),
        yazi,
        lazygit,
        mon,
        sidebar,
        left_pct: req.left_pct,
        show_git: req.show_git,
        show_mon: req.show_mon,
        show_right: req.show_right,
        legacy_sides: Vec::new(),
        legacy: false,
    })
}

pub fn activate_ghostty_center(spec: &PaneSpec) {
    if spec.center.is_empty() {
        return;
    }
    let sc = format!(
        "tell application \"Ghostty\" to focus (first terminal whose id is \"{}\")",
        escape_applescript_str(&spec.center)
    );
    let osa = GhosttyProbeEnv::from_env().osascript_bin;
    if let Ok(mut ch) = Command::new(osa)
        .args(["-e", &sc])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        let _ = thread::spawn(move || {
            let _ = ch.wait();
        });
    }
}

pub fn close_ghostty_panes(spec: &PaneSpec) {
    let ids = spec.all_companion_ids();
    if ids.is_empty() {
        return;
    }
    let sc = build_ghostty_close_applescript(&ids);
    let _ = run_osa(&GhosttyProbeEnv::from_env().osascript_bin, &sc, false);
}

fn ghostty_unsup(reason: &'static str) -> RebalanceOutcome {
    RebalanceOutcome::Unsupported {
        backend: "ghostty",
        reason,
    }
}

pub fn rebalance_ghostty(home: &str, spec: &PaneSpec) -> RebalanceOutcome {
    let _ = prune_dead_specs(home);
    if probe_ghostty(home) != GhosttyCap::MacOsAppleScript {
        return ghostty_unsup("Ghostty AppleScript >= 1.3 unavailable");
    }
    let cid = spec.center.as_str();
    let Some(lid) = spec.yazi.as_deref().filter(|s| !s.is_empty()) else {
        return ghostty_unsup("missing Ghostty pane ids");
    };
    if cid.is_empty() {
        return ghostty_unsup("missing Ghostty center id");
    }
    let has_right = spec.sidebar.as_deref().filter(|s| !s.is_empty()).is_some()
        || (spec.legacy && !spec.legacy_sides.is_empty());
    let live = (caller_yazi_id().as_deref() == Some(spec.yid.as_str()))
        .then(|| {
            let yg = read_geom_file(&spec.yid, "yazi").filter(|&(_, px)| px > 0);
            let sg = if has_right {
                read_geom_file(&spec.yid, "sidebar").filter(|&(_, px)| px > 0)
            } else {
                Some((0, 0))
            };
            match (yg, sg, query_tty_live_geom()) {
                (Some((yc, ypx)), Some((sc_c, spx)), Some((cpx, cc))) => {
                    Some((ypx + cpx + spx, yc + cc + sc_c))
                }
                _ => None,
            }
        })
        .flatten();
    let (total_px, total_cols) = live.unwrap_or_else(|| {
        let raw_w = fs::read_to_string(ipc_path(&format!("ghostty_w_{}.txt", spec.yid)))
            .unwrap_or_default();
        let mut parts = raw_w.split_whitespace();
        let px = parts
            .next()
            .and_then(|p| p.parse::<usize>().ok())
            .filter(|&w| w > 0);
        let _old_r = parts.next();
        let cols = parts
            .next()
            .and_then(|p| p.parse::<usize>().ok())
            .filter(|&c| c > 0);
        px.map(|w| (w, cols.unwrap_or_else(|| (w / 10).max(1))))
            .unwrap_or_else(|| query_tty_live_geom().unwrap_or((1920, 192)))
    });
    if live.is_some() {
        let _ = atomic_write(
            &ipc_path(&format!("ghostty_w_{}.txt", spec.yid)),
            format!("{total_px} {} {total_cols}", usize::from(has_right)).as_bytes(),
        );
    }
    let (dl, dr) = compute_ghostty_equalized_deltas(
        total_px,
        total_cols,
        spec.left_pct,
        spec.show_right,
        true,
        has_right,
    );
    let sc = build_ghostty_rebalance_applescript(cid, lid, dl, dr);
    if run_osa(&GhosttyProbeEnv::from_env().osascript_bin, &sc, true).as_deref() == Some("ok") {
        RebalanceOutcome::Applied
    } else {
        ghostty_unsup("panes not found")
    }
}
