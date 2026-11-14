//! First-Class Ghostty (1.3+ AppleScript Split Engine + Smooth Fallback) Backend (`<= 300` lines).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::commands::workspace::ghostty_script::{
    build_ghostty_open_applescript, compute_ghostty_split_deltas, escape_applescript_str, rsz,
};
use crate::platform::{current_engine_exe, find_executable, ipc_dir, ipc_path};

pub fn is_ghostty_env() -> bool {
    env::var("TERM_PROGRAM")
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("ghostty")
        || !env::var("GHOSTTY_RESOURCES_DIR")
            .unwrap_or_default()
            .trim()
            .is_empty()
}

fn osascript_bin() -> PathBuf {
    let v = env::var("CCIDE_OSASCRIPT_BIN").unwrap_or_default();
    PathBuf::from(if v.trim().is_empty() {
        "/usr/bin/osascript"
    } else {
        v.trim()
    })
}

fn run_osa(script: &str, capture: bool) -> Option<String> {
    let mut c = Command::new(osascript_bin());
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

pub fn probe_ghostty_native_capable(home: &str) -> bool {
    if env::var("SSH_CONNECTION").is_ok()
        || env::var("SSH_TTY").is_ok()
        || !env::var("TMUX").unwrap_or_default().is_empty()
    {
        return false;
    }
    let mock = !env::var("CCIDE_OSASCRIPT_BIN")
        .unwrap_or_default()
        .trim()
        .is_empty();
    if mock || cfg!(target_os = "macos") {
        let cache = ipc_path("ghostty_cap.cache");
        let fresh = !mock
            && fs::metadata(&cache)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs() < 3600)
                .unwrap_or(false);
        if fresh {
            return fs::read_to_string(&cache)
                .map(|s| s.trim() == "1")
                .unwrap_or(false);
        }
        let ok = run_osa("tell application \"Ghostty\" to get version", true)
            .map(|s| parse_ghostty_version_ok(&s))
            .unwrap_or(false);
        if !mock {
            let _ = fs::write(&cache, if ok { "1" } else { "0" });
        }
        return ok;
    }
    if cfg!(target_os = "linux") {
        if let Some(bin) = find_executable("ghostty", home) {
            return Command::new(bin)
                .arg("+help")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("+split"))
                .unwrap_or(false);
        }
    }
    false
}

fn query_tty_pixel_width() -> usize {
    if let Ok(px) = env::var("CCIDE_GHOSTTY_WIDTH_PX")
        .unwrap_or_default()
        .trim()
        .parse::<usize>()
    {
        if px > 0 {
            return px;
        }
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
            if unsafe { crate::ffi::ioctl(fd, crate::ffi::TIOCGWINSZ, &mut ws as *mut _) } == 0 {
                if ws.ws_xpixel > 0 {
                    return ws.ws_xpixel as usize;
                }
                if ws.ws_col > 0 {
                    return (ws.ws_col as usize) * 9;
                }
            }
        }
    }
    1920
}

#[allow(clippy::too_many_arguments)]
pub fn run_ghostty_open(
    home: &str,
    target_dir: &str,
    yazi_id: &str,
    show_git: bool,
    show_mon: bool,
    left_pct: usize,
    git_root: &str,
    show_right: bool,
    shell_pid: u32,
) -> Option<String> {
    let panes_file = ipc_path(&format!("panes_{yazi_id}.txt"));
    if let Ok(prev) = fs::read_to_string(&panes_file) {
        close_ghostty_panes(&prev);
    }
    let eng = current_engine_exe(home);
    let sc = build_ghostty_open_applescript(
        eng.to_str().unwrap_or("ccide-core"),
        home,
        target_dir,
        yazi_id,
        show_git,
        show_mon,
        left_pct,
        git_root,
        show_right,
        shell_pid,
        query_tty_pixel_width(),
    );
    let spec = run_osa(&sc, true).filter(|s| s.starts_with("ghostty:"))?;
    let _ = fs::write(&panes_file, &spec);
    Some(spec)
}

pub fn activate_ghostty_center(center_id: &str) {
    if center_id.is_empty() {
        return;
    }
    let sc = format!(
        "tell application \"Ghostty\" to focus (first terminal whose id is \"{}\")",
        escape_applescript_str(center_id)
    );
    if let Ok(mut ch) = Command::new(osascript_bin())
        .args(["-e", &sc])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        let _ = std::thread::spawn(move || {
            let _ = ch.wait();
        });
    }
}

pub fn close_ghostty_panes(spec: &str) {
    let Some(rest) = spec.strip_prefix("ghostty:") else {
        return;
    };
    let (cid, sides) = rest.split_once(':').unwrap_or((rest, ""));
    let mut sc = String::from("tell application \"Ghostty\"\n");
    let mut any = false;
    for id in sides.split(',').filter(|s| !s.is_empty() && *s != cid) {
        any = true;
        sc.push_str(&format!(
            "try\nclose (first terminal whose id is \"{}\")\nend try\n",
            escape_applescript_str(id)
        ));
    }
    if any {
        sc.push_str("end tell");
        let _ = run_osa(&sc, false);
    }
}

pub fn rebalance_ghostty_panes(left_pct: usize) {
    let (dx_l, dx_r) = compute_ghostty_split_deltas(query_tty_pixel_width(), left_pct);
    let (dl, dr) = (rsz(dx_l, "left", "right"), rsz(dx_r, "right", "left"));
    let mut sc = String::from("tell application \"Ghostty\"\ntry\nperform action \"equalize_splits\" on focused terminal of selected tab of front window\nend try\n");
    if let Ok(rd) = fs::read_dir(ipc_dir()) {
        for e in rd.flatten() {
            if !e.file_name().to_string_lossy().starts_with("panes_") {
                continue;
            }
            if let Some(rest) = fs::read_to_string(e.path())
                .ok()
                .and_then(|t| t.trim().strip_prefix("ghostty:").map(str::to_string))
            {
                let (cid, sides) = rest.split_once(':').unwrap_or((&rest, ""));
                if let Some(lid) = sides.split(',').find(|s| !s.is_empty()) {
                    sc.push_str(&format!(
                        "try\nperform action \"resize_split:{},{}\" on (first terminal whose id is \"{}\")\nperform action \"resize_split:{},{}\" on (first terminal whose id is \"{}\")\nend try\n",
                        dl.0, dl.1, escape_applescript_str(lid), dr.0, dr.1, escape_applescript_str(cid)
                    ));
                }
            }
        }
    }
    sc.push_str("end tell");
    let _ = run_osa(&sc, false);
}
