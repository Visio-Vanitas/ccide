//! `ccide-core pane-runner`: Cross-Platform Companion TUI Pane Supervisor (`src/commands/pane_runner.rs`).
//!
//! Launches `yazi`, `lazygit`, `macmon`, or `ccide-core sidebar` without requiring `zsh`,
//! publishing live `geom_<yid>_<role>.txt` (`<cols> <rows> <xpx> <ypx>`) and honoring
//! `reload_lg_<yid>` theme-reload sentinels across macOS, Linux, and Windows.

use std::fs;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::ffi::{is_pid_alive, RUNNING};
use crate::platform::{
    atomic_write, current_engine_exe, find_executable, ipc_dir, ipc_path, user_home_dir,
};

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const MIN_BACKOFF_MS: u64 = 350;
const MAX_BACKOFF_MS: u64 = 3000;
const CRASH_WINDOW: Duration = Duration::from_millis(500);

type Geom = (usize, usize, usize, usize);

enum SuperviseExit {
    Stop,
    ReloadNow,
    Crashed(bool),
}

fn canonical_geom_role(role: &str) -> Option<&'static str> {
    match role {
        "yazi" => Some("yazi"),
        "lazygit" => Some("lazygit"),
        "macmon" | "mon" => Some("mon"),
        "sidebar" => Some("sidebar"),
        _ => None,
    }
}

fn query_pane_geom() -> Option<Geom> {
    #[cfg(unix)]
    unsafe {
        for fd in [1, 0, 2] {
            let mut ws = crate::ffi::Winsize {
                ws_row: 0,
                ws_col: 0,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            if crate::ffi::ioctl(
                fd,
                crate::ffi::TIOCGWINSZ,
                &mut ws as *mut crate::ffi::Winsize,
            ) == 0
                && ws.ws_col > 0
                && ws.ws_row > 0
            {
                return Some((
                    ws.ws_col as usize,
                    ws.ws_row as usize,
                    ws.ws_xpixel as usize,
                    ws.ws_ypixel as usize,
                ));
            }
        }
        None
    }
    #[cfg(windows)]
    {
        crate::ffi::query_terminal_winsize().map(|(rows, cols, _, _)| (cols, rows, 0, 0))
    }
}

fn sync_geom_if_changed(yazi_id: &str, geom_role: &str, last: &mut Option<Geom>) {
    if yazi_id.is_empty() || geom_role.is_empty() {
        return;
    }
    if let Some(g) = query_pane_geom() {
        if *last != Some(g) {
            *last = Some(g);
            let line = format!("{} {} {} {}\n", g.0, g.1, g.2, g.3);
            let _ = atomic_write(
                &ipc_path(&format!("geom_{yazi_id}_{geom_role}.txt")),
                line.as_bytes(),
            );
        }
    }
}

fn should_exit(yazi_id: &str, shell_pid: u32) -> bool {
    if !yazi_id.is_empty() && ipc_path(&format!("close_{yazi_id}")).exists() {
        return true;
    }
    shell_pid > 1 && !is_pid_alive(shell_pid)
}

fn check_lazygit_reload(yazi_id: &str, spawn_time: SystemTime) -> bool {
    if yazi_id.is_empty() {
        return false;
    }
    let sentinel = ipc_path(&format!("reload_lg_{yazi_id}"));
    let Ok(meta) = fs::symlink_metadata(&sentinel) else {
        return false;
    };
    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let _ = fs::remove_file(&sentinel);
    mtime > spawn_time
}

fn supervise(
    mut child: Child,
    role: &str,
    geom_role: &str,
    yazi_id: &str,
    shell_pid: u32,
    spawn_time: SystemTime,
    last_geom: &mut Option<Geom>,
) -> SuperviseExit {
    let started = Instant::now();
    let mut ticks = 0u32;
    loop {
        if should_exit(yazi_id, shell_pid) {
            let _ = child.kill();
            let _ = child.wait();
            return SuperviseExit::Stop;
        }
        if role == "lazygit" && check_lazygit_reload(yazi_id, spawn_time) {
            #[cfg(unix)]
            if child.id() > 1 {
                unsafe {
                    let _ = crate::ffi::kill(child.id() as i32, 15);
                }
                for _ in 0..6 {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        return SuperviseExit::ReloadNow;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            return SuperviseExit::ReloadNow;
        }
        ticks = ticks.wrapping_add(1);
        if ticks % 4 == 0 {
            sync_geom_if_changed(yazi_id, geom_role, last_geom);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return SuperviseExit::Stop;
                }
                return SuperviseExit::Crashed(started.elapsed() < CRASH_WINDOW);
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(_) => return SuperviseExit::Stop,
        }
    }
}

pub fn run(args: &[String]) {
    let role = args.get(2).map(|s| s.as_str()).unwrap_or("sidebar");
    let target_dir = args.get(3).map(|s| s.as_str()).unwrap_or(".");
    let yazi_id = args.get(4).map(|s| s.as_str()).unwrap_or("");
    let shell_pid = args.get(5).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);

    let home = user_home_dir();
    let _ = fs::create_dir_all(ipc_dir());

    if !yazi_id.is_empty() {
        let _ = atomic_write(&ipc_path(&format!("osc_ack_{yazi_id}")), b"ok\n");
    }

    let geom_role = canonical_geom_role(role).unwrap_or("");
    let mut last_geom: Option<Geom> = None;
    sync_geom_if_changed(yazi_id, geom_role, &mut last_geom);

    if role == "sidebar" {
        if !yazi_id.is_empty() {
            let yid_owned = yazi_id.to_string();
            let mut bg_geom = last_geom;
            let _ = thread::Builder::new().spawn(move || {
                while RUNNING.load(Ordering::Relaxed) && !should_exit(&yid_owned, shell_pid) {
                    thread::sleep(Duration::from_millis(800));
                    sync_geom_if_changed(&yid_owned, "sidebar", &mut bg_geom);
                }
            });
        }
        crate::commands::sidebar::run(target_dir, yazi_id, shell_pid);
        return;
    }

    let engine_exe = current_engine_exe(&home);
    let mut backoff = MIN_BACKOFF_MS;
    while !should_exit(yazi_id, shell_pid) {
        let mut cmd: Command = match role {
            "yazi" => {
                let Some(ybin) = find_executable("yazi", &home) else {
                    eprintln!("yazi not found in PATH; install sxyazi.yazi / brew install yazi.");
                    thread::sleep(Duration::from_secs(3));
                    continue;
                };
                let mut c = Command::new(ybin);
                c.current_dir(target_dir);
                if !yazi_id.is_empty() {
                    c.args(["--client-id", yazi_id]);
                }
                c.arg(target_dir);
                c
            }
            "lazygit" => {
                let Some(lgbin) = find_executable("lazygit", &home) else {
                    eprintln!("lazygit not found in PATH; install lazygit.");
                    thread::sleep(Duration::from_secs(3));
                    continue;
                };
                let py =
                    find_executable("python3", &home).or_else(|| find_executable("python", &home));
                let bridge = Path::new(&home).join(".claude/scripts/pty-hscroll-bridge.py");
                let cfg_yml = Path::new(&home).join(".config/cc-ide/lazygit.yml");
                let mut c = match py {
                    Some(py_exe) if !cfg!(windows) && bridge.is_file() => {
                        let mut c = Command::new(py_exe);
                        c.arg("-I").arg(&bridge).arg("--").arg(&lgbin);
                        c
                    }
                    _ => Command::new(&lgbin),
                };
                c.current_dir(target_dir).arg("-p").arg(target_dir);
                if cfg_yml.is_file() {
                    c.arg("--use-config-file").arg(cfg_yml);
                }
                c
            }
            "macmon" | "mon" => {
                if let Some(mbin) = find_executable("macmon", &home) {
                    let mut c = Command::new(mbin);
                    c.current_dir(target_dir);
                    c
                } else {
                    let mut c = Command::new(&engine_exe);
                    c.args(["sidebar", target_dir, yazi_id, &shell_pid.to_string()]);
                    c
                }
            }
            _ => break,
        };

        let spawn_time = SystemTime::now();
        sync_geom_if_changed(yazi_id, geom_role, &mut last_geom);
        match cmd.spawn() {
            Ok(child) => match supervise(
                child,
                role,
                geom_role,
                yazi_id,
                shell_pid,
                spawn_time,
                &mut last_geom,
            ) {
                SuperviseExit::ReloadNow => {
                    backoff = MIN_BACKOFF_MS;
                }
                SuperviseExit::Crashed(crashed_fast) => {
                    backoff = if crashed_fast {
                        (backoff * 2).min(MAX_BACKOFF_MS)
                    } else {
                        MIN_BACKOFF_MS
                    };
                    thread::sleep(Duration::from_millis(backoff));
                }
                SuperviseExit::Stop => break,
            },
            Err(err) => {
                eprintln!("pane-runner: failed to launch {role}: {err}");
                thread::sleep(Duration::from_secs(3));
            }
        }
    }
}
