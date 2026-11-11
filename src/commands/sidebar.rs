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
    handle_sig, handle_sigusr2, kill, poll, read, signal, tcgetattr, tcsetattr, PollFd, Termios,
    DIRTY_EVENT, RUNNING, SIGUSR2,
};
use crate::session::transcript::{
    append_dismissed_subagents, get_main_session_file_ops, get_session_subagents, JsonlTracker,
    SubagentItem,
};
use crate::session::{load_config_models, resolve_isolated_session_id, ConfigModels};
use crate::telemetry::chart::build_telemetry_panel_rows;
use crate::telemetry::TelemetrySampler;
use crate::tui::grid::{get_terminal_geometry, render_multi_panel_grid, BoxRow, PanelSpec};
use crate::tui::logo::{build_top_logo_panel, load_adaptive_logos};
use crate::tui::palette::PaletteWatcher;
use crate::tui::width::{display_width, trunc_to_width};
use crate::tui::{BOLD, RESET, SPINNERS};

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
    if !session_id.is_empty() || claude_pid > 1 {
        let _ = fs::create_dir_all("/tmp/.cc-ide");
        let safe_desc = desc
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\n', '\r', '\t'], " ");
        let safe_type = raw_type
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\n', '\r', '\t'], " ");
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
        if claude_pid > 1 {
            let pid_req = format!("/tmp/.cc-ide/req_pid_{}.json", claude_pid);
            let pid_tmp = format!("{}.tmp", pid_req);
            if fs::write(&pid_tmp, &payload).is_ok() {
                let _ = fs::rename(&pid_tmp, &pid_req);
            }
        }
        if !session_id.is_empty() {
            let req_path = format!("/tmp/.cc-ide/req_{}.json", session_id);
            let tmp_path = format!("{}.tmp", req_path);
            if fs::write(&tmp_path, &payload).is_ok() {
                let _ = fs::rename(&tmp_path, &req_path);
            }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputEvent {
    LeftClick { col: usize, row: usize },
    SecondaryClick { row: usize },
    WheelUp,
    WheelDown,
    CursorUp,
    CursorDown,
    PageUp,
    PageDown,
    Home,
    End,
    ActivateSelected,
    DismissSelected,
    ClearAllDone,
    Escape,
}

fn parse_input_events(input: &str) -> Vec<InputEvent> {
    let mut events = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        if bytes[i] == 0x1b {
            if i + 2 < len && bytes[i + 1] == b'[' && bytes[i + 2] == b'<' {
                // SGR Mouse sequence: `\x1b[<btn;col;rowM` or `m`
                let rest = &input[i + 3..];
                if let Some(end_idx) = rest.find(['M', 'm']) {
                    let kind = rest.as_bytes()[end_idx] as char;
                    let body = &rest[..end_idx];
                    let mut parts = body.split(';');
                    if let (Some(b_str), Some(c_str), Some(r_str)) =
                        (parts.next(), parts.next(), parts.next())
                    {
                        if kind == 'M' {
                            if let (Ok(btn), Ok(col), Ok(row)) = (
                                b_str.parse::<u32>(),
                                c_str.parse::<usize>(),
                                r_str.parse::<usize>(),
                            ) {
                                match btn {
                                    0 => events.push(InputEvent::LeftClick { col, row }),
                                    1 | 2 => events.push(InputEvent::SecondaryClick { row }),
                                    64 => events.push(InputEvent::WheelUp),
                                    65 => events.push(InputEvent::WheelDown),
                                    _ => {}
                                }
                            }
                        }
                    }
                    i += 3 + end_idx + 1;
                    continue;
                }
            }

            if i + 2 < len && (bytes[i + 1] == b'[' || bytes[i + 1] == b'O') {
                let mode = bytes[i + 1];
                let code = bytes[i + 2];
                if mode == b'[' && code.is_ascii_digit() {
                    // Extended CSI sequence like `\x1b[5~`, `\x1b[6~`, `\x1b[3~`, `\x1b[1~`, `\x1b[4~`
                    let mut j = i + 2;
                    while j < len && (bytes[j].is_ascii_digit() || bytes[j] == b';') {
                        j += 1;
                    }
                    if j < len {
                        let final_b = bytes[j];
                        let param = &input[i + 2..j];
                        match (param, final_b) {
                            ("5", b'~') => events.push(InputEvent::PageUp),
                            ("6", b'~') => events.push(InputEvent::PageDown),
                            ("3", b'~') => events.push(InputEvent::DismissSelected),
                            ("1", b'~') | ("7", b'~') => events.push(InputEvent::Home),
                            ("4", b'~') | ("8", b'~') => events.push(InputEvent::End),
                            (_, b'A') => events.push(InputEvent::CursorUp),
                            (_, b'B') => events.push(InputEvent::CursorDown),
                            (_, b'C') => events.push(InputEvent::ActivateSelected),
                            (_, b'D') => events.push(InputEvent::DismissSelected),
                            _ => {}
                        }
                        i = j + 1;
                        continue;
                    }
                } else {
                    match code {
                        b'A' => events.push(InputEvent::CursorUp),
                        b'B' => events.push(InputEvent::CursorDown),
                        b'C' => events.push(InputEvent::ActivateSelected),
                        b'D' => events.push(InputEvent::DismissSelected),
                        b'H' => events.push(InputEvent::Home),
                        b'F' => events.push(InputEvent::End),
                        _ => {}
                    }
                    i += 3;
                    continue;
                }
            }

            events.push(InputEvent::Escape);
            i += 1;
            continue;
        }

        match bytes[i] {
            b'k' | b'K' => events.push(InputEvent::CursorUp),
            b'j' | b'J' => events.push(InputEvent::CursorDown),
            b'\r' | b'\n' | b' ' | b'l' | b'L' => events.push(InputEvent::ActivateSelected),
            b'x' | b'X' | b'd' | b'D' | b'h' | b'H' | 0x7f | 0x08 => {
                events.push(InputEvent::DismissSelected);
            }
            b'c' | b'C' => events.push(InputEvent::ClearAllDone),
            b'g' => events.push(InputEvent::Home),
            b'G' => events.push(InputEvent::End),
            _ => {}
        }
        i += 1;
    }

    events
}

fn sync_scroll_to_cursor(
    cursor_idx: usize,
    sub_scroll_offset: &mut usize,
    viewport_h: usize,
    max_scroll: usize,
) {
    if cursor_idx == 0 {
        return;
    }
    let i_ag = cursor_idx - 1;
    let item_top = i_ag * 2;
    let item_bot = item_top + 2;
    if item_top < *sub_scroll_offset {
        *sub_scroll_offset = item_top.min(max_scroll);
    } else if viewport_h >= 2 && item_bot > *sub_scroll_offset + viewport_h {
        *sub_scroll_offset = item_bot.saturating_sub(viewport_h).min(max_scroll);
    }
}

fn default_cursor_for_active(active_view_id: &str, subagents: &[SubagentItem]) -> usize {
    if active_view_id == "main" {
        return 0;
    }
    subagents
        .iter()
        .position(|a| a.full_id == active_view_id)
        .map(|idx| idx + 1)
        .unwrap_or(0)
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
    let mut pal_watcher = PaletteWatcher::new(&home);
    let mut telemetry = TelemetrySampler::new(48);
    let mut sub_trackers: HashMap<String, JsonlTracker> = HashMap::new();
    let mut main_tracker: HashMap<String, JsonlTracker> = HashMap::new();
    let mut seen_sub_seq: HashMap<String, usize> = HashMap::new();
    let mut frame_buf = String::with_capacity(16384);
    let mut prev_rows: Vec<String> = Vec::new();
    let mut row_tag_map: HashMap<usize, (u8, usize)> = HashMap::new();

    let mut active_view_id = "main".to_string();
    let mut cursor_idx: Option<usize> = None;
    let mut sub_scroll_offset = 0usize;
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
    if cached_claude_pid > 1 {
        let _ = fs::write(
            format!("/tmp/.cc-ide/sb_pid_{}.pid", cached_claude_pid),
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

        // Live hot-reload `~/.config/cc-ide/palette.json` whenever its mtime changes (`1` stat syscall)
        if (is_dirty || tick % 2 == 0) && pal_watcher.reload_if_changed(&home) {
            prev_rows.clear();
        }
        let pal = &pal_watcher.palette;
        let white = &pal.text;
        let dim = &pal.dim;
        let cyan = &pal.cyan;
        let green = &pal.green;
        let amber = &pal.amber;
        let mint = &pal.mint;

        // Strictly event-driven: only query session/transcripts on `SIGUSR2` hook/state events,
        // mouse interaction, or while a subagent is actively running (with a 30s safety fallback).
        if is_dirty || has_running_sub || cached_session_id.is_empty() || tick % 30 == 0 {
            if cached_session_id.is_empty() || is_dirty || tick % 30 == 0 {
                let (sid, cpid) =
                    resolve_isolated_session_id(&home, tab_yazi_id, target_cwd, shell_pid_arg);
                if !sid.is_empty() {
                    if sid != cached_session_id {
                        sub_trackers.clear();
                        seen_sub_seq.clear();
                        let _ = fs::write(format!("/tmp/.cc-ide/sb_{}.pid", sid), &my_pid_str);
                    }
                    if cpid > 1 && cpid != cached_claude_pid {
                        let _ = fs::write(format!("/tmp/.cc-ide/sb_pid_{}.pid", cpid), &my_pid_str);
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

            // Check `/tmp/.cc-ide/cur_pid_<pid>.txt` or `cur_<sessionId>.txt` pushed by `claude.exe`
            let pid_cur = if cached_claude_pid > 1 {
                fs::read_to_string(format!("/tmp/.cc-ide/cur_pid_{}.txt", cached_claude_pid)).ok()
            } else {
                None
            };
            let sess_cur = if !cached_session_id.is_empty() {
                fs::read_to_string(format!("/tmp/.cc-ide/cur_{}.txt", cached_session_id)).ok()
            } else {
                None
            };
            if let Some(cur_txt) = pid_cur.or(sess_cur) {
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

            cached_cfg =
                load_config_models(&home, &cached_session_id, tab_yazi_id, &transcript_model);
            subagents =
                get_session_subagents(&home, &cached_session_id, target_cwd, &mut sub_trackers);

            for ag in &subagents {
                let prev = seen_sub_seq.insert(ag.full_id.clone(), ag.file_op_seq);
                if let Some(p_seq) = prev {
                    if ag.file_op_seq > p_seq && active_view_id == ag.full_id {
                        trigger_yazi_reveal(tab_yazi_id, &ag.file);
                    }
                }
            }
        }

        // ── 1. TOP PANEL: MODEL LOGO ──
        let is_main_active = active_view_id == "main";
        let (top_family, top_badge) = if is_main_active {
            (cached_cfg.main_family, cached_cfg.main_model_short.clone())
        } else {
            (cached_cfg.sub_family, cached_cfg.sub_model_short.clone())
        };
        let (top_panel, top_h) = build_top_logo_panel(geom, &logos, top_family, top_badge, pal);

        // Compute exact panel heights upfront so `SUBAGENTS` knows its exact scrollable viewport
        let rem_after_top = geom.rows.saturating_sub(top_h);
        let bot_h = ((rem_after_top as f64) * (1.12 / 2.0)).round() as usize;
        let bot_h = bot_h.clamp(12, rem_after_top.saturating_sub(6).max(12));
        let mid_h = rem_after_top.saturating_sub(bot_h).max(4);
        let mid_interior = mid_h.saturating_sub(2);

        // ── 2. MIDDLE PANEL: SUBAGENTS (Pinned `[main]` + Arrow-Key Cursor + Scrollable List) ──
        let total_selectable = 1 + subagents.len();
        if let Some(cur) = cursor_idx {
            if cur >= total_selectable {
                cursor_idx = Some(total_selectable.saturating_sub(1));
            }
        }

        let box_inner_w = geom.cols.saturating_sub(2).max(12);
        let running_cnt = subagents.iter().filter(|a| a.status == "running").count();
        let done_cnt = subagents.len().saturating_sub(running_cnt);
        let mut sub_rows = Vec::with_capacity(mid_interior.max(4));

        let is_main_cursor = cursor_idx == Some(0);
        let main_bg: &str = match (is_main_active, is_main_cursor) {
            (true, true) => &pal.active_cursor_bg,
            (true, false) => &pal.active_view_bg,
            (false, true) => &pal.cursor_bg,
            (false, false) => "",
        };
        let main_ptr = if is_main_cursor {
            format!("{amber}❯{RESET}{main_bg}")
        } else if is_main_active {
            format!("{cyan}▸{RESET}{main_bg}")
        } else {
            " ".to_string()
        };
        let main_dot = if is_main_active {
            format!("{green}●{RESET}{main_bg}")
        } else {
            format!("{dim}○{RESET}{main_bg}")
        };
        let mut main_tail = if is_main_active {
            format!(
                "{amber}{}{RESET}{main_bg}",
                if main_tool.is_empty() {
                    "chat"
                } else {
                    &main_tool
                }
            )
        } else {
            format!(
                "{dim}{}{RESET}{main_bg}",
                if main_tool.is_empty() {
                    "chat"
                } else {
                    &main_tool
                }
            )
        };
        if !main_file.is_empty() {
            let bname = Path::new(&main_file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            main_tail.push_str(&format!(
                " {cyan}{}{RESET}{main_bg}",
                trunc_to_width(bname, 15)
            ));
        }
        sub_rows.push(BoxRow {
            text: format!("{main_ptr}{main_dot} {BOLD}{white}[main]{RESET}{main_bg} {main_tail}"),
            bg: main_bg.to_string(),
            tag: Some((1, 0)),
        });

        let cwd_base = Path::new(target_cwd)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("~");
        let main_sub_txt = if is_main_active {
            format!("{} · {}", cached_cfg.main_model_short, cwd_base)
        } else if is_main_cursor {
            format!("{} (Enter to switch)", cached_cfg.main_model_short)
        } else {
            format!("{} (click / ↑↓)", cached_cfg.main_model_short)
        };
        sub_rows.push(BoxRow {
            text: format!(
                "   {dim}{}",
                trunc_to_width(&main_sub_txt, inner_w.saturating_sub(1))
            ),
            bg: main_bg.to_string(),
            tag: Some((1, 0)),
        });

        let mut all_agent_rows = Vec::with_capacity(subagents.len() * 2);
        for (i_ag, ag) in subagents.iter().enumerate() {
            let is_viewed = ag.full_id == active_view_id;
            let is_cur = cursor_idx == Some(i_ag + 1);
            let row_bg: &str = match (is_viewed, is_cur) {
                (true, true) => &pal.active_cursor_bg,
                (true, false) => &pal.active_view_bg,
                (false, true) => &pal.cursor_bg,
                (false, false) => "",
            };
            let ptr = if is_cur {
                format!("{amber}❯{RESET}{row_bg}")
            } else if is_viewed {
                format!("{cyan}▸{RESET}{row_bg}")
            } else {
                " ".to_string()
            };
            let is_done = ag.status != "running";
            let (icon, mut tail_tag) = if !is_done {
                (
                    format!("{cyan}{}{RESET}{row_bg}", SPINNERS[tick % SPINNERS.len()]),
                    format!(
                        "{amber}{}{RESET}{row_bg}",
                        if ag.tool.is_empty() { "run" } else { &ag.tool }
                    ),
                )
            } else {
                let ic = if is_viewed {
                    format!("{green}●{RESET}{row_bg}")
                } else {
                    format!("{green}✓{RESET}{row_bg}")
                };
                let tc: &str = if is_viewed || is_cur { amber } else { dim };
                (
                    ic,
                    format!(
                        "{tc}{}{RESET}{row_bg}",
                        if ag.tool.is_empty() { "done" } else { &ag.tool }
                    ),
                )
            };

            let prefix_plain_w = 4
                + display_width(&ag.short_type)
                + 1
                + display_width(if ag.tool.is_empty() {
                    if is_done {
                        "done"
                    } else {
                        "run"
                    }
                } else {
                    &ag.tool
                });
            let right_reserve = if is_done { 3 } else { 1 };
            let max_file_w = box_inner_w.saturating_sub(prefix_plain_w + right_reserve + 1);

            if !ag.file.is_empty() && max_file_w >= 4 {
                let bname = Path::new(&ag.file)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                tail_tag.push_str(&format!(
                    " {cyan}{}{RESET}{row_bg}",
                    trunc_to_width(bname, max_file_w.min(13))
                ));
            }

            let left_hdr = format!(
                "{ptr}{icon} {BOLD}{white}[{}]{RESET}{row_bg} {tail_tag}",
                ag.short_type
            );
            let hdr_text = if is_done {
                let lw = display_width(&left_hdr);
                if lw + 2 <= box_inner_w {
                    let pad = box_inner_w - lw - 2;
                    let x_col: &str = if is_cur { amber } else { dim };
                    format!("{left_hdr}{}{x_col}×{RESET}{row_bg} ", " ".repeat(pad))
                } else {
                    left_hdr
                }
            } else {
                left_hdr
            };

            // Tag `2` = subagent header row (has right-edge clickable `×`), Tag `5` = subagent desc row
            all_agent_rows.push(BoxRow {
                text: hdr_text,
                bg: row_bg.to_string(),
                tag: Some((2, i_ag)),
            });
            let desc_line = format!("{} · {}", cached_cfg.sub_model_short, ag.desc);
            all_agent_rows.push(BoxRow {
                text: format!(
                    "   {dim}{}",
                    trunc_to_width(&desc_line, inner_w.saturating_sub(1))
                ),
                bg: row_bg.to_string(),
                tag: Some((5, i_ag)),
            });
        }

        let show_clear_bar = done_cnt > 0 && mid_interior >= 5;
        let reserved_rows = 2 + usize::from(show_clear_bar);
        let viewport_h = mid_interior.saturating_sub(reserved_rows);
        let max_scroll = all_agent_rows.len().saturating_sub(viewport_h);
        if sub_scroll_offset > max_scroll {
            sub_scroll_offset = max_scroll;
        }

        for r in all_agent_rows
            .into_iter()
            .skip(sub_scroll_offset)
            .take(viewport_h)
        {
            sub_rows.push(r);
        }

        if show_clear_bar {
            while sub_rows.len() + 1 < mid_interior {
                sub_rows.push(BoxRow {
                    text: String::new(),
                    bg: String::new(),
                    tag: None,
                });
            }
            let clear_plain = format!(" ↑↓ ⏎ · x del · c clear {}", done_cnt);
            let clear_styled = if display_width(&clear_plain) <= box_inner_w {
                format!(
                    " {dim}↑↓ ⏎ · [{amber}x{dim}] del · [{amber}c{dim}] clear {}{RESET}",
                    done_cnt
                )
            } else {
                format!(" {dim}[{amber}c{dim}] × clear {} done{RESET}", done_cnt)
            };
            sub_rows.push(BoxRow {
                text: if display_width(&clear_styled) <= box_inner_w {
                    clear_styled
                } else {
                    trunc_to_width(&clear_plain, inner_w)
                },
                bg: String::new(),
                tag: Some((3, 0)),
            });
        }

        let sub_badge = if max_scroll > 0 {
            let first_item = (sub_scroll_offset / 2) + 1;
            let vis_items = (viewport_h / 2).max(1);
            let last_item = (first_item + vis_items - 1).min(subagents.len());
            if running_cnt > 0 {
                format!(
                    "{}run · ↕{}-{}/{}",
                    running_cnt,
                    first_item,
                    last_item,
                    subagents.len()
                )
            } else {
                format!("↕ {}-{}/{}", first_item, last_item, subagents.len())
            }
        } else if running_cnt == 0 {
            format!("main+{}", subagents.len())
        } else {
            format!("{} run/{}", running_cnt, subagents.len())
        };
        let mid_panel = PanelSpec {
            title: "SUBAGENTS".to_string(),
            badge: sub_badge,
            accent: cyan.clone(),
            rows: sub_rows,
            fixed_height: Some(mid_h),
            weight: 0.88,
            raw_interior: false,
        };

        // ── 3. BOTTOM PANEL: SHARED SINGLE-INSTANCE TELEMETRY OSCILLOSCOPE CURVES ──
        telemetry.sample();
        let bot_rows = build_telemetry_panel_rows(&telemetry, inner_w, bot_h, pal);
        let bot_panel = PanelSpec {
            title: telemetry.cpu_brand.to_uppercase(),
            badge: format!("{}C · {:.0}GB", telemetry.cpu_cores, telemetry.total_mem_gb),
            accent: mint.clone(),
            rows: bot_rows,
            fixed_height: Some(bot_h),
            weight: 1.12,
            raw_interior: false,
        };

        let panels = [top_panel, mid_panel, bot_panel];
        render_multi_panel_grid(
            geom,
            &panels,
            pal,
            &mut frame_buf,
            &mut row_tag_map,
            &mut prev_rows,
        );
        if !frame_buf.is_empty() {
            let _ = stdout.write_all(frame_buf.as_bytes());
            let _ = stdout.flush();
        }

        // Sleep until `SIGUSR2` event, mouse/key input on `stdin_fd`, or 1000ms telemetry tick
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
                let mut clear_all_done = false;
                let mut dismiss_single_idx: Option<usize> = None;

                for ev in parse_input_events(&raw_in) {
                    let max_idx = subagents.len();
                    let cur_max_scroll = (subagents.len() * 2).saturating_sub(viewport_h);

                    match ev {
                        InputEvent::Escape => {
                            cursor_idx = None;
                            if active_view_id != "main" {
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
                        }
                        InputEvent::CursorUp => {
                            let base = cursor_idx.unwrap_or_else(|| {
                                default_cursor_for_active(&active_view_id, &subagents)
                            });
                            let next = base.saturating_sub(1);
                            cursor_idx = Some(next);
                            sync_scroll_to_cursor(
                                next,
                                &mut sub_scroll_offset,
                                viewport_h,
                                cur_max_scroll,
                            );
                        }
                        InputEvent::CursorDown => {
                            let base = cursor_idx.unwrap_or_else(|| {
                                default_cursor_for_active(&active_view_id, &subagents)
                            });
                            let next = (base + 1).min(max_idx);
                            cursor_idx = Some(next);
                            sync_scroll_to_cursor(
                                next,
                                &mut sub_scroll_offset,
                                viewport_h,
                                cur_max_scroll,
                            );
                        }
                        InputEvent::PageUp => {
                            let step = (viewport_h / 2).max(1);
                            let base = cursor_idx.unwrap_or_else(|| {
                                default_cursor_for_active(&active_view_id, &subagents)
                            });
                            let next = base.saturating_sub(step);
                            cursor_idx = Some(next);
                            if next == 0 {
                                sub_scroll_offset = 0;
                            } else {
                                sync_scroll_to_cursor(
                                    next,
                                    &mut sub_scroll_offset,
                                    viewport_h,
                                    cur_max_scroll,
                                );
                            }
                        }
                        InputEvent::PageDown => {
                            let step = (viewport_h / 2).max(1);
                            let base = cursor_idx.unwrap_or_else(|| {
                                default_cursor_for_active(&active_view_id, &subagents)
                            });
                            let next = (base + step).min(max_idx);
                            cursor_idx = Some(next);
                            sync_scroll_to_cursor(
                                next,
                                &mut sub_scroll_offset,
                                viewport_h,
                                cur_max_scroll,
                            );
                        }
                        InputEvent::Home => {
                            cursor_idx = Some(0);
                            sub_scroll_offset = 0;
                        }
                        InputEvent::End => {
                            cursor_idx = Some(max_idx);
                            sub_scroll_offset = cur_max_scroll;
                        }
                        InputEvent::ActivateSelected => {
                            let target = cursor_idx.unwrap_or_else(|| {
                                default_cursor_for_active(&active_view_id, &subagents)
                            });
                            cursor_idx = Some(target);
                            if target == 0 {
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
                            } else if let Some(ag) = subagents.get(target - 1) {
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
                        InputEvent::DismissSelected => {
                            let target = cursor_idx.unwrap_or_else(|| {
                                default_cursor_for_active(&active_view_id, &subagents)
                            });
                            if target >= 1 {
                                dismiss_single_idx = Some(target - 1);
                            }
                        }
                        InputEvent::ClearAllDone => {
                            clear_all_done = true;
                        }
                        InputEvent::WheelUp => {
                            sub_scroll_offset = sub_scroll_offset.saturating_sub(2);
                        }
                        InputEvent::WheelDown => {
                            sub_scroll_offset = (sub_scroll_offset + 2).min(cur_max_scroll);
                        }
                        InputEvent::SecondaryClick { row: r_clicked } => {
                            if let Some(&(tkind, tidx)) = row_tag_map.get(&r_clicked) {
                                if tkind == 2 || tkind == 5 {
                                    dismiss_single_idx = Some(tidx);
                                }
                            }
                        }
                        InputEvent::LeftClick {
                            col: c_clicked,
                            row: r_clicked,
                        } => {
                            if let Some(&(tkind, tidx)) = row_tag_map.get(&r_clicked) {
                                if tkind == 1 {
                                    cursor_idx = Some(0);
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
                                } else if tkind == 2 || tkind == 5 {
                                    // Left-clicking the right-aligned `×` on a completed subagent's header row dismisses it directly
                                    let clicked_close_x = tkind == 2
                                        && c_clicked >= geom.cols.saturating_sub(4)
                                        && subagents
                                            .get(tidx)
                                            .map(|a| a.status != "running")
                                            .unwrap_or(false);
                                    if clicked_close_x {
                                        dismiss_single_idx = Some(tidx);
                                    } else if let Some(ag) = subagents.get(tidx) {
                                        cursor_idx = Some(tidx + 1);
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
                                } else if tkind == 3 {
                                    clear_all_done = true;
                                }
                            }
                        }
                    }

                    if let Some(d_idx) = dismiss_single_idx.take() {
                        if let Some(ag) = subagents.get(d_idx) {
                            if ag.status != "running" {
                                let dismissed_id = ag.full_id.clone();
                                append_dismissed_subagents(
                                    &home,
                                    &cached_session_id,
                                    std::slice::from_ref(&dismissed_id),
                                );
                                subagents.remove(d_idx);
                                if let Some(cur) = cursor_idx {
                                    if cur > subagents.len() {
                                        cursor_idx = Some(subagents.len());
                                    }
                                }
                                if active_view_id == dismissed_id {
                                    active_view_id = "main".to_string();
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
                            }
                        }
                    }
                }

                if clear_all_done {
                    let done_ids: Vec<String> = subagents
                        .iter()
                        .filter(|a| a.status != "running")
                        .map(|a| a.full_id.clone())
                        .collect();
                    if !done_ids.is_empty() {
                        let need_reset_main = done_ids.iter().any(|id| id == &active_view_id);
                        append_dismissed_subagents(&home, &cached_session_id, &done_ids);
                        subagents.retain(|a| a.status == "running");
                        sub_scroll_offset = 0;
                        if let Some(cur) = cursor_idx {
                            if cur > subagents.len() {
                                cursor_idx = Some(subagents.len());
                            }
                        }
                        if need_reset_main {
                            active_view_id = "main".to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sgr_wheel_and_click_events() {
        let stream = "\x1b[<64;10;18M\x1b[<65;10;18M\x1b[<0;37;14M\x1b[<0;37;14m\x1b[<2;8;16M";
        let parsed = parse_input_events(stream);
        assert_eq!(
            parsed,
            vec![
                InputEvent::WheelUp,
                InputEvent::WheelDown,
                InputEvent::LeftClick { col: 37, row: 14 },
                InputEvent::SecondaryClick { row: 16 },
            ]
        );
    }

    #[test]
    fn parses_arrow_bursts_and_keyboard_shortcuts() {
        let stream = "\x1b[B\x1bOB\r\x1b[3~c\x1b";
        let parsed = parse_input_events(stream);
        assert_eq!(
            parsed,
            vec![
                InputEvent::CursorDown,
                InputEvent::CursorDown,
                InputEvent::ActivateSelected,
                InputEvent::DismissSelected,
                InputEvent::ClearAllDone,
                InputEvent::Escape,
            ]
        );
    }

    #[test]
    fn sync_scroll_tracks_cursor_into_viewport() {
        let mut off = 0usize;
        // Viewport holds 6 rows (3 subagents). Cursor 1..=3 fits at offset 0.
        sync_scroll_to_cursor(3, &mut off, 6, 14);
        assert_eq!(off, 0);

        // Moving to subagent #4 (`cursor_idx == 4`, rows 6..8) scrolls down to offset 2.
        sync_scroll_to_cursor(4, &mut off, 6, 14);
        assert_eq!(off, 2);

        // Moving back up to subagent #1 (`cursor_idx == 1`, rows 0..2) snaps back to 0.
        sync_scroll_to_cursor(1, &mut off, 6, 14);
        assert_eq!(off, 0);
    }
}
