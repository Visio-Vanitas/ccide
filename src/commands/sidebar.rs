//! `ccide-core sidebar`: Event-Driven 3-Tier Interactive Right Sidebar Daemon.
//!
//! Architecture:
//! - **Zero Idle Polling for Session/Model/Subagents**:
//!   Registers `SIGUSR2` (`31` on macOS, `12` on Linux) and writes its PID to `/tmp/.cc-ide/sb_<sessionId>.pid`
//!   and `/tmp/.cc-ide/sb_yazi_<yaziId>.pid`. Whenever Claude Code switches `/model`, enters/exits
//!   `viewing-agent` mode, starts/stops a subagent (`SubagentStart`/`SubagentStop`), or runs a
//!   tool (`PostToolUse` / `statusline`), `SIGUSR2` wakes `poll()` in `< 0.05ms` to refresh
//!   `Panel 1` (Sprite/Logo) and `Panel 2` (`SUBAGENTS`).
//! - **Instant Signal-Driven View Switching (`SIGUSR2 -> claude_pid`)**:
//!   Clicking `● [main]` or any `[subagent]` writes `/tmp/.cc-ide/req_<sessionId>.json` and sends
//!   `kill(claude_pid, SIGUSR2)` directly to the Center Claude Code process so its injected
//!   `SIGUSR2` listener invokes `YU(id, Qt)` / `EL(Qt)` in `< 0.05ms` without timer polling.
//! - **Row-Level Diffing & Shared 500ms Telemetry**:
//!   When idle, `Panel 1` and `Panel 2` perform `0` disk reads and write `0` bytes to `stdout`;
//!   only the changed oscilloscope curve rows of `Panel 3` (`M4 PRO`) are emitted once per second.

use std::collections::HashMap;
use std::env;
use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::commands::workspace::focus_center_after_view_switch;
use crate::ffi::{
    DIRTY_EVENT, PollFd, RUNNING, SIGUSR2, Termios, handle_sig, handle_sigusr2, kill, poll, read,
    signal, tcgetattr, tcsetattr,
};
use crate::session::transcript::{
    JsonlTracker, SubagentItem, get_main_session_file_ops, get_session_subagents,
};
use crate::session::{ConfigModels, load_config_models, resolve_isolated_session_id};
use crate::telemetry::TelemetrySampler;
use crate::telemetry::chart::build_telemetry_panel_rows;
use crate::tui::grid::{BoxRow, PanelSpec, get_terminal_geometry, render_multi_panel_grid};
use crate::tui::logo::{build_top_logo_panel, load_adaptive_logos};
use crate::tui::width::trunc_to_width;
use crate::tui::{ACTIVE_VIEW_BG, AMBER, BOLD, CYAN, DIM, GREEN, MINT, RESET, SPINNERS, WHITE};

fn trigger_yazi_reveal(tab_yazi_id: &str, file_path: &str) {
    if tab_yazi_id.is_empty() || file_path.is_empty() || !Path::new(file_path).exists() {
        return;
    }
    let ya_bin = if Path::new("/opt/homebrew/bin/ya").exists() {
        "/opt/homebrew/bin/ya"
    } else {
        "ya"
    };
    let _ = Command::new(ya_bin)
        .args(["emit-to", tab_yazi_id, "reveal", file_path])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn write_active_view_state(home: &str, tab_yazi_id: &str, view_id: &str) {
    if tab_yazi_id.is_empty() {
        return;
    }
    let p = format!("{}/.cache/cc-ide/active_view_{}.txt", home, tab_yazi_id);
    let _ = fs::write(p, if view_id.is_empty() { "main" } else { view_id });
}

#[allow(clippy::too_many_arguments)]
fn trigger_native_view_switch(
    home: &str,
    tab_yazi_id: &str,
    session_id: &str,
    claude_pid: u32,
    view_id: &str,
    raw_type: &str,
    desc: &str,
    done: bool,
    footer_rank: usize,
) {
    write_active_view_state(home, tab_yazi_id, view_id);
    if !session_id.is_empty() {
        let _ = fs::create_dir_all("/tmp/.cc-ide");
        let req_path = format!("/tmp/.cc-ide/req_{}.json", session_id);
        let tmp_path = format!("{}.tmp", req_path);
        let safe_desc = desc.replace('\\', "\\\\").replace('"', "\\\"");
        let safe_type = raw_type.replace('\\', "\\\\").replace('"', "\\\"");
        let payload = format!(
            "{{\"id\":\"{}\",\"type\":\"{}\",\"desc\":\"{}\",\"done\":{}}}",
            view_id,
            if safe_type.is_empty() {
                "general-purpose"
            } else {
                &safe_type
            },
            if safe_desc.is_empty() {
                "subagent"
            } else {
                &safe_desc
            },
            if done { "true" } else { "false" }
        );
        if fs::write(&tmp_path, payload).is_ok() {
            let _ = fs::rename(&tmp_path, &req_path);
        }
        // Immediately wake `claude.exe`'s injected SIGUSR2 listener (`< 0.05ms` latency!)
        if claude_pid > 1 {
            unsafe {
                let _ = kill(claude_pid as i32, SIGUSR2);
            }
        }
    }

    focus_center_after_view_switch(home, tab_yazi_id, session_id, view_id, footer_rank);
}

fn parse_sgr_mouse_clicks(input: &str) -> Vec<usize> {
    let mut clicks = Vec::new();
    let mut search = input;
    while let Some(pos) = search.find("\x1b[<") {
        let rest = &search[pos + 3..];
        let Some(end_idx) = rest.find(['M', 'm']) else {
            break;
        };
        let kind = rest.as_bytes()[end_idx] as char;
        let body = &rest[..end_idx];
        let mut parts = body.split(';');
        if let (Some(b_str), Some(_c_str), Some(r_str)) =
            (parts.next(), parts.next(), parts.next())
            && kind == 'M'
            && b_str.parse::<u32>().ok() == Some(0)
            && let Ok(row) = r_str.parse::<usize>()
        {
            clicks.push(row);
        }
        search = &rest[end_idx + 1..];
    }
    clicks
}

pub fn run(target_cwd: &str, tab_yazi_id: &str, shell_pid_arg: u32) {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let _ = fs::create_dir_all(format!("{}/.cache/cc-ide", home));
    let _ = fs::create_dir_all("/tmp/.cc-ide");
    let my_pid_str = std::process::id().to_string();

    if !tab_yazi_id.is_empty() {
        let _ = fs::write(
            format!("/tmp/.cc-ide/sb_yazi_{}.pid", tab_yazi_id),
            &my_pid_str,
        );
    }

    unsafe {
        signal(2, handle_sig); // SIGINT
        signal(15, handle_sig); // SIGTERM
        signal(SIGUSR2, handle_sigusr2); // SIGUSR2 (event wakeup from claude.exe / hooks / statusline)
    }

    let stdin_fd = std::io::stdin().as_raw_fd();
    let mut orig_termios = Termios::zeroed();
    let has_termios = unsafe { tcgetattr(stdin_fd, &mut orig_termios) == 0 };
    if has_termios {
        let mut raw = orig_termios;
        raw.make_raw();
        unsafe {
            let _ = tcsetattr(stdin_fd, 0, &raw);
        }
    }

    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(
        b"\x1b]1;[AI] Companion & Agents\x07\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1006h\x1b[49m\x1b[2J\x1b[3J",
    );
    let _ = stdout.flush();

    let logos = load_adaptive_logos(&home);
    let mut telemetry = TelemetrySampler::new(48);
    let mut sub_trackers: HashMap<String, JsonlTracker> = HashMap::new();
    let mut main_tracker: HashMap<String, JsonlTracker> = HashMap::new();
    let mut seen_sub_seq: HashMap<String, usize> = HashMap::new();
    let mut frame_buf = String::with_capacity(16384);
    let mut prev_rows: Vec<String> = Vec::new();
    let mut row_tag_map: HashMap<usize, (u8, usize)> = HashMap::new();

    let mut active_view_id = "main".to_string();
    let mut last_click_instant = Instant::now() - Duration::from_secs(10);
    let mut tick = 0usize;

    write_active_view_state(&home, tab_yazi_id, "main");

    let (mut cached_session_id, mut cached_claude_pid) =
        resolve_isolated_session_id(&home, tab_yazi_id, target_cwd, shell_pid_arg);
    if !cached_session_id.is_empty() {
        let _ = fs::write(
            format!("/tmp/.cc-ide/sb_{}.pid", cached_session_id),
            &my_pid_str,
        );
    }

    let mut main_tool = String::new();
    let mut main_file = String::new();
    let mut subagents: Vec<SubagentItem> = Vec::new();
    let mut cached_cfg: ConfigModels =
        load_config_models(&home, &cached_session_id, tab_yazi_id, "");
    let mut prev_geom_key = (0usize, 0usize);

    while RUNNING.load(Ordering::Relaxed) {
        let geom = get_terminal_geometry();
        if (geom.rows, geom.cols) != prev_geom_key {
            prev_geom_key = (geom.rows, geom.cols);
            prev_rows.clear();
            DIRTY_EVENT.store(true, Ordering::Relaxed);
        }
        let inner_w = geom.cols.saturating_sub(4).max(10);

        let has_running_sub = subagents.iter().any(|a| a.status == "running");
        let is_dirty = DIRTY_EVENT.swap(false, Ordering::SeqCst);

        // Strictly event-driven: only query session/transcripts on `SIGUSR2` hook/state events,
        // mouse interaction, or while a subagent is actively running (with a 30s safety fallback).
        if is_dirty || has_running_sub || cached_session_id.is_empty() || tick.is_multiple_of(30) {
            if cached_session_id.is_empty() || is_dirty || tick.is_multiple_of(30) {
                let (sid, cpid) =
                    resolve_isolated_session_id(&home, tab_yazi_id, target_cwd, shell_pid_arg);
                if !sid.is_empty() {
                    if sid != cached_session_id {
                        sub_trackers.clear();
                        seen_sub_seq.clear();
                        let _ = fs::write(format!("/tmp/.cc-ide/sb_{}.pid", sid), &my_pid_str);
                    }
                    cached_session_id = sid;
                    cached_claude_pid = cpid;
                }
            }

            let (mt, mf, mut transcript_model) = get_main_session_file_ops(
                &home,
                &cached_session_id,
                tab_yazi_id,
                target_cwd,
                &mut main_tracker,
            );
            main_tool = mt;
            main_file = mf;

            // Check `/tmp/.cc-ide/cur_<sessionId>.txt` pushed by `claude.exe` on view switch or `/model` change
            if !cached_session_id.is_empty() {
                let cur_path = format!("/tmp/.cc-ide/cur_{}.txt", cached_session_id);
                if let Ok(cur_txt) = fs::read_to_string(&cur_path) {
                    let mut lines = cur_txt.lines();
                    let native_cur = lines.next().unwrap_or("main").trim();
                    let pushed_model = lines.next().unwrap_or("").trim();
                    if !pushed_model.is_empty() {
                        transcript_model = pushed_model.to_string();
                    }
                    if last_click_instant.elapsed() > Duration::from_millis(600) {
                        let target_cur = if native_cur.is_empty() {
                            "main"
                        } else {
                            native_cur
                        };
                        if target_cur != active_view_id {
                            active_view_id = target_cur.to_string();
                            write_active_view_state(&home, tab_yazi_id, &active_view_id);
                            if active_view_id == "main" {
                                trigger_yazi_reveal(tab_yazi_id, &main_file);
                            }
                        }
                    }
                }
            }

            cached_cfg =
                load_config_models(&home, &cached_session_id, tab_yazi_id, &transcript_model);
            subagents = get_session_subagents(
                &home,
                &cached_session_id,
                target_cwd,
                &mut sub_trackers,
            );

            for ag in &subagents {
                let prev = seen_sub_seq.insert(ag.full_id.clone(), ag.file_op_seq);
                if let Some(p_seq) = prev
                    && ag.file_op_seq > p_seq
                    && active_view_id == ag.full_id
                {
                    trigger_yazi_reveal(tab_yazi_id, &ag.file);
                }
            }
        }

        // ── 1. TOP PANEL: MODEL LOGO ──
        let is_main_active = active_view_id == "main";
        let (top_family, top_accent, top_badge) = if is_main_active {
            (
                cached_cfg.main_family,
                cached_cfg.main_accent,
                cached_cfg.main_model_short.clone(),
            )
        } else {
            (
                cached_cfg.sub_family,
                cached_cfg.sub_accent,
                cached_cfg.sub_model_short.clone(),
            )
        };
        let (top_panel, top_h) =
            build_top_logo_panel(geom, &logos, top_family, top_accent, top_badge);

        // ── 2. MIDDLE PANEL: SUBAGENTS ──
        let running_cnt = subagents.iter().filter(|a| a.status == "running").count();
        let mut sub_rows = Vec::with_capacity(2 + subagents.len() * 2);

        let main_bg = if is_main_active { ACTIVE_VIEW_BG } else { "" };
        let main_ptr = if is_main_active {
            format!("{CYAN}▸{RESET}{main_bg}")
        } else {
            " ".to_string()
        };
        let main_dot = if is_main_active {
            format!("{GREEN}●{RESET}{main_bg}")
        } else {
            format!("{DIM}○{RESET}{main_bg}")
        };
        let mut main_tail = if is_main_active {
            format!(
                "{AMBER}{}{RESET}{main_bg}",
                if main_tool.is_empty() { "chat" } else { &main_tool }
            )
        } else {
            format!(
                "{DIM}{}{RESET}{main_bg}",
                if main_tool.is_empty() { "chat" } else { &main_tool }
            )
        };
        if !main_file.is_empty() {
            let bname = Path::new(&main_file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            main_tail.push_str(&format!(
                " {CYAN}{}{RESET}{main_bg}",
                trunc_to_width(bname, 15)
            ));
        }
        sub_rows.push(BoxRow {
            text: format!("{main_ptr}{main_dot} {BOLD}{WHITE}[main]{RESET}{main_bg} {main_tail}"),
            bg: main_bg,
            tag: Some((1, 0)),
        });

        let cwd_base = Path::new(target_cwd)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("~");
        let main_sub_txt = if is_main_active {
            format!("{} · {}", cached_cfg.main_model_short, cwd_base)
        } else {
            format!("{} (click to switch)", cached_cfg.main_model_short)
        };
        sub_rows.push(BoxRow {
            text: format!(
                "   {DIM}{}",
                trunc_to_width(&main_sub_txt, inner_w.saturating_sub(1))
            ),
            bg: main_bg,
            tag: Some((1, 0)),
        });

        for (i_ag, ag) in subagents.iter().enumerate() {
            let is_viewed = ag.full_id == active_view_id;
            let row_bg = if is_viewed { ACTIVE_VIEW_BG } else { "" };
            let ptr = if is_viewed {
                format!("{CYAN}▸{RESET}{row_bg}")
            } else {
                " ".to_string()
            };
            let (icon, mut tail_tag) = if ag.status == "running" {
                (
                    format!("{CYAN}{}{RESET}{row_bg}", SPINNERS[tick % SPINNERS.len()]),
                    format!(
                        "{AMBER}{}{RESET}{row_bg}",
                        if ag.tool.is_empty() { "run" } else { &ag.tool }
                    ),
                )
            } else {
                let ic = if is_viewed {
                    format!("{GREEN}●{RESET}{row_bg}")
                } else {
                    format!("{GREEN}✓{RESET}{row_bg}")
                };
                let tc = if is_viewed { AMBER } else { DIM };
                (
                    ic,
                    format!(
                        "{tc}{}{RESET}{row_bg}",
                        if ag.tool.is_empty() { "done" } else { &ag.tool }
                    ),
                )
            };

            if !ag.file.is_empty() {
                let bname = Path::new(&ag.file)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                tail_tag.push_str(&format!(
                    " {CYAN}{}{RESET}{row_bg}",
                    trunc_to_width(bname, 13)
                ));
            }

            sub_rows.push(BoxRow {
                text: format!(
                    "{ptr}{icon} {BOLD}{WHITE}[{}]{RESET}{row_bg} {tail_tag}",
                    ag.short_type
                ),
                bg: row_bg,
                tag: Some((2, i_ag)),
            });
            let desc_line = format!("{} · {}", cached_cfg.sub_model_short, ag.desc);
            sub_rows.push(BoxRow {
                text: format!(
                    "   {DIM}{}",
                    trunc_to_width(&desc_line, inner_w.saturating_sub(1))
                ),
                bg: row_bg,
                tag: Some((2, i_ag)),
            });
        }

        let sub_badge = if running_cnt == 0 {
            format!("main+{}", subagents.len())
        } else {
            format!("{} run/{}", running_cnt, subagents.len())
        };
        let mid_panel = PanelSpec {
            title: "SUBAGENTS".to_string(),
            badge: sub_badge,
            accent: CYAN,
            rows: sub_rows,
            fixed_height: None,
            weight: 0.88,
            raw_interior: false,
        };

        // ── 3. BOTTOM PANEL: SHARED SINGLE-INSTANCE TELEMETRY OSCILLOSCOPE CURVES ──
        telemetry.sample();
        let rem_after_top = geom.rows.saturating_sub(top_h);
        let bot_h = ((rem_after_top as f64) * (1.12 / 2.0)).round() as usize;
        let bot_h = bot_h.clamp(12, rem_after_top.saturating_sub(6).max(12));
        let bot_rows = build_telemetry_panel_rows(&telemetry, inner_w, bot_h);
        let bot_panel = PanelSpec {
            title: telemetry.cpu_brand.to_uppercase(),
            badge: format!("{}C · {:.0}GB", telemetry.cpu_cores, telemetry.total_mem_gb),
            accent: MINT,
            rows: bot_rows,
            fixed_height: Some(bot_h),
            weight: 1.12,
            raw_interior: false,
        };

        let panels = [top_panel, mid_panel, bot_panel];
        render_multi_panel_grid(
            geom,
            &panels,
            &mut frame_buf,
            &mut row_tag_map,
            &mut prev_rows,
        );
        if !frame_buf.is_empty() {
            let _ = stdout.write_all(frame_buf.as_bytes());
            let _ = stdout.flush();
        }

        // Sleep until `SIGUSR2` event, mouse click on `stdin_fd`, or 1000ms telemetry tick (250ms when spinner active)
        let timeout_ms = if running_cnt > 0 { 250 } else { 1000 };
        let mut pfd = PollFd {
            fd: stdin_fd,
            events: 1,
            revents: 0,
        };
        let ready = unsafe { poll(&mut pfd, 1, timeout_ms) };
        if ready > 0 && (pfd.revents & 1) != 0 {
            let mut in_buf = [0u8; 256];
            let n = unsafe { read(stdin_fd, in_buf.as_mut_ptr() as *mut c_void, 256) };
            if n > 0 {
                DIRTY_EVENT.store(true, Ordering::Relaxed);
                let raw_in = String::from_utf8_lossy(&in_buf[..n as usize]);
                if raw_in.contains('\x1b') && !raw_in.contains("[<") && active_view_id != "main" {
                    active_view_id = "main".to_string();
                    last_click_instant = Instant::now();
                    trigger_native_view_switch(
                        &home,
                        tab_yazi_id,
                        &cached_session_id,
                        cached_claude_pid,
                        "main",
                        "",
                        "",
                        false,
                        0,
                    );
                    trigger_yazi_reveal(tab_yazi_id, &main_file);
                }

                for r_clicked in parse_sgr_mouse_clicks(&raw_in) {
                    if let Some(&(tkind, tidx)) = row_tag_map.get(&r_clicked) {
                        if tkind == 1 {
                            active_view_id = "main".to_string();
                            last_click_instant = Instant::now();
                            trigger_native_view_switch(
                                &home,
                                tab_yazi_id,
                                &cached_session_id,
                                cached_claude_pid,
                                "main",
                                "",
                                "",
                                false,
                                0,
                            );
                            trigger_yazi_reveal(tab_yazi_id, &main_file);
                        } else if tkind == 2
                            && let Some(ag) = subagents.get(tidx)
                        {
                            active_view_id = ag.full_id.clone();
                            last_click_instant = Instant::now();
                            trigger_native_view_switch(
                                &home,
                                tab_yazi_id,
                                &cached_session_id,
                                cached_claude_pid,
                                &ag.full_id,
                                &ag.raw_type,
                                &ag.desc,
                                ag.status != "running",
                                ag.footer_rank,
                            );
                            trigger_yazi_reveal(tab_yazi_id, &ag.file);
                        }
                    }
                }
            }
        }

        tick = tick.wrapping_add(1);
    }

    let _ = stdout.write_all(b"\x1b[?1000l\x1b[?1006l\x1b[?1049l\x1b[?25h\x1b[0m\n");
    let _ = stdout.flush();
    if has_termios {
        unsafe {
            let _ = tcsetattr(stdin_fd, 0, &orig_termios);
        }
    }
}
