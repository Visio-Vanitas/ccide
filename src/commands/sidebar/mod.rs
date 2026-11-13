//! `ccide-core sidebar`: Event-Driven Interactive Right Sidebar Daemon (`src/commands/sidebar/`).

pub mod input;
pub mod ipc;
pub mod subagents_panel;

use std::collections::HashMap;
use std::env;
use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::commands::workspace::focus_center_after_view_switch;
use crate::ffi::{
    handle_sig, handle_sigusr2, poll, read, signal, tcgetattr, tcsetattr, PollFd, Termios,
    DIRTY_EVENT, RUNNING, SIGUSR2,
};
use crate::session::transcript::{
    append_dismissed_subagents, get_main_session_file_ops, get_session_subagents, JsonlTracker,
    SubagentItem,
};
use crate::session::{load_config_models, resolve_isolated_session_id, ConfigModels};
use crate::telemetry::chart::build_telemetry_panel_rows;
use crate::telemetry::TelemetrySampler;
use crate::tui::context::{
    build_context_panel, build_cost_panel, SessionContextAnalyzer, TAG_CTX_TOGGLE,
};
use crate::tui::grid::{get_terminal_geometry, render_multi_panel_grid, PanelSpec};
use crate::tui::logo::{build_top_logo_panel, load_adaptive_logos};
use crate::tui::palette::PaletteWatcher;

use input::{default_cursor_for_active, parse_input_events, sync_scroll_to_cursor, InputEvent};
use ipc::{
    read_pushed_claude_state, register_sidebar_tab, trigger_native_view_switch,
    trigger_yazi_reveal, write_active_view_state,
};
use subagents_panel::build_subagents_panel;

pub fn run(target_cwd: &str, tab_yazi_id: &str, shell_pid_arg: u32) {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let _ = fs::create_dir_all(format!("{}/.cache/cc-ide", home));
    let _ = fs::create_dir_all("/tmp/.cc-ide");
    let my_pid_str = std::process::id().to_string();

    register_sidebar_tab(&home, tab_yazi_id, target_cwd, shell_pid_arg, &my_pid_str);

    unsafe {
        signal(2, handle_sig);
        signal(15, handle_sig);
        signal(SIGUSR2, handle_sigusr2);
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
        b"\x1b]1;[AI] Companion & Agents\x07\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[49m\x1b[2J\x1b[3J",
    );
    let _ = stdout.flush();

    let mut logos = load_adaptive_logos(&home);
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
    let mut cost_scroll_offset = 0usize;
    let mut cost_max_scroll = 0usize;
    let mut ctx_analyzer = SessionContextAnalyzer::new();
    let mut show_context_panel = false;
    let mut seen_main_seq: Option<usize> = None;
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
    let mut main_line = 0usize;
    let mut main_span = 0usize;
    let mut subagents: Vec<SubagentItem> = Vec::new();
    let mut cached_cfg: ConfigModels =
        load_config_models(&home, &cached_session_id, tab_yazi_id, None, None);
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

        if is_dirty || tick % 2 == 0 {
            let pal_changed = pal_watcher.reload_if_changed(&home);
            let sprite_changed = logos.refresh_mode_if_changed(&home);
            if pal_changed || sprite_changed {
                prev_rows.clear();
            }
        }
        let pal = &pal_watcher.palette;

        if is_dirty || has_running_sub || cached_session_id.is_empty() || tick % 30 == 0 {
            if cached_session_id.is_empty() || is_dirty || tick % 30 == 0 {
                let (sid, cpid) =
                    resolve_isolated_session_id(&home, tab_yazi_id, target_cwd, shell_pid_arg);
                if !sid.is_empty() {
                    if sid != cached_session_id {
                        sub_trackers.clear();
                        seen_sub_seq.clear();
                        seen_main_seq = None;
                        ctx_analyzer.clear();
                        cost_scroll_offset = 0;
                        let _ = fs::write(format!("/tmp/.cc-ide/sb_{}.pid", sid), &my_pid_str);
                    }
                    if cpid > 1 && cpid != cached_claude_pid {
                        let _ = fs::write(format!("/tmp/.cc-ide/sb_pid_{}.pid", cpid), &my_pid_str);
                    }
                    cached_session_id = sid;
                    cached_claude_pid = cpid;
                }
            }

            let m_ops = get_main_session_file_ops(
                &home,
                &cached_session_id,
                tab_yazi_id,
                target_cwd,
                &mut main_tracker,
            );
            main_tool = m_ops.tool;
            main_file = m_ops.file;
            main_line = m_ops.line;
            main_span = m_ops.span;
            let transcript_model = m_ops.model;
            let transcript_mtime = m_ops.model_mtime;
            let main_seq = m_ops.seq;
            if let Some(prev_seq) = seen_main_seq {
                if main_seq > prev_seq && active_view_id == "main" {
                    trigger_yazi_reveal(
                        &home,
                        tab_yazi_id,
                        target_cwd,
                        &main_file,
                        main_line,
                        main_span,
                    );
                }
            }
            seen_main_seq = Some(main_seq);

            let mut pushed_model_cand: Option<(std::time::SystemTime, String)> = None;
            if let Some((target_cur, p_mod)) =
                read_pushed_claude_state(cached_claude_pid, &cached_session_id)
            {
                pushed_model_cand = p_mod;
                if last_click_instant.elapsed() > Duration::from_millis(600)
                    && target_cur != active_view_id
                {
                    active_view_id = target_cur;
                    write_active_view_state(&home, tab_yazi_id, &active_view_id);
                    if active_view_id == "main" {
                        trigger_yazi_reveal(
                            &home,
                            tab_yazi_id,
                            target_cwd,
                            &main_file,
                            main_line,
                            main_span,
                        );
                    }
                }
            }

            let t_cand = (!transcript_model.is_empty())
                .then_some((transcript_mtime, transcript_model.as_str()));
            let p_cand = pushed_model_cand.as_ref().map(|(mt, s)| (*mt, s.as_str()));
            cached_cfg = load_config_models(&home, &cached_session_id, tab_yazi_id, t_cand, p_cand);
            subagents =
                get_session_subagents(&home, &cached_session_id, target_cwd, &mut sub_trackers);

            for ag in &subagents {
                let prev = seen_sub_seq.insert(ag.full_id.clone(), ag.file_op_seq);
                if let Some(p_seq) = prev {
                    if ag.file_op_seq > p_seq && active_view_id == ag.full_id {
                        trigger_yazi_reveal(
                            &home,
                            tab_yazi_id,
                            target_cwd,
                            &ag.file,
                            ag.line,
                            ag.span,
                        );
                    }
                }
            }
        }

        let is_main_active = active_view_id == "main";
        let (top_family, top_badge) = if is_main_active {
            (cached_cfg.main_family, cached_cfg.main_model_short.clone())
        } else {
            (cached_cfg.sub_family, cached_cfg.sub_model_short.clone())
        };
        let (top_panel, top_h) = build_top_logo_panel(geom, &logos, top_family, top_badge, pal);

        let rem_after_top = geom.rows.saturating_sub(top_h);
        let bot_h = ((rem_after_top as f64) * (1.12 / 2.0)).round() as usize;
        let bot_h = bot_h.clamp(12, rem_after_top.saturating_sub(6).max(12));
        let mid_h = rem_after_top.saturating_sub(bot_h).max(4);

        let total_selectable = 1 + subagents.len();
        if let Some(cur) = cursor_idx {
            if cur >= total_selectable {
                cursor_idx = Some(total_selectable.saturating_sub(1));
            }
        }

        let sub_out = build_subagents_panel(
            geom.cols,
            mid_h,
            target_cwd,
            &active_view_id,
            cursor_idx,
            &mut sub_scroll_offset,
            &main_tool,
            &main_file,
            &subagents,
            &cached_cfg,
            tick,
            pal,
        );
        let running_cnt = sub_out.running_cnt;
        let viewport_h = sub_out.viewport_h;

        telemetry.sample();
        let ctx_snap = ctx_analyzer.sample(
            &home,
            target_cwd,
            &cached_session_id,
            &active_view_id,
            &cached_cfg.main_model_short,
            &cached_cfg.sub_model_short,
        );

        let mut panels = Vec::with_capacity(4);
        panels.push(top_panel);
        panels.push(sub_out.panel);

        let split_ctx_cost = show_context_panel && bot_h >= 18;
        let ctx_box_h = if split_ctx_cost {
            (bot_h + 1) / 2
        } else {
            bot_h
        };
        if show_context_panel {
            if split_ctx_cost {
                let cost_h = bot_h - ctx_box_h;
                panels.push(build_context_panel(
                    &ctx_snap, geom.cols, ctx_box_h, false, pal,
                ));
                let (cost_panel, max_sc) =
                    build_cost_panel(&ctx_snap, geom.cols, cost_h, cost_scroll_offset, pal);
                cost_max_scroll = max_sc;
                cost_scroll_offset = cost_scroll_offset.min(cost_max_scroll);
                panels.push(cost_panel);
            } else {
                cost_max_scroll = 0;
                panels.push(build_context_panel(&ctx_snap, geom.cols, bot_h, true, pal));
            }
        } else {
            let bot_rows = build_telemetry_panel_rows(&telemetry, inner_w, bot_h, pal);
            panels.push(PanelSpec {
                title: telemetry.cpu_brand.to_uppercase(),
                badge: format!(
                    "{}C·{:.0}G [CTX {}%]",
                    telemetry.cpu_cores, telemetry.total_mem_gb, ctx_snap.budget.used_pct
                ),
                accent: pal.mint.clone(),
                rows: bot_rows,
                fixed_height: Some(bot_h),
                weight: 1.12,
                raw_interior: false,
            });
        }

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
                let mut restore_center_focus = false;
                let mut switched_view_in_batch = false;
                let panel3_top_row = top_h + mid_h + 1;
                let cost_top_row = panel3_top_row + ctx_box_h;

                for ev in parse_input_events(&raw_in) {
                    let max_idx = subagents.len();
                    let cur_max_scroll = (subagents.len() * 2).saturating_sub(viewport_h);

                    match ev {
                        InputEvent::ToggleBottomPanel => show_context_panel = !show_context_panel,
                        InputEvent::Escape => {
                            if show_context_panel
                                && active_view_id == "main"
                                && cursor_idx.is_none()
                            {
                                show_context_panel = false;
                            } else {
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
                                    trigger_yazi_reveal(
                                        &home,
                                        tab_yazi_id,
                                        target_cwd,
                                        &main_file,
                                        main_line,
                                        main_span,
                                    );
                                }
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
                                trigger_yazi_reveal(
                                    &home,
                                    tab_yazi_id,
                                    target_cwd,
                                    &main_file,
                                    main_line,
                                    main_span,
                                );
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
                                trigger_yazi_reveal(
                                    &home,
                                    tab_yazi_id,
                                    target_cwd,
                                    &ag.file,
                                    ag.line,
                                    ag.span,
                                );
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
                        InputEvent::ClearAllDone => clear_all_done = true,
                        InputEvent::WheelUp { row: r_wheel } => {
                            if r_wheel < panel3_top_row {
                                sub_scroll_offset = sub_scroll_offset.saturating_sub(2);
                            } else if split_ctx_cost && r_wheel >= cost_top_row {
                                cost_scroll_offset = cost_scroll_offset.saturating_sub(1);
                            }
                        }
                        InputEvent::WheelDown { row: r_wheel } => {
                            if r_wheel < panel3_top_row {
                                sub_scroll_offset = (sub_scroll_offset + 2).min(cur_max_scroll);
                            } else if split_ctx_cost && r_wheel >= cost_top_row {
                                cost_scroll_offset = (cost_scroll_offset + 1).min(cost_max_scroll);
                            }
                        }
                        InputEvent::SecondaryClick { row: r_clicked } => {
                            restore_center_focus = true;
                            if r_clicked > top_h && r_clicked >= panel3_top_row {
                                show_context_panel = !show_context_panel;
                            } else if r_clicked > top_h {
                                if let Some(&(tkind, tidx)) = row_tag_map.get(&r_clicked) {
                                    if tkind == 2 || tkind == 5 {
                                        dismiss_single_idx = Some(tidx);
                                    }
                                }
                            }
                        }
                        InputEvent::LeftClick {
                            col: c_clicked,
                            row: r_clicked,
                        } => {
                            if r_clicked <= top_h {
                                restore_center_focus = true;
                            } else if r_clicked >= panel3_top_row {
                                restore_center_focus = true;
                                let clicked_hw_badge = !show_context_panel
                                    && r_clicked == panel3_top_row
                                    && c_clicked >= geom.cols.saturating_sub(14);
                                let clicked_ctx_hdr = show_context_panel
                                    && (r_clicked == panel3_top_row
                                        || (split_ctx_cost && r_clicked == cost_top_row)
                                        || row_tag_map
                                            .get(&r_clicked)
                                            .map(|&(k, _)| k == TAG_CTX_TOGGLE)
                                            .unwrap_or(false));
                                if clicked_hw_badge || clicked_ctx_hdr {
                                    show_context_panel = !show_context_panel;
                                }
                            } else if let Some(&(tkind, tidx)) = row_tag_map.get(&r_clicked) {
                                if tkind == 1 {
                                    cursor_idx = Some(0);
                                    active_view_id = "main".to_string();
                                    last_click_instant = Instant::now();
                                    switched_view_in_batch = true;
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
                                    trigger_yazi_reveal(
                                        &home,
                                        tab_yazi_id,
                                        target_cwd,
                                        &main_file,
                                        main_line,
                                        main_span,
                                    );
                                } else if tkind == 2 || tkind == 5 {
                                    let clicked_close_x = tkind == 2
                                        && c_clicked >= geom.cols.saturating_sub(4)
                                        && subagents
                                            .get(tidx)
                                            .map(|a| a.status != "running")
                                            .unwrap_or(false);
                                    if clicked_close_x {
                                        dismiss_single_idx = Some(tidx);
                                        restore_center_focus = true;
                                    } else if let Some(ag) = subagents.get(tidx) {
                                        cursor_idx = Some(tidx + 1);
                                        active_view_id = ag.full_id.clone();
                                        last_click_instant = Instant::now();
                                        switched_view_in_batch = true;
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
                                        trigger_yazi_reveal(
                                            &home,
                                            tab_yazi_id,
                                            target_cwd,
                                            &ag.file,
                                            ag.line,
                                            ag.span,
                                        );
                                    }
                                } else if tkind == 3 {
                                    clear_all_done = true;
                                    restore_center_focus = true;
                                } else {
                                    restore_center_focus = true;
                                }
                            } else {
                                restore_center_focus = true;
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
                                    trigger_yazi_reveal(
                                        &home,
                                        tab_yazi_id,
                                        target_cwd,
                                        &main_file,
                                        main_line,
                                        main_span,
                                    );
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
                            switched_view_in_batch = true;
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
                            trigger_yazi_reveal(
                                &home,
                                tab_yazi_id,
                                target_cwd,
                                &main_file,
                                main_line,
                                main_span,
                            );
                        }
                    }
                }

                if restore_center_focus && !switched_view_in_batch {
                    focus_center_after_view_switch(
                        &home,
                        tab_yazi_id,
                        &cached_session_id,
                        &active_view_id,
                        0,
                    );
                }
            }
        }

        tick = tick.wrapping_add(1);
    }

    let _ = stdout.write_all(b"\x1b[?1000l\x1b[?1002l\x1b[?1006l\x1b[?1049l\x1b[?25h\x1b[0m\n");
    let _ = stdout.flush();
    if has_termios {
        unsafe {
            let _ = tcsetattr(stdin_fd, 0, &orig_termios);
        }
    }
}
