//! `ccide-core sidebar`: Event-Driven Interactive Right Sidebar Daemon (`src/commands/sidebar/`).

pub mod bottom_panel;
pub mod dispatch;
pub mod hit_mask;
pub mod input;
pub mod ipc;
pub mod mouse_actions;
pub mod orch_panel;
pub mod subagents_panel;

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::sync::atomic::Ordering;

use crate::ffi::{install_signal_handlers, ConsoleRawSession, DIRTY_EVENT, RUNNING};
use crate::orchestrator::OrchestratorWatcher;
use crate::platform::{cache_dir, ipc_dir, ipc_path, user_home_dir};
use crate::session::transcript::SubagentItem;
use crate::session::{
    classify_model_family, load_config_models, resolve_isolated_session_id, ConfigModels,
};
use crate::telemetry::TelemetrySampler;
use crate::tui::context::{ContextSnapshot, SessionContextAnalyzer};
use crate::tui::grid::{get_terminal_geometry, render_multi_panel_grid};
use crate::tui::logo::{build_top_logo_panel, load_adaptive_logos};
use crate::tui::palette::PaletteWatcher;

use bottom_panel::append_bottom_panels;
use dispatch::{handle_input_batch, DispatchEnv, SidebarSelectState};
use hit_mask::{FocusZone, SidebarGeomMask};
use ipc::{
    refresh_session_and_subagents, register_sidebar_tab, update_isolated_session_binding,
    write_active_view_state,
};
use orch_panel::build_orchestrator_panel;
use subagents_panel::build_subagents_panel;

pub fn run(target_cwd: &str, tab_yazi_id: &str, shell_pid_arg: u32) {
    let home = user_home_dir();
    let _ = fs::create_dir_all(cache_dir(&home));
    let _ = fs::create_dir_all(ipc_dir());
    let my_pid_str = std::process::id().to_string();
    register_sidebar_tab(&home, tab_yazi_id, target_cwd, shell_pid_arg, &my_pid_str);

    install_signal_handlers(true);
    let console = ConsoleRawSession::enter();
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(
        b"\x1b]1;[AI] Companion & Agents\x07\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1004h\x1b[?1006h\x1b[49m\x1b[2J\x1b[3J",
    );
    let _ = stdout.flush();

    let mut logos = load_adaptive_logos(&home);
    let mut pal_watcher = PaletteWatcher::new(&home);
    let mut orch_watcher = OrchestratorWatcher::new(&home);
    let mut telemetry = TelemetrySampler::new(48);
    let (mut sub_trackers, mut main_tracker, mut seen_sub_seq) =
        (HashMap::new(), HashMap::new(), HashMap::new());
    let mut frame_buf = String::with_capacity(16384);
    let (mut prev_rows, mut row_tag_map) = (Vec::<String>::new(), HashMap::new());
    let mut sel = SidebarSelectState {
        cost_lines: logos.cost_lines,
        ..SidebarSelectState::default()
    };

    let mut active_view_id = "main".to_string();
    let mut cost_max_scroll = 0usize;
    let mut ctx_analyzer = SessionContextAnalyzer::new();
    let mut cached_ctx_snap: Option<ContextSnapshot> = None;
    let mut fast_input_redraw = false;
    let mut seen_main_seq: Option<usize> = None;
    let mut tick = 0usize;
    write_active_view_state(&home, tab_yazi_id, "main");

    let (mut c_sid, mut c_pid) =
        resolve_isolated_session_id(&home, tab_yazi_id, target_cwd, shell_pid_arg);
    if !c_sid.is_empty() {
        let _ = fs::write(ipc_path(&format!("sb_{c_sid}.pid")), &my_pid_str);
    }
    if c_pid > 1 {
        let _ = fs::write(ipc_path(&format!("sb_pid_{c_pid}.pid")), &my_pid_str);
    }

    let (mut main_tool, mut main_file) = (String::new(), String::new());
    let (mut main_line, mut main_span) = (0usize, 0usize);
    let mut subagents: Vec<SubagentItem> = Vec::new();
    let mut cached_cfg: ConfigModels = load_config_models(&home, &c_sid, tab_yazi_id, None, None);
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
        let is_dirty = !fast_input_redraw && DIRTY_EVENT.swap(false, Ordering::SeqCst);

        if !fast_input_redraw && (is_dirty || tick % 2 == 0) {
            if pal_watcher.reload_if_changed(&home) || logos.refresh_mode_if_changed(&home) {
                sel.cost_lines = logos.cost_lines;
                prev_rows.clear();
            }
            orch_watcher.reload_if_changed(&home);
        }
        let pal = &pal_watcher.palette;

        if !fast_input_redraw && (is_dirty || has_running_sub || c_sid.is_empty() || tick % 30 == 0)
        {
            if (c_sid.is_empty() || is_dirty || tick % 30 == 0)
                && update_isolated_session_binding(
                    &home,
                    tab_yazi_id,
                    target_cwd,
                    shell_pid_arg,
                    &my_pid_str,
                    &mut c_sid,
                    &mut c_pid,
                )
            {
                sub_trackers.clear();
                seen_sub_seq.clear();
                ctx_analyzer.clear();
                cached_ctx_snap = None;
                (seen_main_seq, sel.cost_scroll) = (None, 0);
            }
            (
                main_tool, main_file, main_line, main_span, cached_cfg, subagents,
            ) = refresh_session_and_subagents(
                &home,
                tab_yazi_id,
                target_cwd,
                &c_sid,
                c_pid,
                &mut active_view_id,
                &sel.last_click_instant,
                &mut main_tracker,
                &mut sub_trackers,
                &mut seen_main_seq,
                &mut seen_sub_seq,
            );
        }

        let is_main_active = active_view_id == "main";
        let active_sub = (!is_main_active)
            .then(|| subagents.iter().find(|a| a.full_id == active_view_id))
            .flatten();
        let (top_family, top_badge) = match (is_main_active, active_sub) {
            (true, _) => (cached_cfg.main_family, cached_cfg.main_model_short.clone()),
            (false, Some(ag)) if ag.model_raw.is_some() && ag.model_short.is_some() => (
                classify_model_family(ag.model_raw.as_deref().unwrap_or("")),
                ag.model_short.clone().unwrap_or_default(),
            ),
            _ => (cached_cfg.sub_family, cached_cfg.sub_model_short.clone()),
        };
        let (top_panel, top_h) = build_top_logo_panel(geom, &logos, top_family, top_badge, pal);

        let rem_after_top = geom.rows.saturating_sub(top_h);
        let bot_h = (((rem_after_top as f64) * 0.58).round() as usize)
            .clamp(12, rem_after_top.saturating_sub(6).max(12));
        let mid_h = rem_after_top.saturating_sub(bot_h).max(4);
        sel.sub_cursor = sel.sub_cursor.map(|c| c.min(subagents.len()));

        let sub_foc = sel.focus_zone == Some(FocusZone::Subagents);
        let (mid_panel, running_cnt, viewport_h) = if sel.show_orchestrator {
            let p = build_orchestrator_panel(
                geom.cols,
                mid_h,
                &orch_watcher.cfg,
                sel.orch_cursor,
                sub_foc,
                pal,
            );
            (
                p,
                subagents.iter().filter(|a| a.status == "running").count(),
                mid_h.saturating_sub(2),
            )
        } else {
            let o = build_subagents_panel(
                geom.cols,
                mid_h,
                target_cwd,
                &active_view_id,
                sel.sub_cursor,
                sub_foc,
                &mut sel.sub_scroll,
                &main_tool,
                &main_file,
                &subagents,
                &cached_cfg,
                &orch_watcher.cfg.active_preset,
                tick,
                pal,
            );
            (o.panel, o.running_cnt, o.viewport_h)
        };
        if !fast_input_redraw || cached_ctx_snap.is_none() {
            telemetry.sample();
            let active_sub_m = active_sub
                .and_then(|a| a.model_raw.as_deref())
                .unwrap_or(&cached_cfg.sub_model_short);
            cached_ctx_snap = Some(ctx_analyzer.sample(
                &home,
                target_cwd,
                &c_sid,
                c_pid,
                &active_view_id,
                &cached_cfg.main_model_short,
                active_sub_m,
            ));
        }
        let ctx_snap = cached_ctx_snap.as_ref().expect("ctx_snap initialized");

        let mut panels = vec![top_panel, mid_panel];
        let (split_ctx_cost, ctx_box_h) = append_bottom_panels(
            &mut panels,
            &mut sel,
            ctx_snap,
            &telemetry,
            geom.cols,
            inner_w,
            bot_h,
            &mut cost_max_scroll,
            pal,
        );

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

        let timeout_ms = if fast_input_redraw {
            fast_input_redraw = false;
            DIRTY_EVENT.store(true, Ordering::Relaxed);
            8
        } else if running_cnt > 0 {
            250
        } else {
            1000
        };
        let mut in_buf = [0u8; 1024];
        if let Some(n) = console.poll_stdin(&mut in_buf, timeout_ms) {
            fast_input_redraw = true;
            let panel3_top_row = top_h + mid_h + 1;
            let env = DispatchEnv {
                home: &home,
                yazi_id: tab_yazi_id,
                cwd: target_cwd,
                sid: &c_sid,
                cpid: c_pid,
                main_file: &main_file,
                main_line,
                main_span,
                viewport_h,
                cost_max_scroll,
                hit_mask: SidebarGeomMask {
                    cols: geom.cols,
                    top_h,
                    panel3_top_row,
                    cost_top_row: panel3_top_row + ctx_box_h,
                    show_context_panel: sel.show_context,
                    split_ctx_cost,
                },
            };
            handle_input_batch(
                &String::from_utf8_lossy(&in_buf[..n]),
                &env,
                &row_tag_map,
                &mut subagents,
                &mut active_view_id,
                &mut sel,
                &mut orch_watcher.cfg,
            );
        }
        tick = tick.wrapping_add(1);
    }

    bottom_panel::cleanup_sidebar_pid_files(tab_yazi_id, &c_sid, c_pid, &my_pid_str);

    let _ = stdout.write_all(
        b"\x1b_Ga=d,d=i,i=71,q=2\x1b\\\x1b[?1004l\x1b[?1000l\x1b[?1002l\x1b[?1006l\x1b[?1049l\x1b[?25h\x1b[0m\n",
    );
    let _ = stdout.flush();
}
