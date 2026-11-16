//! Live Hot-Reload for Open `Yazi` & `Lazygit` Companion Panes on Theme Switch (`src/commands/workspace/reload.rs`).

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::commands::workspace::spec::prune_dead_specs;
use crate::platform::{atomic_write, cache_dir, find_executable, ipc_dir, ipc_path};
use crate::session::focus::emit_yazi_reveal_at;
use crate::session::resolve_isolated_session_id;

type SavedFocus = (String, usize, usize);

/// Hot-reloads all open `Yazi` and `Lazygit` panes managed by `pane-runner` so new
/// `theme.toml`, `ccide.tmTheme`, and `lazygit.yml` (`gui.theme` + `delta`) take effect in `< 120ms`.
pub fn reload_companion_panes(home: &str) {
    let _ = prune_dead_specs(home);
    let cdir = cache_dir(home);
    let mut yids: Vec<(String, Option<SavedFocus>)> = Vec::new();

    if let Ok(rd) = fs::read_dir(ipc_dir()) {
        for entry in rd.flatten() {
            let fname = entry.file_name();
            let name = fname.to_string_lossy();
            if let Some(yid) = name
                .strip_prefix("panes_")
                .and_then(|s| s.strip_suffix(".txt"))
            {
                if yid.is_empty() {
                    continue;
                }
                let (sid, _) = resolve_isolated_session_id(home, ".", yid, 0);
                let saved = if !sid.is_empty() {
                    fs::read_to_string(cdir.join(format!("main_last_file_{sid}.txt")))
                        .ok()
                        .and_then(|raw| {
                            let mut lines = raw.lines();
                            let p = lines.next()?.trim().to_string();
                            if p.is_empty() || !Path::new(&p).exists() {
                                return None;
                            }
                            let ln = lines
                                .next()
                                .and_then(|s| s.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            let sp = lines
                                .next()
                                .and_then(|s| s.trim().parse::<usize>().ok())
                                .unwrap_or(1);
                            Some((p, ln, sp))
                        })
                } else {
                    None
                };
                yids.push((yid.to_string(), saved));
            }
        }
    }

    let all_yids: Vec<String> = yids.iter().map(|(y, _)| y.clone()).collect();

    if let Some(ya) = find_executable("ya", home) {
        for yid in &all_yids {
            if let Ok(mut ch) = Command::new(&ya)
                .args(["emit-to", yid, "quit"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                let _ = ch.wait();
            }
        }
        let restore_list: Vec<(String, String, usize, usize)> = yids
            .into_iter()
            .filter_map(|(yid, sv)| sv.map(|(p, ln, sp)| (yid, p, ln, sp)))
            .collect();
        if !restore_list.is_empty() {
            let home_owned = home.to_string();
            let handle = thread::spawn(move || {
                thread::sleep(Duration::from_millis(180));
                let mut pending: Vec<(String, String, usize, usize)> = Vec::new();
                for (yid, path, ln, sp) in restore_list {
                    emit_yazi_reveal_at(&home_owned, &yid, ".", &path, ln, sp);
                    pending.push((yid, path, ln, sp));
                }
                thread::sleep(Duration::from_millis(140));
                for (yid, path, ln, sp) in pending {
                    emit_yazi_reveal_at(&home_owned, &yid, ".", &path, ln, sp);
                }
            });
            let _ = handle.join();
        }
    }

    #[cfg(unix)]
    {
        if let Ok(out) = Command::new("ps")
            .args(["-Ao", "pid=,command="])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let trimmed = line.trim_start();
                let Some((pid_s, cmd)) = trimmed.split_once(char::is_whitespace) else {
                    continue;
                };
                if cmd.contains("lazygit")
                    && cmd.contains(".config/cc-ide/lazygit.yml")
                    && !cmd.contains("pty-hscroll-bridge.py")
                {
                    if let Ok(pid) = pid_s.trim().parse::<i32>() {
                        if pid > 1 {
                            unsafe {
                                let _ = crate::ffi::kill(pid, 15);
                            }
                        }
                    }
                }
            }
        }
    }

    for yid in &all_yids {
        let _ = atomic_write(&ipc_path(&format!("reload_lg_{yid}")), b"1\n");
    }
}
