//! First-Class Ghostty (1.3+ AppleScript Split Engine + Smooth Fallback) Backend (`<= 300` lines).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::commands::workspace::ghostty_script::{
    build_ghostty_open_applescript, compute_ghostty_equalized_deltas, escape_applescript_str,
};
use crate::platform::{current_engine_exe, ipc_dir, ipc_path};

pub fn is_ghostty_env() -> bool {
    let tp = env::var("TERM_PROGRAM").unwrap_or_default();
    let gr = env::var("GHOSTTY_RESOURCES_DIR").unwrap_or_default();
    tp.trim().eq_ignore_ascii_case("ghostty") || !gr.trim().is_empty()
}

fn osascript_bin() -> PathBuf {
    let v = env::var("CCIDE_OSASCRIPT_BIN").unwrap_or_default();
    let s = v.trim();
    PathBuf::from(if s.is_empty() {
        "/usr/bin/osascript"
    } else {
        s
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

fn ghostty_app_stamp(home: &str) -> String {
    for p in [
        "/Applications/Ghostty.app/Contents/Info.plist".to_string(),
        format!("{home}/Applications/Ghostty.app/Contents/Info.plist"),
    ] {
        if let Some(secs) = fs::metadata(&p)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
        {
            return secs.to_string();
        }
    }
    "1".to_string()
}

pub fn probe_ghostty_native_capable(home: &str) -> bool {
    if env::var("SSH_CONNECTION").is_ok()
        || env::var("SSH_TTY").is_ok()
        || !env::var("TMUX").unwrap_or_default().is_empty()
    {
        return false;
    }
    // Native splits are macOS-only; a mocked `CCIDE_OSASCRIPT_BIN` still drives the AppleScript path.
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
            let want = ghostty_app_stamp(home);
            if fs::read_to_string(&cache)
                .map(|s| s.trim() == want)
                .unwrap_or(false)
            {
                return true;
            }
        }
        let ok = run_osa("tell application \"Ghostty\" to get version", true)
            .map(|s| parse_ghostty_version_ok(&s))
            .unwrap_or(false);
        if !mock {
            if ok {
                let _ = fs::write(&cache, ghostty_app_stamp(home));
            } else {
                let _ = fs::remove_file(&cache);
            }
        }
        return ok;
    }
    false
}

fn query_tty_pixel_width() -> usize {
    if let Some(px) = env::var("CCIDE_GHOSTTY_WIDTH_PX")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&p| p > 0)
    {
        return px;
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
    let total_px = query_tty_pixel_width();
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
        total_px,
    );
    let Some(spec) = run_osa(&sc, true).filter(|s| s.starts_with("ghostty:")) else {
        let _ = fs::remove_file(ipc_path("ghostty_cap.cache"));
        return None;
    };
    let _ = fs::write(&panes_file, &spec);
    let _ = fs::write(
        ipc_path(&format!("ghostty_w_{yazi_id}.txt")),
        format!("{total_px} {}", usize::from(show_right)),
    );
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
    let fallback_w = query_tty_pixel_width();
    let mut sc = String::from("tell application \"Ghostty\"\n");
    let mut any = false;
    if let Ok(rd) = fs::read_dir(ipc_dir()) {
        for e in rd.flatten() {
            let fname = e.file_name();
            let Some(yid) = fname
                .to_string_lossy()
                .strip_prefix("panes_")
                .and_then(|s| s.strip_suffix(".txt"))
                .map(str::to_string)
            else {
                continue;
            };
            if let Some(rest) = fs::read_to_string(e.path())
                .ok()
                .and_then(|t| t.trim().strip_prefix("ghostty:").map(str::to_string))
            {
                let (cid, sides_raw) = rest.split_once(':').unwrap_or((&rest, ""));
                let sides: Vec<&str> = sides_raw.split(',').filter(|s| !s.is_empty()).collect();
                let Some(lid) = sides.first().copied() else {
                    continue;
                };
                if cid.is_empty() {
                    continue;
                }
                let raw_w = fs::read_to_string(ipc_path(&format!("ghostty_w_{yid}.txt")))
                    .unwrap_or_default();
                let mut parts = raw_w.split_whitespace();
                let w = parts
                    .next()
                    .and_then(|p| p.parse::<usize>().ok())
                    .filter(|&w| w > 0)
                    .unwrap_or(fallback_w);
                let has_right = parts.next().map(|p| p == "1").unwrap_or(sides.len() >= 2);
                let (dl, dr) = compute_ghostty_equalized_deltas(w, left_pct, has_right);
                any = true;
                let (esc_cid, esc_lid) = (escape_applescript_str(cid), escape_applescript_str(lid));
                sc.push_str(&format!(
                    "try\nperform action \"equalize_splits\" on (first terminal whose id is \"{esc_cid}\")\n"
                ));
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
                sc.push_str("end try\n");
            }
        }
    }
    if any {
        sc.push_str("end tell");
        let _ = run_osa(&sc, false);
    }
}
