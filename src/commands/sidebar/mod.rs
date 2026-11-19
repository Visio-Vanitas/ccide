//! `ccide-core sidebar`: Event-Driven Interactive Right Sidebar Daemon (`src/commands/sidebar/`).

pub mod agent_refresh;
pub mod agent_tab;
pub mod bottom_panel;
pub mod dismiss;
pub mod dispatch;
pub mod hit_mask;
pub mod input;
pub mod ipc;
pub mod ipc_guard;
pub mod mouse_actions;
pub mod orch_panel;
pub mod subagents_panel;
pub mod subagents_rows;

use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::ffi::{ProcToken, DIRTY_EVENT, RUNNING};
use crate::orchestrator::OrchestratorWatcher;
use crate::platform::user_home_dir;
use crate::session::transcript::SubagentItem;
use crate::session::{
    is_subagent_alive, load_config_models_for, AgentKind, AgentSession, AgySwarmSnapshot,
    ConfigModels, SessionBinding,
};
use crate::telemetry::TelemetrySampler;
use crate::tui::context::SessionContextAnalyzer;
use crate::tui::grid::{get_terminal_geometry, render_multi_panel_grid};
use crate::tui::logo::load_adaptive_logos;
use crate::tui::palette::PaletteWatcher;

use agent_refresh::sample_ctx;
use agent_tab::{refresh_tab_dispatch, TabAgentWatcher};
use bottom_panel::{
    append_bottom_panels, begin_sidebar_session, finish_sidebar_session, poll_and_dispatch_input,
    sweep_stale_sidebar_files,
};
use dispatch::SidebarSelectState;
use ipc::{update_isolated_session_binding, write_active_view_state};
use subagents_panel::build_top_and_mid_panels;

pub fn run(target_cwd: &str, tab_yazi_id: &str, shell_pid_arg: u32) {
    let home = user_home_dir();
    let my_tok = ProcToken::self_token();
    let my_tok_str = my_tok.encode();
    let (console, mut stdout) =
        begin_sidebar_session(&home, tab_yazi_id, target_cwd, shell_pid_arg, &my_tok_str);
    let mut last_sweep = Instant::now();

    let mut logos = load_adaptive_logos(&home);
    let mut pal_watcher = PaletteWatcher::new(&home);
    let mut orch_watcher = OrchestratorWatcher::new(&home);
    let mut agent_watcher = TabAgentWatcher::new(&home, tab_yazi_id);
    let mut cur_agent_sess: Option<AgentSession> = None;
    let mut cur_swarm: Option<AgySwarmSnapshot> = None;
    let mut telemetry = TelemetrySampler::new(48);
    let (mut sub_trackers, mut main_tracker, mut seen_sub_seq) =
        (HashMap::new(), HashMap::new(), HashMap::new());
    let mut frame_buf = String::with_capacity(16384);
    let (mut prev_rows, mut row_tag_map) = (Vec::<String>::new(), HashMap::new());
    let mut sel = SidebarSelectState {
        cost_lines: logos.cost_lines,
        ..SidebarSelectState::default()
    };

    let (mut active_view_id, mut cost_max_scroll, mut tick) = ("main".to_string(), 0usize, 0usize);
    let (mut cached_ctx_snap, mut seen_main_seq, mut fast_input_redraw) = (None, None, false);
    let mut ctx_analyzer = SessionContextAnalyzer::new();
    write_active_view_state(&home, tab_yazi_id, "main");

    let mut sb = SessionBinding {
        agent: agent_watcher.kind,
        ..SessionBinding::default()
    };
    if agent_watcher.kind == AgentKind::Claude {
        let _ = update_isolated_session_binding(
            &home,
            tab_yazi_id,
            target_cwd,
            shell_pid_arg,
            my_tok,
            &mut sb,
        );
    }

    let (mut main_tool, mut main_file) = (String::new(), String::new());
    let (mut main_line, mut main_span) = (0usize, 0usize);
    let mut subagents: Vec<SubagentItem> = Vec::new();
    let mut cached_cfg: ConfigModels =
        load_config_models_for(agent_watcher.kind, &home, &sb.session_id, tab_yazi_id, None);
    let mut prev_geom_key = (0usize, 0usize);

    while RUNNING.load(Ordering::Relaxed) {
        if last_sweep.elapsed() >= Duration::from_secs(30) {
            sweep_stale_sidebar_files();
            last_sweep = Instant::now();
        }
        let geom = get_terminal_geometry();
        if (geom.rows, geom.cols) != prev_geom_key {
            prev_geom_key = (geom.rows, geom.cols);
            prev_rows.clear();
            DIRTY_EVENT.store(true, Ordering::Relaxed);
        }
        let inner_w = geom.cols.saturating_sub(4).max(10);
        let has_running_sub = subagents.iter().any(|a| is_subagent_alive(a.status));
        let is_dirty = !fast_input_redraw && DIRTY_EVENT.swap(false, Ordering::SeqCst);

        if !fast_input_redraw && (is_dirty || tick % 2 == 0) {
            if agent_watcher.reload_if_changed(&home, tab_yazi_id) {
                main_tracker.clear();
                sub_trackers.clear();
                seen_sub_seq.clear();
                ctx_analyzer.clear();
                prev_rows.clear();
                cur_agent_sess = None;
                cur_swarm = None;
                (
                    cached_ctx_snap,
                    seen_main_seq,
                    sel.cost_scroll,
                    sel.status_notice,
                ) = (None, None, 0, None);
            }
            let r = pal_watcher.reload_if_changed(&home);
            if r.synced_leader {
                crate::commands::workspace::actions::refresh_open_yazi_previews(&home);
            }
            if r.changed || logos.refresh_mode_if_changed(&home) {
                sel.cost_lines = logos.cost_lines;
                prev_rows.clear();
            }
            orch_watcher.reload_if_changed(&home);
        }
        let pal = &pal_watcher.palette;
        let agent_kind = agent_watcher.kind;
        sb.agent = agent_kind;

        if !fast_input_redraw
            && (is_dirty || has_running_sub || sb.session_id.is_empty() || tick % 2 == 0)
        {
            let rebind_due = sb.session_id.is_empty() || is_dirty || tick % 2 == 0;
            let tr = refresh_tab_dispatch(
                agent_kind,
                rebind_due,
                &home,
                tab_yazi_id,
                target_cwd,
                shell_pid_arg,
                my_tok,
                &mut sb,
                &mut cur_agent_sess,
                &mut active_view_id,
                &sel.last_click_instant,
                &mut sel.status_notice,
                &mut main_tracker,
                &mut sub_trackers,
                &mut seen_main_seq,
                &mut seen_sub_seq,
            );
            sb.agent = agent_kind;
            (main_tool, main_file, main_line, main_span) = (tr.tool, tr.file, tr.line, tr.span);
            (cached_cfg, subagents, cur_swarm) = (tr.cfg, tr.subs, tr.swarm);
            if tr.rebound {
                ctx_analyzer.clear();
                prev_rows.clear();
                active_view_id = "main".to_string();
                write_active_view_state(&home, tab_yazi_id, "main");
                (cached_ctx_snap, sel.cost_scroll, sel.status_notice) = (None, 0, None);
            }
        }

        let (top_panel, top_h, mid_panel, mid_h, bot_h, running_cnt, viewport_h) =
            build_top_and_mid_panels(
                geom,
                &logos,
                agent_kind,
                target_cwd,
                &active_view_id,
                &mut sel,
                &main_tool,
                &main_file,
                &subagents,
                &cached_cfg,
                &orch_watcher.cfg,
                cur_swarm.as_ref(),
                tick,
                pal,
            );
        if !fast_input_redraw || cached_ctx_snap.is_none() {
            telemetry.sample();
            cached_ctx_snap = Some(sample_ctx(
                &mut ctx_analyzer,
                agent_kind,
                &home,
                target_cwd,
                cur_agent_sess.as_ref(),
                cur_swarm.as_ref(),
                &active_view_id,
                &sb.session_id,
                sb.data_pid,
                &subagents,
                &cached_cfg,
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
        } else if running_cnt > 0 || sb.session_id.is_empty() {
            250
        } else {
            1000
        };
        if poll_and_dispatch_input(
            &console,
            timeout_ms,
            &home,
            tab_yazi_id,
            target_cwd,
            &sb,
            &main_file,
            main_line,
            main_span,
            geom.cols,
            top_h,
            mid_h,
            ctx_box_h,
            viewport_h,
            cost_max_scroll,
            split_ctx_cost,
            &row_tag_map,
            &mut subagents,
            &mut active_view_id,
            &mut sel,
            &mut orch_watcher.cfg,
        ) {
            fast_input_redraw = true;
        }
        tick = tick.wrapping_add(1);
    }

    finish_sidebar_session(tab_yazi_id, &sb.session_id, sb.ui_pid, my_tok);
}
