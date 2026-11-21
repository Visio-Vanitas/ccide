//! Bottom Region (`Hardware Telemetry ⇄ Split CONTEXT + COST`) & Stale-PID Sweep (`src/commands/sidebar/bottom_panel.rs`).

use std::fs;
use std::io::Write;

use crate::commands::sidebar::dispatch::handle_input_batch;
use crate::commands::sidebar::hit_mask::{FocusZone, SidebarGeomMask};
use crate::commands::sidebar::ipc::register_sidebar_tab;
use crate::commands::sidebar::mouse_actions::{DispatchEnv, SidebarSelectState};
use crate::ffi::{install_signal_handlers, ConsoleRawSession, ProcToken};
use crate::json::extract_json_str;
use crate::orchestrator::OrchestratorConfig;
use crate::platform::{cache_dir, ipc_dir, ipc_path};
use crate::session::transcript::SubagentItem;
use crate::session::SessionBinding;
use crate::telemetry::chart::build_telemetry_panel_rows;
use crate::telemetry::TelemetrySampler;
use crate::tui::context::{build_context_panel, build_cost_panel, ContextSnapshot};
use crate::tui::grid::PanelSpec;
use crate::tui::palette::Palette;
use std::cell::Cell;
use std::collections::HashMap;

#[allow(clippy::too_many_arguments)]
pub fn append_bottom_panels(
    panels: &mut Vec<PanelSpec>,
    sel: &mut SidebarSelectState,
    ctx_snap: &ContextSnapshot,
    telemetry: &TelemetrySampler,
    cols: usize,
    inner_w: usize,
    bot_h: usize,
    cost_max_scroll: &mut usize,
    pal: &Palette,
) -> (bool, usize) {
    let split_ctx_cost = sel.show_context && bot_h >= 18;
    // Fixed 11-row compact `CONTEXT` (header + spectrum + 5 components + CACHE + meta = 9 interior rows)
    let ctx_box_h = if split_ctx_cost {
        11.min(bot_h.saturating_sub(8))
    } else {
        bot_h
    };
    let ctx_foc = sel.focus_zone == Some(FocusZone::Context);
    let cost_foc = sel.focus_zone == Some(FocusZone::Cost);
    if sel.show_context {
        if split_ctx_cost {
            panels.push(build_context_panel(
                ctx_snap, cols, ctx_box_h, false, ctx_foc, pal,
            ));
            let (cost_p, max_sc) = build_cost_panel(
                ctx_snap,
                cols,
                bot_h - ctx_box_h,
                sel.cost_scroll,
                sel.cost_lines,
                cost_foc,
                pal,
            );
            *cost_max_scroll = max_sc;
            sel.cost_scroll = sel.cost_scroll.min(*cost_max_scroll);
            panels.push(cost_p);
        } else {
            *cost_max_scroll = 0;
            panels.push(build_context_panel(
                ctx_snap,
                cols,
                bot_h,
                true,
                ctx_foc || cost_foc,
                pal,
            ));
        }
    } else {
        panels.push(PanelSpec {
            title: telemetry.cpu_brand.to_uppercase(),
            badge: format!(
                "{}C·{:.0}G [CTX {}%]",
                telemetry.cpu_cores, telemetry.total_mem_gb, ctx_snap.budget.used_pct
            ),
            accent: pal.mint.clone(),
            rows: build_telemetry_panel_rows(telemetry, inner_w, bot_h, pal),
            fixed_height: Some(bot_h),
            weight: 1.12,
            raw_interior: false,
            focused: sel.focus_zone == Some(FocusZone::Telemetry),
        });
    }
    (split_ctx_cost, ctx_box_h)
}

fn tab_json_is_alive(path: &std::path::Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|txt| extract_json_str(&txt, "pid"))
        .and_then(|s| ProcToken::parse(&s))
        .map(|t| t.is_alive_match())
        .unwrap_or(false)
}

/// Removes stale or bare-PID `sb_*.pid` and `sidebar_tab_*.json` files in `ipc_dir()`.
pub fn sweep_stale_sidebar_files() {
    let dir = ipc_dir();
    let Ok(rd) = fs::read_dir(&dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let fname = entry.file_name();
        let name = fname.to_string_lossy();
        if name.starts_with("sb_") && name.ends_with(".pid") {
            let alive = fs::read_to_string(&p)
                .ok()
                .and_then(|s| ProcToken::parse(&s))
                .map(|t| t.is_alive_match())
                .unwrap_or(false);
            if !alive {
                let _ = fs::remove_file(&p);
                if let Some(yid) = name
                    .strip_prefix("sb_yazi_")
                    .and_then(|s| s.strip_suffix(".pid"))
                {
                    let tab_json = dir.join(format!("sidebar_tab_{yid}.json"));
                    if !tab_json_is_alive(&tab_json) {
                        let _ = fs::remove_file(&tab_json);
                    }
                }
            }
        } else if name.starts_with("sidebar_tab_")
            && name.ends_with(".json")
            && !tab_json_is_alive(&p)
        {
            let _ = fs::remove_file(&p);
        }
    }
}

pub fn cleanup_sidebar_pid_files(tab_yazi_id: &str, c_sid: &str, c_pid: u32, my_tok: ProcToken) {
    let rm_if_mine = |p: std::path::PathBuf| -> bool {
        let mine = fs::read_to_string(&p)
            .ok()
            .and_then(|s| ProcToken::parse(&s))
            .map(|t| t == my_tok)
            .unwrap_or(false);
        if mine {
            let _ = fs::remove_file(&p);
        }
        mine
    };
    if !tab_yazi_id.is_empty() && rm_if_mine(ipc_path(&format!("sb_yazi_{}.pid", tab_yazi_id))) {
        let _ = fs::remove_file(ipc_path(&format!("sidebar_tab_{}.json", tab_yazi_id)));
    }
    if !c_sid.is_empty() {
        rm_if_mine(ipc_path(&format!("sb_{}.pid", c_sid)));
    }
    if c_pid > 1 {
        rm_if_mine(ipc_path(&format!("sb_pid_{}.pid", c_pid)));
    }
}

pub fn begin_sidebar_session(
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    shell_pid_arg: u32,
    my_tok_str: &str,
) -> (ConsoleRawSession, std::io::Stdout) {
    let _ = fs::create_dir_all(cache_dir(home));
    let _ = fs::create_dir_all(ipc_dir());
    sweep_stale_sidebar_files();
    register_sidebar_tab(home, tab_yazi_id, target_cwd, shell_pid_arg, my_tok_str);
    install_signal_handlers(true);
    let console = ConsoleRawSession::enter();
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(
        b"\x1b]1;[AI] Companion & Agents\x07\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1004h\x1b[?1006h\x1b[49m\x1b[2J\x1b[3J",
    );
    let _ = stdout.flush();
    (console, stdout)
}

#[allow(clippy::too_many_arguments)]
pub fn poll_and_dispatch_input(
    console: &ConsoleRawSession,
    timeout_ms: i32,
    home: &str,
    tab_yazi_id: &str,
    target_cwd: &str,
    sb: &SessionBinding,
    main_file: &str,
    main_line: usize,
    main_span: usize,
    cols: usize,
    top_h: usize,
    mid_h: usize,
    ctx_box_h: usize,
    viewport_h: usize,
    cost_max_scroll: usize,
    split_ctx_cost: bool,
    row_tag_map: &HashMap<usize, (u8, usize)>,
    subagents: &mut Vec<SubagentItem>,
    active_view_id: &mut String,
    sel: &mut SidebarSelectState,
    orch_cfg: &mut OrchestratorConfig,
) -> bool {
    let mut in_buf = [0u8; 1024];
    let Some(n) = console.poll_stdin(&mut in_buf, timeout_ms) else {
        return false;
    };
    let panel3_top_row = top_h + mid_h + 1;
    let env = DispatchEnv {
        home,
        yazi_id: tab_yazi_id,
        cwd: target_cwd,
        sid: &sb.session_id,
        cpid: sb.ui_pid,
        agent: sb.agent,
        main_file,
        main_line,
        main_span,
        viewport_h,
        cost_max_scroll,
        hit_mask: SidebarGeomMask {
            cols,
            top_h,
            panel3_top_row,
            cost_top_row: panel3_top_row + ctx_box_h,
            show_context_panel: sel.show_context,
            split_ctx_cost,
        },
        switch_signal_ok: Cell::new(None),
    };
    handle_input_batch(
        &String::from_utf8_lossy(&in_buf[..n]),
        &env,
        row_tag_map,
        subagents,
        active_view_id,
        sel,
        orch_cfg,
    );
    env.sync_notice(&mut sel.status_notice);
    true
}

pub fn finish_sidebar_session(tab_yazi_id: &str, c_sid: &str, c_pid: u32, my_tok: ProcToken) {
    cleanup_sidebar_pid_files(tab_yazi_id, c_sid, c_pid, my_tok);
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(
        b"\x1b_Ga=d,d=i,i=71,q=2\x1b\\\x1b[?1004l\x1b[?1000l\x1b[?1002l\x1b[?1006l\x1b[?1049l\x1b[?25h\x1b[0m\n",
    );
    let _ = stdout.flush();
}
